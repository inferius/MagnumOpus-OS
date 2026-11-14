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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

/// Musi sedet s `QEMU_MARKER` v `user/init/src/tests/input.rs`.
pub const MARKER: &str = "init: test vstupu: cekam na udalosti z QEMU";

/// Jak dlouho nejdyl cekat na znacku (cely beh testu).
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

/// Pocka, az se v logu objevi `marker`; `false` = nedockal se.
fn wait_marker(log: &Path, marker: &str, start: Instant) -> bool {
    loop {
        if start.elapsed() > WAIT_MAX {
            return false;
        }
        let seen = std::fs::read(log)
            .map(|b| String::from_utf8_lossy(&b).contains(marker))
            .unwrap_or(false);
        if seen {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Spusti vlakno, ktere po objeveni znacky v `log` posle udalosti. `ps2`
/// = stroj ma radic PS/2; bez nej test PS/2 svou znacku nevypise a kroky
/// PS/2 se preskoci (jinak by vlakno na znacku cekalo a k USB se nedostalo).
/// `port` cte az pri vkladani: opakovany start QEMU (souboj o port) ho
/// zmeni.
pub fn spawn_injector(log: PathBuf, port: Arc<AtomicU16>, ps2: bool) {
    std::thread::spawn(move || {
        let start = Instant::now();
        let segs = segments(ps2);
        // Pripojit az na zacatku prvniho useku (QEMU uz bezi).
        let Some(first) = next_segment(&log, &segs, 0, start) else { return };
        match inject(port.load(Ordering::Relaxed), &log, start, &segs, first) {
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
    gui::DIALOG,
    gui::SECURE,
    gui::TERM_DRAG,
    gui::CLICK,
    gui::SNAPSHOT,
    gui::PALETTE_SWITCH,
    gui::SHOT[0],
    gui::FILES_CLICK,
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

/// Pocka na zacatek nektereho useku od `from`; vrati jeho index (nejdriv
/// zacinajici z tech, jejichz znacka v logu je). `None` = beh testu skoncil
/// nebo vyprsel limit.
fn next_segment(log: &Path, segs: &[(&'static str, Vec<Step>)], from: usize, start: Instant) -> Option<usize> {
    loop {
        if start.elapsed() > WAIT_MAX {
            return None;
        }
        let text = std::fs::read(log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        if let Some(j) = (from..segs.len()).find(|&j| text.contains(segs[j].0)) {
            return Some(j);
        }
        if END.iter().any(|e| text.contains(e)) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
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
    gui_steps(&mut s);
    s
}

/// GUI (`user/init/src/tests/gui.rs`): fyzicka klavesnice odpovi na
/// systemovy dialog (umely vstup ho nepotvrdi) a na bezpecnou obrazovku.
fn gui_steps(s: &mut Vec<Step>) {
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
    pub const DIALOG: &str = "init: test gui: dialog ceka na klavesy z QEMU";
    pub const SECURE: &str = "init: test gui: cekam na zkratku z QEMU";
    pub const SECURE_ENTER: &str = "init: test gui: bezpecna obrazovka ceka na Enter";
    pub const CLICK: &str = "init: test gui: klik mysi na ";
    /// `user/init/src/tests/gui_term.rs`.
    pub const TERM_DRAG: &str = "init: test gui: terminal tah mysi ";
    pub const TERM_KEYS: &str = "init: test gui: terminal ceka na Ctrl+C a Ctrl+V";
    pub const TERM_TAB: &str = "init: test gui: terminal ceka na doplneni Tab";
    pub const TERM_DOWN: &str = "init: test gui: terminal ceka na sipku v seznamu navrhu";
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
    s.push(Step::Qmp(device_add_at("usb-hub", "usbhub1", "4")));
    s.push(Step::Qmp(device_add_at("usb-kbd", "usbkbd3", "4.1")));
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
    }
}

/// Projde useky od `first` (dalsi vzdy az po objeveni jeho znacky, useky
/// bez znacky preskoci); vrati pocet provedenych useku.
fn inject(port: u16, log: &Path, start: Instant, segs: &[(&'static str, Vec<Step>)], first: usize) -> Result<usize, String> {
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
        reply(reader)
    };
    send(r#"{"execute":"qmp_capabilities"}"#.into(), &mut reader)?;
    let mut done = 0;
    let mut next = Some(first);
    while let Some(i) = next {
        done += 1;
        println!("xtask: QMP: usek '{}'", segs[i].0);
        for step in &segs[i].1 {
            let step = step.clone();
            let (events, device) = match step {
                Step::Send(e) => (e, r#""device":"video0","#),
                Step::SendPs2(e) => (e, ""),
                Step::SendUsb(e) => (e, r#""device":"video1","#),
                Step::Qmp(cmd) => {
                    send(cmd, &mut reader)?;
                    continue;
                }
                Step::WaitFor(marker) => {
                    if !wait_marker(log, marker, start) {
                        return Err(format!("znacka '{marker}' se neobjevila"));
                    }
                    continue;
                }
                Step::DragAt(marker) => {
                    if !wait_marker(log, marker, start) {
                        return Err(format!("znacka '{marker}' se neobjevila"));
                    }
                    let p = marker_numbers(log, marker).filter(|v| v.len() >= 4).ok_or_else(|| format!("poloha za znackou '{marker}'"))?;
                    for events in [
                        vec![rel("x", -30000), rel("y", -30000)],
                        vec![rel("x", p[0]), rel("y", p[1]), btn("left", true)],
                        vec![rel("x", (p[2] - p[0]) / 2), rel("y", (p[3] - p[1]) / 2)],
                        vec![rel("x", p[2] - p[0] - (p[2] - p[0]) / 2), rel("y", p[3] - p[1] - (p[3] - p[1]) / 2)],
                        vec![btn("left", false)],
                    ] {
                        let cmd = format!(r#"{{"execute":"input-send-event","arguments":{{"device":"video0","events":[{}]}}}}"#, events.join(","));
                        send(cmd, &mut reader)?;
                    }
                    continue;
                }
                Step::ClickAt(marker) => {
                    if !wait_marker(log, marker, start) {
                        return Err(format!("znacka '{marker}' se neobjevila"));
                    }
                    let (x, y) = marker_point(log, marker).ok_or_else(|| format!("poloha za znackou '{marker}'"))?;
                    for events in [
                        vec![rel("x", -30000), rel("y", -30000)],
                        vec![rel("x", x), rel("y", y), btn("left", true)],
                        vec![btn("left", false)],
                    ] {
                        let cmd = format!(r#"{{"execute":"input-send-event","arguments":{{"device":"video0","events":[{}]}}}}"#, events.join(","));
                        send(cmd, &mut reader)?;
                    }
                    continue;
                }
            };
            let cmd = format!(
                r#"{{"execute":"input-send-event","arguments":{{{}"events":[{}]}}}}"#,
                device,
                events.join(",")
            );
            send(cmd, &mut reader)?;
        }
        next = next_segment(log, segs, i + 1, start);
    }
    Ok(done)
}

/// Dve cisla "x y" za znackou v logu.
fn marker_point(log: &Path, marker: &str) -> Option<(i32, i32)> {
    let v = marker_numbers(log, marker)?;
    Some((*v.first()?, *v.get(1)?))
}

/// Cisla oddelena mezerami za znackou v logu (do konce radku).
fn marker_numbers(log: &Path, marker: &str) -> Option<Vec<i32>> {
    let b = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(&b);
    let rest = &text[text.find(marker)? + marker.len()..];
    rest.lines().next()?.split_whitespace().map(|v| v.parse::<i32>().ok()).collect()
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
