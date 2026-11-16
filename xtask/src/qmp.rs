//! Vkladani vstupu do QEMU pres QMP (QEMU Machine Protocol) pro test vstupu.
//!
//! Testy v hostovanem systemu (vstup, PS/2, USB, GUI) vypisuji do seriove
//! linky znacky. Vlakno xtasku sleduje log seriove linky a skript vstupu je
//! rozdeleny na useky podle zacatku testu ([`STARTS`]): kazdy usek se spusti,
//! az se objevi jeho znacka, a usek, jehoz test nebezi (`--only`, jiny
//! stroj), se preskoci - testy tak nezavisi na poradi a vyberu sad.
//! Posila `input-send-event` (klavesy, mys, kolecko, Ctrl+Alt+Delete),
//! pripojeni USB a `screendump`. Udalosti projdou cele: QEMU -> virtio-input
//! -> ovladac -> inputd -> test.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

/// Musi sedet s `QEMU_MARKER` v `user/init/src/tests/input.rs`.
pub const MARKER: &str = "init: test vstupu: cekam na udalosti z QEMU";

/// Jak dlouho nejdyl cekat na jednu znacku uvnitr useku.
const WAIT_MAX: Duration = Duration::from_secs(600);

/// Volny port TCP na localhostu pro QMP.
pub fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Argumenty QEMU pro QMP server na `port`.
pub fn qemu_arg(port: u16) -> String {
    format!("tcp:127.0.0.1:{port},server=on,wait=off")
}

/// Treti znacka: test vstupu PS/2 (udalosti bez zarizeni jdou do PS/2).
/// Musi sedet s `QEMU_MARKER_PS2` v `user/init/src/tests/input.rs`.
pub const MARKER_PS2: &str = "init: test vstupu: cekam na udalosti PS/2 z QEMU";

/// Druha znacka: test dostal klavesy i mys a ceka na bezpecnou zkratku.
/// Musi sedet s `QEMU_MARKER_SECURE` v `user/init/src/tests/input.rs`.
pub const MARKER_SECURE: &str = "init: test vstupu: cekam na zkratku z QEMU";

/// Sledovani logu seriove linky od posledni nalezene znacky. Znacky se
/// hledaji jen za `pos`: s `--repeat` je kazde kolo v logu znovu a znacka
/// z minuleho kola uz neplati (jinak by se useky dalsich kol spustily
/// hned, nebo vubec).
struct Log {
    path: PathBuf,
    pos: usize,
}

/// Vysledek hledani v logu ([`scan`]).
#[derive(Debug, PartialEq)]
enum Scan {
    /// Znacka `wanted[i]` zacina na bajtu `at` a jeji radek je cely; `rest`
    /// = zbytek radku za znackou.
    Found { i: usize, at: usize, rest: String },
    /// Driv nez kterakoli hledana znacka se objevila znacka z `abort`.
    Aborted,
    /// Zatim nic (nebo radek se znackou jeste neni cely).
    Pending,
}

/// Nejdrive se objevujici znacka z `wanted` v `text[pos..]` (cely radek),
/// nebo [`Scan::Aborted`], kdyz pred ni lezi znacka z `abort`.
fn scan(text: &[u8], pos: usize, wanted: &[&str], abort: &[&str]) -> Scan {
    let hay = text.get(pos..).unwrap_or(&[]);
    let first = |ms: &[&str]| ms.iter().enumerate().filter_map(|(i, m)| find(hay, m.as_bytes()).map(|k| (k, i))).min();
    let hit = first(wanted);
    if let Some((k, _)) = first(abort)
        && hit.is_none_or(|(h, _)| k < h)
    {
        return Scan::Aborted;
    }
    let Some((k, i)) = hit else { return Scan::Pending };
    let after = &hay[k + wanted[i].len()..];
    let Some(nl) = after.iter().position(|&c| c == b'\n') else { return Scan::Pending };
    let rest = String::from_utf8_lossy(&after[..nl]).trim_end_matches('\r').to_string();
    Scan::Found { i, at: pos + k, rest }
}

/// Pozice `needle` v `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

impl Log {
    fn read(&mut self) -> Vec<u8> {
        let text = std::fs::read(&self.path).unwrap_or_default();
        // Opakovany start QEMU log smaze: hledat znovu od zacatku.
        if text.len() < self.pos {
            self.pos = 0;
        }
        text
    }

    /// Pocka na `marker` za posledni znackou; vrati zbytek jeho radku
    /// a posune se za nej. `None` = driv prisla znacka z `abort` (test
    /// skoncil nebo prestoupil k dalsimu useku) nebo vyprsel limit.
    fn wait(&mut self, marker: &str, abort: &[&str]) -> Option<String> {
        let start = Instant::now();
        while start.elapsed() < WAIT_MAX {
            let text = self.read();
            match scan(&text, self.pos, &[marker], abort) {
                Scan::Found { at, rest, .. } => {
                    self.pos = at + marker.len();
                    return Some(rest);
                }
                Scan::Aborted => return None,
                Scan::Pending => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        None
    }
}

/// Spusti vlakno, ktere po objeveni znacky v `log` posle udalosti. `ps2`
/// = stroj ma radic PS/2; bez nej test PS/2 svou znacku nevypise a kroky
/// PS/2 se preskoci (jinak by vlakno na znacku cekalo a k USB se nedostalo).
/// `port` cte az pri vkladani: opakovany start QEMU (souboj o port) ho
/// zmeni. S `--repeat` se useky spousti v kazdem kole znovu (znacky se
/// hledaji za posledni nalezenou, [`Log`]).
pub fn spawn_injector(log: PathBuf, port: Arc<AtomicU16>, ps2: bool) {
    std::thread::spawn(move || {
        let mut log = Log { path: log, pos: 0 };
        let segs = segments(ps2);
        // Pripojit az na zacatku prvniho useku (QEMU uz bezi).
        let Some(first) = next_segment(&mut log, &segs) else { return };
        match inject(port.load(Ordering::Relaxed), &mut log, &segs, first) {
            Ok(n) => println!("xtask: QMP: udalosti vstupu poslany ({n} useku)"),
            Err(e) => eprintln!("xtask: QMP: vkladani vstupu selhalo: {e}"),
        }
    });
}

/// Zacatky testu, ktere vstup z QEMU potrebuji: podle nich se skript deli
/// na useky (kazdy test zvlast, aby sel spustit i sam).
const STARTS: &[&str] = &[
    MARKER,
    MARKER_PS2,
    usb::KEYS,
    gui::DARK_START,
    gui::PANEL_SHOT[0],
    media::PLUG,
    gui::DIALOG,
    gui::SECURE,
    gui::TERM_DRAG,
    gui::CLICK,
    gui::SNAPSHOT,
    gui::PALETTE_SWITCH,
    gui::SHOT[0],
    gui::FILES_CLICK,
    sdl::CLICK,
    wine::KEYS_CLICK,
    wine::CLICK,
    wine::PASTE_CLICK,
    wine::OFFER_CLICK,
];

/// Konec testovaciho behu userspace (dal uz zadny usek neprijde).
const END: [&str; 2] = ["init: vsechny testy userspace prosly", "testu userspace selhalo"];

/// Skript rozdeleny na useky: (znacka zacatku, kroky vcetne cekani na ni).
fn segments(ps2: bool) -> Vec<(&'static str, Vec<Step>)> {
    let mut out: Vec<(&'static str, Vec<Step>)> = Vec::new();
    for step in script(ps2) {
        let start = match &step {
            Step::WaitFor(m) | Step::ClickAt(m) | Step::DragAt(m) if STARTS.contains(m) => Some(*m),
            _ => None,
        };
        match (start, out.last_mut()) {
            (Some(m), _) => out.push((m, vec![step])),
            (None, Some(last)) => last.1.push(step),
            // Kroky pred prvni znackou nejsou (skript zacina cekanim).
            (None, None) => {}
        }
    }
    out
}

/// Pocka na zacatek nektereho useku za posledni znackou v logu; vrati
/// index useku, jehoz znacka se objevila nejdriv (useky testu, ktere
/// nebezi, se tak preskoci), a nastavi pozici na ni. `None` = beh testu
/// skoncil. Bez limitu: mezi useky muzou bezet dlouhe sady (beh hlida
/// limit testu v `qemu.rs`, vlakno skonci s xtaskem).
fn next_segment(log: &mut Log, segs: &[(&'static str, Vec<Step>)]) -> Option<usize> {
    let starts: Vec<&str> = segs.iter().map(|s| s.0).collect();
    loop {
        let text = log.read();
        match scan(&text, log.pos, &starts, &END) {
            Scan::Found { i, at, .. } => {
                log.pos = at;
                return Some(i);
            }
            Scan::Aborted => return None,
            Scan::Pending => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

/// Znacky, ktere ukonci cekani uvnitr useku: zacatek jineho useku nebo
/// konec behu (test selhal a znacku nevypise).
fn aborts(marker: &str) -> Vec<&'static str> {
    STARTS.iter().chain(END.iter()).copied().filter(|m| *m != marker).collect()
}

/// Klavesa (qcode QEMU).
fn key(qcode: &str, down: bool) -> String {
    format!(
        r#"{{"type":"key","data":{{"down":{down},"key":{{"type":"qcode","data":"{qcode}"}}}}}}"#
    )
}

fn btn(button: &str, down: bool) -> String {
    format!(r#"{{"type":"btn","data":{{"down":{down},"button":"{button}"}}}}"#)
}

fn abs(axis: &str, value: i32) -> String {
    format!(r#"{{"type":"abs","data":{{"axis":"{axis}","value":{value}}}}}"#)
}

fn rel(axis: &str, value: i32) -> String {
    format!(r#"{{"type":"rel","data":{{"axis":"{axis}","value":{value}}}}}"#)
}

/// Krok skriptu: udalosti jednoho prikazu `input-send-event` (= jeden
/// snimek), nebo pauza.
#[derive(Clone)]
enum Step {
    /// Udalosti pro virtio (konzole `video0`).
    Send(Vec<String>),
    /// Udalosti bez zarizeni: dostane je PS/2 (nebo naposled aktivni
    /// zarizeni USB bez vazby na konzoli, pripojene za behu).
    SendPs2(Vec<String>),
    /// Udalosti pro klavesnici USB (konzole `video1`).
    SendUsb(Vec<String>),
    /// Primo prikaz QMP (pripojeni a odpojeni zarizeni).
    Qmp(String),
    /// Pockat na znacku v logu seriove linky.
    WaitFor(&'static str),
    /// Klik levym tlacitkem na polohu z logu (za znackou "x y"): ukazatel
    /// do rohu, pak pohyb a stisk v jednom snimku, pusteni zvlast.
    ClickAt(&'static str),
    /// Tah levym tlacitkem z "x0 y0" na "x1 y1" (za znackou): ukazatel do
    /// rohu, pohyb a stisk, pohyb, pusteni.
    DragAt(&'static str),
}

/// Posloupnost kroku. Test v `input.rs` ocekava presne tyto udalosti.
/// Pred bezpecnou zkratkou se ceka na druhou znacku: klavesnice a mys jsou
/// ruzna zarizeni a jejich udalosti si navzajem nedrzi poradi, mys musi
/// dorazit driv, nez zkratka prepne vstup na kanal `secure`. Test znacku
/// vypise, az udalosti mysi dostane (zadne pevne cekani).
fn script(ps2: bool) -> Vec<Step> {
    let mut s: Vec<Step> = Vec::new();
    s.push(Step::WaitFor(MARKER));
    for (k, d) in [
        ("a", true),
        ("a", false),
        ("shift", true),
        ("1", true),
        ("1", false),
        ("shift", false),
    ] {
        s.push(Step::Send(vec![key(k, d)]));
    }
    s.push(Step::Send(vec![rel("x", 10), rel("y", -5)]));
    s.push(Step::Send(vec![btn("left", true)]));
    s.push(Step::Send(vec![btn("left", false)]));
    s.push(Step::Send(vec![btn("wheel-down", true)]));
    s.push(Step::Send(vec![btn("wheel-down", false)]));
    s.push(Step::WaitFor(MARKER_SECURE));
    for (k, d) in [
        ("ctrl", true),
        ("alt", true),
        ("delete", true),
        ("delete", false),
        ("alt", false),
        ("ctrl", false),
    ] {
        s.push(Step::Send(vec![key(k, d)]));
    }
    // PS/2: b, Shift+2 (US: @), Delete (rozsirena klavesa E0), pohyb,
    // prave tlacitko, kolecko nahoru.
    if ps2 {
        ps2_steps(&mut s);
    }
    usb_steps(&mut s);
    media_steps(&mut s);
    gui_steps(&mut s);
    sdl_steps(&mut s);
    wine_steps(&mut s);
    s
}

/// Port SDL (`user/init/src/tests/sdl.rs`, program `testmagnumopus`):
/// zamek ukazatele, schranka a cela obrazovka plati jen po fyzickem vstupu.
fn sdl_steps(s: &mut Vec<Step>) {
    // Klik: zamek ukazatele; posun: relativni pohyb; Super: zruseni.
    s.push(Step::ClickAt(sdl::CLICK));
    s.push(Step::WaitFor(sdl::MOVE));
    s.push(Step::Send(vec![rel("x", 30), rel("y", 10)]));
    s.push(Step::WaitFor(sdl::SUPER));
    s.push(Step::Send(vec![key("meta_l", true)]));
    s.push(Step::Send(vec![key("meta_l", false)]));
    // Druhy klik: kopie do schranky; Ctrl+V: vlozeni.
    s.push(Step::ClickAt(sdl::CLICK2));
    s.push(Step::WaitFor(sdl::PASTE));
    for (name, d) in [("ctrl", true), ("v", true), ("v", false), ("ctrl", false)] {
        s.push(Step::Send(vec![key(name, d)]));
    }
    // F: cela obrazovka; Esc: konec programu.
    // Dialog povoleni nahravani: Enter = Povolit (prvni volba).
    for (marker, k) in [(sdl::FULL, "f"), (sdl::ESC, "esc"), (sdl::PERM, "ret")] {
        s.push(Step::WaitFor(marker));
        s.push(Step::Send(vec![key(k, true)]));
        s.push(Step::Send(vec![key(k, false)]));
    }
}

/// Windows vrstva W3 (`user/init/src/tests/wine_gui.rs`): poznamkovy blok
/// pres winemo.drv dostane fyzicky klik a klavesy (text, Ctrl+S, nazev
/// souboru v dialogu Ulozit jako, Alt+F4).
fn wine_steps(s: &mut Vec<Step>) {
    let press = |s: &mut Vec<Step>, keys: &[&str]| {
        for k in keys {
            s.push(Step::Send(vec![key(k, true)]));
        }
        for k in keys.iter().rev() {
            s.push(Step::Send(vec![key(k, false)]));
        }
    };
    // Okno klaves (wintest klavesy): klik, pak a a z bez modifikatoru.
    s.push(Step::ClickAt(wine::KEYS_CLICK));
    for k in ["a", "z"] {
        press(s, &[k]);
    }
    s.push(Step::ClickAt(wine::CLICK));
    s.push(Step::WaitFor(wine::TYPE));
    for k in ["a", "h", "o", "j"] {
        press(s, &[k]);
    }
    // Ctrl+S az po kontrole, ze je text videt.
    s.push(Step::WaitFor(wine::SAVE_KEYS));
    press(s, &["ctrl", "s"]);
    // t.txt a Enter v dialogu (bez dvojtecky a lomitka: rozlozeni
    // klavesnice systemu nemusi byt americke).
    s.push(Step::WaitFor(wine::SAVE));
    for k in ["t", "dot", "t", "x", "t", "ret"] {
        press(s, &[k]);
    }
    // Schranka: cely text do schranky (Wine -> kompozitor), pak konec.
    s.push(Step::WaitFor(wine::COPY));
    press(s, &["ctrl", "a"]);
    press(s, &["ctrl", "c"]);
    s.push(Step::WaitFor(wine::CLOSE));
    press(s, &["alt", "f4"]);
    // Vlozeni do naseho okna (gui-probe vloz): klik a Ctrl+V.
    s.push(Step::ClickAt(wine::PASTE_CLICK));
    press(s, &["ctrl", "v"]);
    // W4: nase aplikace nabidne text (klik a stisk klavesy = gesto),
    // poznamkovy blok ho vlozi a ulozi jako u.txt.
    s.push(Step::ClickAt(wine::OFFER_CLICK));
    press(s, &["ctrl", "c"]);
    s.push(Step::ClickAt(wine::PASTE_WINE_CLICK));
    press(s, &["ctrl", "v"]);
    s.push(Step::WaitFor(wine::SAVE2_KEYS));
    press(s, &["ctrl", "s"]);
    s.push(Step::WaitFor(wine::SAVE2_NAME));
    for k in ["u", "dot", "t", "x", "t", "ret"] {
        press(s, &[k]);
    }
}

/// Znacky testu W3 (musi sedet s `user/init/src/tests/wine_gui.rs`).
mod wine {
    pub const KEYS_CLICK: &str = "init: test wine: klik do okna klaves na ";
    pub const CLICK: &str = "init: test wine: klik do poznamkoveho bloku na ";
    pub const TYPE: &str = "init: test wine: pis do poznamkoveho bloku";
    pub const SAVE_KEYS: &str = "init: test wine: Ctrl+S";
    pub const SAVE: &str = "init: test wine: dialog ulozeni";
    pub const COPY: &str = "init: test wine: kopiruj text";
    pub const CLOSE: &str = "init: test wine: zavri poznamkovy blok";
    pub const PASTE_CLICK: &str = "init: test wine: klik do vkladaciho okna na ";
    pub const OFFER_CLICK: &str = "init: test wine: klik do nabidky textu na ";
    pub const PASTE_WINE_CLICK: &str = "init: test wine: klik pro vlozeni ve Wine na ";
    pub const SAVE2_KEYS: &str = "init: test wine: uloz vlozeny text";
    pub const SAVE2_NAME: &str = "init: test wine: nazev vlozeneho souboru";
}

/// Znacky testu SDL (musi sedet s `user/init/src/tests/sdl.rs`).
mod sdl {
    pub const CLICK: &str = "init: test sdl: klik mysi na ";
    pub const MOVE: &str = "init: test sdl: ceka na posun mysi";
    pub const SUPER: &str = "init: test sdl: ceka na Super";
    pub const CLICK2: &str = "init: test sdl: druhy klik na ";
    pub const PASTE: &str = "init: test sdl: ceka na Ctrl+V";
    pub const FULL: &str = "init: test sdl: ceka na F";
    pub const ESC: &str = "init: test sdl: ceka na Esc";
    pub const PERM: &str = "init: test sdl: dialog povoleni ceka na Enter";
}

/// GUI (`user/init/src/tests/gui.rs`): fyzicka klavesnice odpovi na
/// systemovy dialog (umely vstup ho nepotvrdi) a na bezpecnou obrazovku.
fn gui_steps(s: &mut Vec<Step>) {
    // Snimek aplikaci spustenych v tmavem rezimu (test `start_dark`).
    s.push(Step::WaitFor(gui::DARK_START));
    let shots = crate::target_dir().join("shots");
    let _ = std::fs::create_dir_all(&shots);
    let file = shots.join("tmavy-start.png").display().to_string().replace('\\', "/");
    s.push(Step::Qmp(format!(r#"{{"execute":"screendump","arguments":{{"filename":"{file}","device":"gpu0","format":"png"}}}}"#)));
    // Panely listy (test `panel_shots`).
    for (marker, name) in gui::PANEL_SHOT.iter().zip(gui::PANEL_NAMES) {
        s.push(Step::WaitFor(marker));
        let file = shots.join(format!("{name}.png")).display().to_string().replace('\\', "/");
        s.push(Step::Qmp(format!(r#"{{"execute":"screendump","arguments":{{"filename":"{file}","device":"gpu0","format":"png"}}}}"#)));
    }
    s.push(Step::WaitFor(gui::DIALOG));
    for (k, d) in [("tab", true), ("tab", false), ("ret", true), ("ret", false)] {
        s.push(Step::Send(vec![key(k, d)]));
    }
    s.push(Step::WaitFor(gui::SECURE));
    for (k, d) in [("ctrl", true), ("alt", true), ("delete", true), ("delete", false), ("alt", false), ("ctrl", false)] {
        s.push(Step::Send(vec![key(k, d)]));
    }
    s.push(Step::WaitFor(gui::SECURE_ENTER));
    s.push(Step::Send(vec![key("ret", true)]));
    s.push(Step::Send(vec![key("ret", false)]));
    // Terminal (test `select_copy_paste`): tah mysi vybere radek, pak
    // Ctrl+C a Ctrl+V (mys a klavesnice si nedrzi poradi: klavesy az po
    // druhe znacce, kdy test vidi vyber).
    s.push(Step::DragAt(gui::TERM_DRAG));
    s.push(Step::WaitFor(gui::TERM_KEYS));
    for k in ["c", "v"] {
        for (q, d) in [("ctrl", true), (k, true), (k, false), ("ctrl", false)] {
            s.push(Step::Send(vec![key(q, d)]));
        }
    }
    // Nabidka doplnovani: Ctrl+U smaze radek, "e" + Tab ji otevre; sipka
    // dolu az po druhe znacce (odpoved s navrhy prijde od relace pozdeji).
    s.push(Step::WaitFor(gui::TERM_TAB));
    for (q, d) in [("ctrl", true), ("u", true), ("u", false), ("ctrl", false)] {
        s.push(Step::Send(vec![key(q, d)]));
    }
    for k in ["e", "tab"] {
        s.push(Step::Send(vec![key(k, true)]));
        s.push(Step::Send(vec![key(k, false)]));
    }
    s.push(Step::WaitFor(gui::TERM_DOWN));
    s.push(Step::Send(vec![key("down", true)]));
    s.push(Step::Send(vec![key("down", false)]));
    // Tah za volne misto vlastniho titulku: vpravo od karet, pak vlevo
    // mezi odznakem a kartou zpet.
    s.push(Step::DragAt(gui::TERM_TITLE_RIGHT));
    s.push(Step::DragAt(gui::TERM_TITLE_LEFT));
    // Spravce uloh: fyzicky klik na zaskrtavaci pole (test `task_manager`).
    s.push(Step::ClickAt(gui::CLICK));
    // Diagnosticky snimek (test `diag_snapshot`): Print Screen.
    s.push(Step::WaitFor(gui::SNAPSHOT));
    s.push(Step::Send(vec![key("print", true)]));
    s.push(Step::Send(vec![key("print", false)]));
    // Paleta prikazu (test `palette_keys`): Ctrl+K, hledani, Enter.
    for (marker, text) in [(gui::PALETTE_SWITCH, "uka"), (gui::PALETTE_ACTION, "pricist")] {
        s.push(Step::WaitFor(marker));
        for (k, d) in [("ctrl", true), ("k", true), ("k", false), ("ctrl", false)] {
            s.push(Step::Send(vec![key(k, d)]));
        }
        for ch in text.chars() {
            let k = ch.to_string();
            s.push(Step::Send(vec![key(&k, true)]));
            s.push(Step::Send(vec![key(&k, false)]));
        }
        s.push(Step::Send(vec![key("ret", true)]));
        s.push(Step::Send(vec![key("ret", false)]));
    }
    // Snimky vzhledu (test `look_shots`): screendump virtio-gpu do PNG.
    let dir = crate::target_dir().join("shots");
    let _ = std::fs::create_dir_all(&dir);
    for (marker, name) in gui::SHOT.iter().zip(gui::SHOT_NAMES) {
        s.push(Step::WaitFor(marker));
        let file = dir.join(format!("{name}.png")).display().to_string().replace('\\', "/");
        s.push(Step::Qmp(format!(r#"{{"execute":"screendump","arguments":{{"filename":"{file}","device":"gpu0","format":"png"}}}}"#)));
    }
    // Spravce souboru (test `files_window`): klik na radek (fokus okna),
    // Ctrl+C, Ctrl+V, Ctrl+V, snimek.
    s.push(Step::ClickAt(gui::FILES_CLICK));
    for (marker, k) in [(gui::FILES_COPY, "c"), (gui::FILES_PASTE, "v"), (gui::FILES_PASTE_AGAIN, "v")] {
        s.push(Step::WaitFor(marker));
        for (name, d) in [("ctrl", true), (k, true), (k, false), ("ctrl", false)] {
            s.push(Step::Send(vec![key(name, d)]));
        }
    }
    s.push(Step::WaitFor(gui::FILES_SHOT));
    let file = dir.join("soubory.png").display().to_string().replace('\\', "/");
    s.push(Step::Qmp(format!(r#"{{"execute":"screendump","arguments":{{"filename":"{file}","device":"gpu0","format":"png"}}}}"#)));
}

/// Znacky testu GUI (musi sedet s `user/init/src/tests/gui.rs`).
mod gui {
    /// `user/init/src/tests/gui_look.rs` (`QEMU_SHOT_DARK_START`).
    pub const DARK_START: &str = "init: test gui: snimek tmavy-start";
    /// `gui_look.rs` (`QEMU_SHOT_PANEL` + jmeno souboru, `PANEL_SHOTS`).
    pub const PANEL_SHOT: [&str; 2] = ["init: test gui: snimek panelu panel-vzhled", "init: test gui: snimek panelu panel-oznameni"];
    pub const PANEL_NAMES: [&str; 2] = ["panel-vzhled", "panel-oznameni"];
    pub const DIALOG: &str = "init: test gui: dialog ceka na klavesy z QEMU";
    pub const SECURE: &str = "init: test gui: cekam na zkratku z QEMU";
    pub const SECURE_ENTER: &str = "init: test gui: bezpecna obrazovka ceka na Enter";
    pub const CLICK: &str = "init: test gui: klik mysi na ";
    /// `user/init/src/tests/gui_term.rs`.
    pub const TERM_DRAG: &str = "init: test gui: terminal tah mysi ";
    pub const TERM_KEYS: &str = "init: test gui: terminal ceka na Ctrl+C a Ctrl+V";
    pub const TERM_TAB: &str = "init: test gui: terminal ceka na doplneni Tab";
    pub const TERM_DOWN: &str = "init: test gui: terminal ceka na sipku v seznamu navrhu";
    pub const TERM_TITLE_RIGHT: &str = "init: test gui: terminal tah titulku vpravo ";
    pub const TERM_TITLE_LEFT: &str = "init: test gui: terminal tah titulku vlevo ";
    pub const SNAPSHOT: &str = "init: test gui: diagnosticky snimek ceka na Print Screen";
    pub const PALETTE_SWITCH: &str = "init: test gui: paleta prepne na Ukazku";
    pub const PALETTE_ACTION: &str = "init: test gui: paleta pricte v Ukazce";
    pub const SHOT: [&str; 4] =
        ["init: test gui: snimek light", "init: test gui: snimek dark", "init: test gui: snimek contrast", "init: test gui: snimek terminal"];
    pub const SHOT_NAMES: [&str; 4] = ["light", "dark", "contrast", "terminal"];
    /// Spravce souboru (`user/init/src/tests/gui_files.rs`): fyzicke
    /// klavesy (schranka plati jen po fyzickem vstupu) a snimek.
    pub const FILES_CLICK: &str = "init: test gui: soubory klik mysi na ";
    pub const FILES_COPY: &str = "init: test gui: soubory ceka na Ctrl+C";
    pub const FILES_PASTE: &str = "init: test gui: soubory ceka na Ctrl+V";
    pub const FILES_PASTE_AGAIN: &str = "init: test gui: soubory ceka na dalsi Ctrl+V";
    pub const FILES_SHOT: &str = "init: test gui: snimek soubory";
}

fn ps2_steps(s: &mut Vec<Step>) {
    s.push(Step::WaitFor(MARKER_PS2));
    for (k, d) in [
        ("b", true),
        ("b", false),
        ("shift", true),
        ("2", true),
        ("2", false),
        ("shift", false),
        ("delete", true),
        ("delete", false),
    ] {
        s.push(Step::SendPs2(vec![key(k, d)]));
    }
    s.push(Step::SendPs2(vec![rel("x", 7), rel("y", 3)]));
    s.push(Step::SendPs2(vec![btn("right", true)]));
    s.push(Step::SendPs2(vec![btn("right", false)]));
    s.push(Step::SendPs2(vec![btn("wheel-up", true)]));
    s.push(Step::SendPs2(vec![btn("wheel-up", false)]));
}

/// Vymenna media (`user/init/src/tests/removable.rs`): flash disk za behu
/// (disk `mediahot` definuje `qemu.rs` bez zarizeni) a jeho vytazeni.
fn media_steps(s: &mut Vec<Step>) {
    s.push(Step::WaitFor(media::PLUG));
    s.push(Step::Qmp(format!(
        r#"{{"execute":"device_add","arguments":{{"driver":"usb-storage","id":"usbmediahot","bus":"xhci.0","port":"{}","drive":"mediahot","removable":true}}}}"#,
        crate::usbports::MEDIA_HOT
    )));
    s.push(Step::WaitFor(media::UNPLUG));
    s.push(Step::Qmp(device_del("usbmediahot")));
}

fn usb_steps(s: &mut Vec<Step>) {
    // USB (`user/init/src/tests/usb.rs`): klavesnice ze startu, hotplug mysi
    // a overeni nove klavesnice.
    s.push(Step::WaitFor(usb::KEYS));
    for (k, d) in [
        ("u", true),
        ("u", false),
        ("shift", true),
        ("3", true),
        ("3", false),
        ("shift", false),
    ] {
        s.push(Step::SendUsb(vec![key(k, d)]));
    }
    s.push(Step::WaitFor(usb::MOUSE_PLUG));
    s.push(Step::Qmp(device_add("usb-mouse", "usbmouse1")));
    s.push(Step::WaitFor(usb::MOUSE_READY));
    s.push(Step::SendPs2(vec![rel("x", 4), rel("y", 6)]));
    for (b, d) in [
        ("middle", true),
        ("middle", false),
        ("wheel-down", true),
        ("wheel-down", false),
    ] {
        s.push(Step::SendPs2(vec![btn(b, d)]));
    }
    s.push(Step::WaitFor(usb::MOUSE_UNPLUG));
    s.push(Step::Qmp(device_del("usbmouse1")));
    s.push(Step::WaitFor(usb::KBD_PLUG));
    s.push(Step::Qmp(device_add("usb-kbd", "usbkbd2")));
    s.push(Step::WaitFor(usb::KBD_READY));
    s.push(Step::SendPs2(vec![key("x", true)]));
    s.push(Step::SendPs2(vec![key("x", false)]));
    s.push(Step::WaitFor(usb::KBD_TRUSTED));
    s.push(Step::SendPs2(vec![key("y", true)]));
    s.push(Step::SendPs2(vec![key("y", false)]));
    s.push(Step::WaitFor(usb::KBD_UNPLUG));
    s.push(Step::Qmp(device_del("usbkbd2")));
    s.push(Step::WaitFor(usb::MOUSE2_PLUG));
    s.push(Step::Qmp(device_add("usb-mouse", "usbmouse2")));
    s.push(Step::WaitFor(usb::MOUSE2_PENDING));
    s.push(Step::SendPs2(vec![rel("x", 2), rel("y", 3)]));
    s.push(Step::WaitFor(usb::MOUSE2_TRUSTED));
    s.push(Step::SendPs2(vec![rel("x", 5), rel("y", 1)]));
    s.push(Step::WaitFor(usb::MOUSE2_UNPLUG));
    s.push(Step::Qmp(device_del("usbmouse2")));
    // Externi hub na port 4 radice a klavesnice na jeho port 1.
    s.push(Step::WaitFor(usb::HUB_PLUG));
    let hub = crate::usbports::USB_TEST_HUB;
    s.push(Step::Qmp(device_add_at("usb-hub", "usbhub1", hub)));
    s.push(Step::Qmp(device_add_at("usb-kbd", "usbkbd3", &format!("{hub}.1"))));
    s.push(Step::WaitFor(usb::HUB_KEYS));
    s.push(Step::SendPs2(vec![key("h", true)]));
    s.push(Step::SendPs2(vec![key("h", false)]));
    s.push(Step::WaitFor(usb::HUB_UNPLUG));
    s.push(Step::Qmp(device_del("usbhub1")));
    // Tablet (absolutni osy, report protokol) navazany na konzoli video1.
    s.push(Step::WaitFor(usb::TABLET_PLUG));
    s.push(Step::Qmp(format!(
        r#"{{"execute":"device_add","arguments":{{"driver":"usb-tablet","id":"usbtablet1","bus":"xhci.0","display":"video1"}}}}"#
    )));
    s.push(Step::WaitFor(usb::TABLET_MOVE));
    s.push(Step::SendUsb(vec![abs("x", 0x4000), abs("y", 0x2000)]));
    s.push(Step::SendUsb(vec![btn("left", true)]));
    s.push(Step::SendUsb(vec![btn("left", false)]));
    s.push(Step::WaitFor(usb::TABLET_UNPLUG));
    s.push(Step::Qmp(device_del("usbtablet1")));
}

/// Znacky testu USB (musi sedet s `user/init/src/tests/usb.rs`).
mod media {
    pub const PLUG: &str = "init: test media: zasun flashku";
    pub const UNPLUG: &str = "init: test media: vytahni flashku";
}

mod usb {
    pub const KEYS: &str = "init: test usb: stisky na video1";
    pub const MOUSE_PLUG: &str = "init: test usb: pripoj mys";
    pub const MOUSE_READY: &str = "init: test usb: udalosti mysi";
    pub const MOUSE_UNPLUG: &str = "init: test usb: odpoj mys";
    pub const KBD_PLUG: &str = "init: test usb: pripoj klavesnici";
    pub const KBD_READY: &str = "init: test usb: klavesy nove klavesnice";
    pub const KBD_TRUSTED: &str = "init: test usb: klavesy overene klavesnice";
    pub const KBD_UNPLUG: &str = "init: test usb: odpoj klavesnici";
    pub const MOUSE2_PLUG: &str = "init: test usb: pripoj druhou mys";
    pub const MOUSE2_PENDING: &str = "init: test usb: pohyb neoverene mysi";
    pub const MOUSE2_TRUSTED: &str = "init: test usb: pohyb overene mysi";
    pub const MOUSE2_UNPLUG: &str = "init: test usb: odpoj druhou mys";
    pub const HUB_PLUG: &str = "init: test usb: pripoj hub";
    pub const HUB_KEYS: &str = "init: test usb: klavesy za hubem";
    pub const HUB_UNPLUG: &str = "init: test usb: odpoj hub";
    pub const TABLET_PLUG: &str = "init: test usb: pripoj tablet";
    pub const TABLET_MOVE: &str = "init: test usb: poloha tabletu";
    pub const TABLET_UNPLUG: &str = "init: test usb: odpoj tablet";
}

/// Pripojeni zarizeni USB za behu na radic `xhci`.
fn device_add(driver: &str, id: &str) -> String {
    format!(r#"{{"execute":"device_add","arguments":{{"driver":"{driver}","id":"{id}","bus":"xhci.0"}}}}"#)
}

/// Pripojeni zarizeni na urcity port (`4` = port radice, `4.1` = port 1
/// hubu na nem).
fn device_add_at(driver: &str, id: &str, port: &str) -> String {
    format!(r#"{{"execute":"device_add","arguments":{{"driver":"{driver}","id":"{id}","bus":"xhci.0","port":"{port}"}}}}"#)
}

fn device_del(id: &str) -> String {
    format!(r#"{{"execute":"device_del","arguments":{{"id":"{id}"}}}}"#)
}

/// Precte odpoved na prikaz (preskoci asynchronni udalosti QMP).
fn reply(reader: &mut impl BufRead) -> Result<(), String> {
    reply_events(reader, &mut Vec::new())
}

/// Jako [`reply`]; radky udalosti (`"event"`) prectene cestou da do
/// `events`.
fn reply_events(reader: &mut impl BufRead, events: &mut Vec<String>) -> Result<(), String> {
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            return Err("QMP spojeni skoncilo".into());
        }
        if line.contains("\"return\"") {
            return Ok(());
        }
        if line.contains("\"error\"") {
            return Err(format!("QMP chyba: {}", line.trim()));
        }
        if line.contains("\"event\"") {
            events.push(line);
        }
    }
}

/// Po `device_del` pocka na `DEVICE_DELETED` zarizeni `id`: teprve pak je
/// port volny pro dalsi `device_add` (QEMU odebira asynchronne).
fn wait_deleted(reader: &mut impl BufRead, id: &str, seen: &[String]) -> Result<(), String> {
    let mark = format!("\"device\": \"{id}\"");
    let deleted = |l: &str| l.contains("DEVICE_DELETED") && (l.contains(&mark) || l.contains(&format!("\"device\":\"{id}\"")));
    if seen.iter().any(|l| deleted(l)) {
        return Ok(());
    }
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(20) {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return Err("QMP spojeni skoncilo".into()),
            Ok(_) if deleted(&line) => return Ok(()),
            Ok(_) => {}
            // Limit cteni (10 s) bez udalosti: zkusit dal do celkoveho limitu.
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err(format!("QMP: zarizeni {id} se neodebralo (chybi DEVICE_DELETED)"))
}

/// Id zarizeni z prikazu `device_del` (`None` = jiny prikaz).
fn deleted_id(cmd: &str) -> Option<String> {
    if !cmd.contains("\"device_del\"") {
        return None;
    }
    let i = cmd.find("\"id\"")? + 4;
    let rest = &cmd[i..];
    let a = rest.find('"')? + 1;
    let b = rest[a..].find('"')? + a;
    Some(String::from(&rest[a..b]))
}

/// Projde useky od `first` (dalsi vzdy az po objeveni jeho znacky, useky
/// bez znacky preskoci, s `--repeat` i dalsi kola); vrati pocet
/// provedenych useku. Usek, jehoz test skoncil driv (chybi znacka), se
/// vzda a pokracuje se dalsim.
fn inject(port: u16, log: &mut Log, segs: &[(&'static str, Vec<Step>)], first: usize) -> Result<usize, String> {
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    // Uvitani serveru, pak vyjednani schopnosti.
    let mut greeting = String::new();
    reader
        .read_line(&mut greeting)
        .map_err(|e| e.to_string())?;
    let mut send = |cmd: String, reader: &mut BufReader<TcpStream>| -> Result<(), String> {
        writer
            .write_all(cmd.as_bytes())
            .and_then(|_| writer.write_all(b"\n"))
            .map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        reply_events(reader, &mut events)?;
        if let Some(id) = deleted_id(&cmd) {
            wait_deleted(reader, &id, &events)?;
        }
        Ok(())
    };
    send(r#"{"execute":"qmp_capabilities"}"#.into(), &mut reader)?;
    let mut done = 0;
    let mut next = Some(first);
    while let Some(i) = next {
        done += 1;
        println!("xtask: QMP: usek '{}'", segs[i].0);
        if let Err(e) = run_segment(&mut send, &mut reader, log, &segs[i]) {
            // Spojeni pryc = konec; jina chyba (QMP odmitl prikaz, poloha
            // za znackou) = jen tento usek, dalsi testy dostanou vstup.
            if e.starts_with("QMP spojeni") || !e.starts_with("QMP") && !e.starts_with("poloha") {
                return Err(e);
            }
            eprintln!("xtask: QMP: usek '{}' selhal: {e}; pokracuji dalsim", segs[i].0);
        }
        next = next_segment(log, segs);
    }
    Ok(done)
}

/// Kroky jednoho useku; chyba = usek konci (volajici rozhodne, zda dal).
fn run_segment(
    send: &mut impl FnMut(String, &mut BufReader<TcpStream>) -> Result<(), String>,
    reader: &mut BufReader<TcpStream>,
    log: &mut Log,
    seg: &(&'static str, Vec<Step>),
) -> Result<(), String> {
    let segs = core::slice::from_ref(seg);
    let i = 0;
    {
        'steps: for step in &segs[i].1 {
            let step = step.clone();
            // Cekani na znacku (i s polohou za ni); chybi-li, usek se vzda.
            let rest = match &step {
                Step::WaitFor(m) | Step::ClickAt(m) | Step::DragAt(m) => match log.wait(m, &aborts(m)) {
                    Some(rest) => rest,
                    None => {
                        eprintln!("xtask: QMP: znacka '{m}' se neobjevila, usek '{}' preskocen", segs[i].0);
                        break 'steps;
                    }
                },
                _ => String::new(),
            };
            let (events, device) = match step {
                Step::Send(e) => (e, r#""device":"video0","#),
                Step::SendPs2(e) => (e, ""),
                Step::SendUsb(e) => (e, r#""device":"video1","#),
                Step::Qmp(cmd) => {
                    send(cmd, reader)?;
                    continue;
                }
                Step::WaitFor(_) => continue,
                Step::DragAt(marker) => {
                    let p = numbers(&rest).filter(|v| v.len() >= 4).ok_or_else(|| format!("poloha za znackou '{marker}'"))?;
                    for events in [
                        vec![rel("x", -30000), rel("y", -30000)],
                        vec![rel("x", p[0]), rel("y", p[1]), btn("left", true)],
                        vec![rel("x", (p[2] - p[0]) / 2), rel("y", (p[3] - p[1]) / 2)],
                        vec![rel("x", p[2] - p[0] - (p[2] - p[0]) / 2), rel("y", p[3] - p[1] - (p[3] - p[1]) / 2)],
                        vec![btn("left", false)],
                    ] {
                        let cmd = format!(r#"{{"execute":"input-send-event","arguments":{{"device":"video0","events":[{}]}}}}"#, events.join(","));
                        send(cmd, reader)?;
                    }
                    continue;
                }
                Step::ClickAt(marker) => {
                    let (x, y) = numbers(&rest)
                        .filter(|v| v.len() >= 2)
                        .map(|v| (v[0], v[1]))
                        .ok_or_else(|| format!("poloha za znackou '{marker}'"))?;
                    for events in [
                        vec![rel("x", -30000), rel("y", -30000)],
                        vec![rel("x", x), rel("y", y), btn("left", true)],
                        vec![btn("left", false)],
                    ] {
                        let cmd = format!(r#"{{"execute":"input-send-event","arguments":{{"device":"video0","events":[{}]}}}}"#, events.join(","));
                        send(cmd, reader)?;
                    }
                    continue;
                }
            };
            let cmd = format!(
                r#"{{"execute":"input-send-event","arguments":{{{}"events":[{}]}}}}"#,
                device,
                events.join(",")
            );
            send(cmd, reader)?;
        }
    }
    Ok(())
}

/// Cisla oddelena mezerami (zbytek radku za znackou).
fn numbers(rest: &str) -> Option<Vec<i32>> {
    rest.split_whitespace().map(|v| v.parse::<i32>().ok()).collect()
}

/// Posle jeden prikaz QMP (napr. `system_powerdown`) a pocka na odpoved.
pub fn command(port: u16, cmd: &str) -> Result<(), String> {
    let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| e.to_string())?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(stream);
    let mut greeting = String::new();
    reader.read_line(&mut greeting).map_err(|e| e.to_string())?;
    for c in [r#"{"execute":"qmp_capabilities"}"#, cmd] {
        writer
            .write_all(c.as_bytes())
            .and_then(|_| writer.write_all(b"
"))
            .map_err(|e| e.to_string())?;
        reply(&mut reader)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "qmp_tests.rs"]
mod tests;
