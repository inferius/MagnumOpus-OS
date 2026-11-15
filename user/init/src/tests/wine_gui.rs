//! Okna programu Windows (W3, `docs/cs/windows-vrstva.md`, kapitola W3):
//! Wine s ovladacem grafiky `winemo.drv` nad kompozitorem (sada `wine`,
//! beh s `--wine`, release). Kompozitor spusti prvni test a drzi ho
//! staticky stav; programy Wine dostanou relaci okna roli `Gui` se
//! stejnou identitou `org.winehq.wine` (rozhodnuti 2A) od supervisoru
//! testu.
//!
//! - GDI: barevne obdelniky z `FillRect` jsou v kompozitoru presne,
//! - ramy: okno s titulkem ma ram a odznak od systemu, okno s vlastni
//!   neklientskou oblasti (`WM_NCCALCSIZE`) je bez ramu,
//! - poznamkovy blok: okno se otevre, prijme fyzicky vstup z QEMU (klik,
//!   psani, Ctrl+S, dialog Ulozit jako jako vlastnene okno), soubor je na
//!   disku, Alt+F4 okno zavre a proces skonci,
//! - winecfg: okno konfigurace se otevre.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::control::{ConnectInfo, INSPECT, WindowInfo};
use launchproto::{Launch, Role};
use libos::{Channel, Error, Process, Startup, println, time};

use super::gui::{badge, badge_inactive, pixel, settle, windows};
use super::wine::{APP, TESTS_APP, WINE_ENV, fs};
use super::{TestResult, devices, fail};
use crate::check;
use crate::gui::{self, Gui};
use crate::supervisor::Supervisor;

/// Identita vsech programu Windows (2A).
const WINE_ID: &str = "org.winehq.wine";
const LIMIT: Duration = Duration::from_secs(180);

/// Znacky pro fyzicky vstup z QEMU (musi sedet s `xtask/src/qmp.rs`, `wine`).
const QEMU_CLICK: &str = "init: test wine: klik do poznamkoveho bloku na ";
const QEMU_TYPE: &str = "init: test wine: pis do poznamkoveho bloku";
const QEMU_SAVE_KEYS: &str = "init: test wine: Ctrl+S";
const QEMU_SAVE: &str = "init: test wine: dialog ulozeni";
const QEMU_CLOSE: &str = "init: test wine: zavri poznamkovy blok";

struct W3 {
    gui: Gui,
    inspect: Channel,
    sup: Supervisor,
}

static STATE: spin::Mutex<Option<W3>> = spin::Mutex::new(None);

fn identity() -> ConnectInfo {
    ConnectInfo { system: false, id: String::from(WINE_ID), name: String::from("Programy Windows"), perms: Vec::new(), actions: Vec::new() }
}

/// Kompozitor pro testy W3 (jednou), vstup od `inputd`, supervisor testu
/// s kompozitorem v registru (listky role Gui).
fn with<R>(s: &Startup, f: impl FnOnce(&mut W3) -> Result<R, String>) -> Result<R, String> {
    let mut g = STATE.lock();
    if g.is_none() {
        let gui = gui::start(s).map_err(|e| fail("start kompozitoru", e))?;
        let input = devices().and_then(|d| d.input.as_ref()).ok_or("sluzba vstupu nebezi")?;
        gui.take_input(input).map_err(|e| fail("predani vstupu", e))?;
        let inspect = gui.channel(INSPECT.as_bytes()).map_err(|e| fail("inspect", e))?;
        let mut sup = Supervisor::for_tests(Duration::from_secs(10));
        gui.register(&mut sup);
        *g = Some(W3 { gui, inspect, sup });
    }
    f(g.as_mut().unwrap())
}

/// Bezici program Wine (proces, vystup).
struct Run {
    process: Process,
    out: Channel,
    err: Channel,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Run {
    fn drain(&mut self) {
        while let Ok(m) = self.out.try_read() {
            self.stdout.extend_from_slice(&m.data);
        }
        while let Ok(m) = self.err.try_read() {
            self.stderr.extend_from_slice(&m.data);
        }
    }

    fn text(&self) -> String {
        let err = String::from_utf8_lossy(&self.stderr);
        // Hlaseni fixme Wine nejsou chyby testu.
        let err: Vec<&str> = err.lines().filter(|l| !l.contains(":fixme:")).collect();
        format!("stdout: {}; stderr: {}", String::from_utf8_lossy(&self.stdout).trim(), err.join(" | "))
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

/// Spusti `wine <args>` pres launchd s relaci okna (role Gui) od
/// supervisoru testu.
fn launch(s: &Startup, w: &mut W3, args: &[&str]) -> Result<Run, String> {
    let loader = format!("{APP}/lib/wine/x86_64-unix/wine");
    let (session, lease) = w.sup.session(gui::NAME, identity().encode()).map_err(|e| fail("relace okna", e))?;
    let fsc = fs()?.connect().map_err(|e| fail("relace FS", e))?;
    let launch = crate::launch::client().map_err(|e| fail("kanal k launchd", e))?;
    let params = libos::params::Params {
        cwd: b"/".to_vec(),
        args: core::iter::once(loader.as_str()).chain(args.iter().copied()).map(|a| a.as_bytes().to_vec()).collect(),
        // Ladeni padu: pridat "WINEDEBUG=+seh" (vyjimky Wine) a
        // "MO_SIGDEBUG=1" (kazdy synchronni signal v cstd).
        env: WINE_ENV.iter().map(|e| e.as_bytes().to_vec()).collect(),
        flags: 0,
        fds: Vec::new(),
    }
    .encode()
    .map_err(|e| fail("blok parametru", e))?;
    let roles = vec![Role::Job, Role::Fs, Role::Stdout, Role::Stderr, Role::Gui, Role::GuiLease];
    let req = Launch { program: loader.clone(), flags: 0, roles, params }.encode(1).map_err(|e| fail("pozadavek", e))?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let (out, out_child) = Channel::create().map_err(|e| fail("kanal stdout", e))?;
    let (err, err_child) = Channel::create().map_err(|e| fail("kanal stderr", e))?;
    launch
        .write(
            &req,
            vec![
                job.duplicate(None).map_err(|e| fail("job", e))?.into_handle(),
                fsc.into_channel().into_handle(),
                out_child.into_handle(),
                err_child.into_handle(),
                session.into_handle(),
                lease.into_handle(),
            ],
        )
        .map_err(|e| fail("LAUNCH", e))?;
    let end = time::deadline_after(Duration::from_secs(60));
    let m = loop {
        if let Ok(m) = launch.try_read() {
            break m;
        }
        if time::now() > end {
            return Err(String::from("launchd neodpovedel"));
        }
        w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
    };
    let (_, r) = launchproto::parse_reply(&m.data).map_err(|e| fail("odpoved launchd", e))?;
    if let Err((e, msg)) = r {
        return Err(format!("launchd: {e}: {msg}"));
    }
    let mut hs = m.handles;
    let _control = hs.pop();
    let process = Process::from_handle(hs.pop().ok_or("chybi proces")?);
    Ok(Run { process, out, err, stdout: Vec::new(), stderr: Vec::new() })
}

/// Ridi supervisor testu, dokud `f` nevrati `Some` (nebo program
/// neskonci, nebo nevyprsi `LIMIT`).
fn wait_for<T>(s: &Startup, w: &mut W3, run: &mut Run, what: &str, mut f: impl FnMut(&mut W3, &Run) -> Result<Option<T>, String>) -> Result<T, String> {
    let end = time::deadline_after(LIMIT);
    loop {
        run.drain();
        if let Some(v) = f(w, run)? {
            return Ok(v);
        }
        if let Ok(code) = run.process.wait_exit(Some(0)) {
            run.drain();
            return Err(format!("{what}: program skoncil s kodem {code}; {}", run.text()));
        }
        if time::now() > end {
            return Err(format!("{what}: vyprsel cas; {}", run.text()));
        }
        w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
    }
}

/// Okna programu Windows s titulkem obsahujicim `title`.
fn wine_windows(w: &W3, title: &str) -> Result<Vec<WindowInfo>, String> {
    Ok(windows(&w.inspect)?.into_iter().filter(|x| x.app == WINE_ID && x.title.contains(title)).collect())
}

fn wait_window(s: &Startup, w: &mut W3, run: &mut Run, title: &str) -> Result<WindowInfo, String> {
    wait_for(s, w, run, &format!("okno '{title}'"), |w, _| Ok(wine_windows(w, title)?.into_iter().next()))
}

/// Pocka, az ma pixel obrazovky barvu `want` (nove okno se ukazuje
/// animaci); vrati posledni prectenou barvu.
fn wait_pixel(w: &W3, x: i32, y: i32, want: u32) -> Result<u32, String> {
    let end = time::deadline_after(Duration::from_secs(5));
    loop {
        let p = pixel(&w.inspect, x, y)?;
        if p == want || time::now() > end {
            return Ok(p);
        }
        time::sleep(Duration::from_millis(100));
    }
}

/// Pocet nebilych pixelu v oblasti `x0..x0+w`, `y0..y0+h` (krok 3 px):
/// text v editoru proti prazdnemu stavu.
fn ink(w: &W3, x0: i32, y0: i32, wd: i32, ht: i32) -> Result<u32, String> {
    let mut n = 0;
    for y in (y0..y0 + ht).step_by(3) {
        for x in (x0..x0 + wd).step_by(3) {
            if pixel(&w.inspect, x, y)? & 0x00FF_FFFF != 0x00FF_FFFF {
                n += 1;
            }
        }
    }
    Ok(n)
}

/// Ukonci program a pocka, az jeho okna zmizi z kompozitoru.
fn finish(s: &Startup, w: &mut W3, run: Run, title: &str) -> TestResult {
    let _ = run.process.kill();
    let _ = run.process.wait_exit(Some(time::deadline_after(Duration::from_secs(10))));
    drop(run);
    let end = time::deadline_after(Duration::from_secs(30));
    while !wine_windows(w, title)?.is_empty() {
        if time::now() > end {
            return Err(format!("okno '{title}' po konci programu zustalo"));
        }
        w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
    }
    Ok(())
}

pub fn gdi(s: &Startup) -> TestResult {
    with(s, |w| {
        let exe = format!("{TESTS_APP}/wintest.exe");
        let mut run = launch(s, w, &[&exe, "gdi"])?;
        wait_for(s, w, &mut run, "GDI kresleni", |_, r| Ok(String::from_utf8_lossy(&r.stdout).contains("nakresleno").then_some(())))?;
        let win = wait_window(s, w, &mut run, "wintest gdi")?;
        settle(&w.inspect)?;
        let (x, y) = (win.rect.x, win.rect.y);
        for (dx, name, want) in [(35, "cervena", 0xFFFF_0000u32), (95, "zelena", 0xFF00_FF00), (155, "modra", 0xFF00_00FF)] {
            let p = wait_pixel(w, x + dx, y + 35, want)?;
            check!(p == want, format!("{name} z FillRect: {p:08x} na {},{} (okno {:?})", x + dx, y + 35, win.rect));
        }
        let p = wait_pixel(w, x + 250, y + 150, 0xFFFF_FFFF)?;
        check!(p == 0xFFFF_FFFF, format!("bile pozadi okna: {p:08x}"));
        println!("init: test wine: GDI okno {}x{} na {},{}", win.rect.w, win.rect.h, x, y);
        finish(s, w, run, "wintest gdi")
    })
}

pub fn frames(s: &Startup) -> TestResult {
    with(s, |w| {
        let exe = format!("{TESTS_APP}/wintest.exe");
        let mut run = launch(s, w, &[&exe, "ramy"])?;
        wait_for(s, w, &mut run, "kresleni oken", |_, r| Ok(String::from_utf8_lossy(&r.stdout).contains("nakresleno").then_some(())))?;
        let std = wait_window(s, w, &mut run, "ramy-standard")?;
        let own = wait_window(s, w, &mut run, "ramy-vlastni")?;
        settle(&w.inspect)?;
        // Okno s titulkem: odznak duvery v titulku kresli system.
        let b = pixel(&w.inspect, std.rect.x + 9, std.rect.y - 16)?;
        check!(b == badge() || b == badge_inactive(), format!("odznak v systemovem titulku okna s ramem: {b:08x}"));
        // Vlastni neklientska oblast: obsah zacina zelenym pruhem aplikace
        // a nad nim zadny systemovy titulek (stred mista titulku ma jinou
        // barvu nez titulek okna s ramem; odznak se u okna bez ramu
        // ukazuje u okraje, ne uprostred).
        let p = wait_pixel(w, own.rect.x + 20, own.rect.y + 5, 0xFF00_FF00)?;
        check!(p == 0xFF00_FF00, format!("pruh aplikace v okne bez ramu: {p:08x}"));
        let title = pixel(&w.inspect, std.rect.x + std.rect.w as i32 / 2, std.rect.y - 16)?;
        let above = pixel(&w.inspect, own.rect.x + own.rect.w as i32 / 2, own.rect.y - 16)?;
        check!(above != title, format!("okno bez ramu nema systemovy titulek: {above:08x} (titulek okna s ramem {title:08x})"));
        finish(s, w, run, "ramy-")
    })
}

pub fn notepad(s: &Startup) -> TestResult {
    with(s, |w| {
        // Dialog Ulozit jako zacina ve vychozim adresari (podle verze Wine
        // dokumenty uzivatele, nebo pracovni adresar = Z:\ = koren FS).
        let docs: Vec<String> = fs()?
            .list("/home/wine/.wine/drive_c/users")
            .map(|l| l.into_iter().map(|e| format!("/home/wine/.wine/drive_c/users/{}/Documents/t.txt", e.name)).collect())
            .unwrap_or_default();
        let candidates: Vec<String> = ["/t.txt", "/home/wine/.wine/drive_c/t.txt"].iter().map(|s| String::from(*s)).chain(docs).collect();
        for c in &candidates {
            let _ = fs()?.remove(c);
        }
        let mut run = launch(s, w, &["notepad"])?;
        let win = wait_window(s, w, &mut run, "Notepad")?;
        settle(&w.inspect)?;
        let (cx, cy) = (win.rect.x + win.rect.w as i32 / 2, win.rect.y + win.rect.h as i32 / 2);
        let p = wait_pixel(w, cx, cy, 0xFFFF_FFFF)?;
        check!(p == 0xFFFF_FFFF, format!("bila plocha editoru: {p:08x} (okno {:?})", win.rect));
        // Oblast textu: levy horni roh editoru (pod nabidkou okna).
        let (tx, ty) = (win.rect.x + 2, win.rect.y + 20);
        let empty = ink(w, tx, ty, 90, 30)?;
        // Fyzicky vstup z QEMU: klik a text.
        println!("{QEMU_CLICK}{cx} {cy}");
        time::sleep(Duration::from_millis(500));
        println!("{QEMU_TYPE}");
        // Napsany text je videt: v oblasti textu pribyl inkoust.
        let end = time::deadline_after(Duration::from_secs(30));
        let typed = loop {
            run.drain();
            w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
            let n = ink(w, tx, ty, 90, 30)?;
            if n >= empty + 8 || time::now() > end {
                break n;
            }
            time::sleep(Duration::from_millis(200));
        };
        check!(typed >= empty + 8, format!("napsany text neni videt: inkoust {typed}, prazdny editor {empty}; {}", run.text()));
        println!("{QEMU_SAVE_KEYS}");
        // Dialog Ulozit jako: vlastnene okno (WINDOW_OWNER) nad editorem.
        let dlg = wait_for(s, w, &mut run, "dialog Ulozit jako", |w, _| Ok(wine_windows(w, "Save")?.into_iter().next()))?;
        check!(dlg.id != win.id, "dialog je vlastni okno");
        // Dialog prevezme fokus (vlastnene okno); chvili na jeho inicializaci.
        let end = time::deadline_after(Duration::from_secs(10));
        while !wine_windows(w, "Save")?.iter().any(|d| d.focused) && time::now() < end {
            w.sup.step(s, Some(time::deadline_after(Duration::from_millis(50))));
        }
        check!(wine_windows(w, "Save")?.iter().any(|d| d.focused), "dialog Ulozit jako ma fokus");
        settle(&w.inspect)?;
        time::sleep(Duration::from_millis(500));
        println!("{QEMU_SAVE}");
        let fs = fs()?;
        let data = wait_for(s, w, &mut run, "ulozeny soubor", |_, _| Ok(candidates.iter().find_map(|c| fs.read(c, 0, 4096).ok().filter(|d| !d.is_empty())))).map_err(|e| {
            let wins: Vec<String> = windows(&w.inspect).unwrap_or_default().iter().map(|x| format!("'{}' {:?} fokus {}", x.title, x.rect, x.focused)).collect();
            let files: Vec<String> = fs.list("/home/wine/.wine/drive_c").map(|l| l.into_iter().map(|e| e.name).collect()).unwrap_or_default();
            format!("{e}; okna: {}; drive_c: {}", wins.join(", "), files.join(" "))
        })?;
        let text = String::from_utf8_lossy(&data);
        check!(text.contains("ahoj"), format!("obsah ulozeneho souboru: {text:?}"));
        // Alt+F4: okno se zavre a proces skonci.
        println!("{QEMU_CLOSE}");
        let end = time::deadline_after(Duration::from_secs(60));
        let code = loop {
            match run.process.wait_exit(Some(time::deadline_after(Duration::from_millis(20)))) {
                Ok(c) => break c,
                Err(Error::TimedOut) if time::now() <= end => {
                    w.sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
                }
                Err(e) => return Err(format!("konec poznamkoveho bloku: {e}; {}", run.text())),
            }
        };
        check!(code == 0, format!("kod konce poznamkoveho bloku {code}"));
        finish(s, w, run, "Notepad")
    })
}

pub fn winecfg(s: &Startup) -> TestResult {
    with(s, |w| {
        let mut run = launch(s, w, &["winecfg"])?;
        // Dialog vlastnosti se ukaze maly a pak dostane svou velikost.
        let win = wait_for(s, w, &mut run, "okno winecfg", |w, _| {
            Ok(wine_windows(w, "configuration")?.into_iter().find(|x| x.rect.w >= 200 && x.rect.h >= 200))
        })
        .map_err(|e| {
            let list: Vec<String> = windows(&w.inspect).unwrap_or_default().iter().map(|x| format!("{} '{}' {:?}", x.app, x.title, x.rect)).collect();
            format!("{e}; okna: {}", list.join(", "))
        })?;
        println!("init: test wine: winecfg {:?}", win.rect);
        settle(&w.inspect)?;
        finish(s, w, run, "configuration")
    })
}
