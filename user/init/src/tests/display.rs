//! Nastaveni obrazu v QEMU: ovladac `bochs-display` (na aarch64
//! `-device bochs-display`, na x86 `secondary-vga` - stejne registry
//! DISPI) a vlastni instance kompozitoru `compd` nad nim: vycet
//! konektoru a rezimu z EDID, zmena rozliseni za behu, page flip s vsync
//! a bezpecny navrat na stav firmwaru po padu ovladace.
//!
//! Hlavni GUI testu bezi na virtio-gpu, takze sada bere posledni nativni
//! ovladac (na x86 druha karta; prvni je karta firmwaru s logem jadra).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::control;
use guiproto::display::{Reply, Request, caps};
use libos::initfs::Initfs;
use libos::{Channel, Handle, Process, Startup, println, time};

use super::{Test, TestResult, fail};
use crate::display_native::{self, Kit, NATIVES};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("startovni obraz: kresba bez FPU = reference hostitele", splash_determinism),
    ("bochs-display: konektory a rezimy z EDID", connectors),
    ("bochs-display: compd navaze na startovni obraz, nastavi rezim, zmena rozliseni za behu", compd_mode_change),
    ("bochs-display: pad ovladace, obnova stavu firmwaru, RESET", crash_restores_firmware),
];

const TIMEOUT: Duration = Duration::from_secs(10);

/// Stav sady: ovladac a kompozitor.
struct State {
    kit: Kit,
    /// Rezim firmwaru pred testy (`WxH`, `-` = vystup vypnuty).
    firmware: String,
    compd: Option<(Process, Channel)>,
}

static STATE: spin::Mutex<Option<State>> = spin::Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> Result<R, String>) -> Result<R, String> {
    let mut g = STATE.lock();
    if g.is_none() {
        // Posledni nahlaseny ovladac bochs-display.
        let mut list = NATIVES.lock();
        let i = list.iter().rposition(|k| k.program == "bochs-display").ok_or("devmgr nenahlasil zadny bochs-display")?;
        *g = Some(State { kit: list.remove(i), firmware: String::from("?"), compd: None });
    }
    f(g.as_mut().unwrap())
}

fn deadline() -> Option<u64> {
    Some(time::deadline_after(TIMEOUT))
}

/// Dalsi odpoved ovladace (udalosti preskoci).
fn reply(ch: &Channel) -> Result<Reply, String> {
    loop {
        let m = ch.read(deadline()).map_err(|e| fail("odpoved ovladace", e))?;
        let r = Reply::decode(&m.data).map_err(|e| fail("dekodovani odpovedi", e))?;
        if !r.is_event() {
            return Ok(r);
        }
    }
}

/// Loader kresli s FPU, proces `splash` a kompozitor s plovouci carkou
/// v softwaru: otisk kresby musi byt stejny jako na hostiteli (jinak by
/// prechod mezi nimi poskakoval).
fn splash_determinism(_: &Startup) -> TestResult {
    let t = time::now();
    let v = bootsplash::self_check();
    println!("init: test displej: otisk startovniho obrazu {v:#018x} ({} ms)", (time::now() - t) / 1_000_000);
    check_eq!(v, bootsplash::SELF_CHECK, "otisk kresby");
    Ok(())
}

fn connectors(_: &Startup) -> TestResult {
    with(|s| {
        let ch = s.kit.client.as_ref().ok_or("kanal klienta chybi")?;
        let c = display_native::probe(ch).map_err(|e| fail("probe", e))?;
        println!("init: test displej: konektor {} '{}', {} rezimu", guiproto::modes::kind::name(c.kind), c.monitor, c.modes.len());
        check!(c.connected, "monitor neni pripojen");
        s.firmware = c.current().map(|m| format!("{}x{}", m.w, m.h)).unwrap_or_else(|| String::from("-"));
        check!(c.modes.len() >= 3, "malo rezimu (EDID z QEMU)");
        check!(c.preferred().is_some(), "chybi preferovany rezim");
        check!(c.modes.iter().any(|m| (m.w, m.h) == (1024, 768)), "chybi 1024x768");
        ch.write(&Request::Info.encode(), Vec::new()).map_err(|e| fail("INFO", e))?;
        match reply(ch)? {
            Reply::Modes { caps: c, .. } => check!(c & caps::MODESET != 0 && c & caps::VSYNC != 0, "ovladac nehlasi MODESET a VSYNC"),
            r => return Err(format!("INFO: {r:?}")),
        }
        // Neplatny rezim se odmitne.
        ch.write(&Request::SetMode { output: 0, w: 1234, h: 567, refresh_mhz: 0 }.encode(), Vec::new()).map_err(|e| fail("SET_MODE", e))?;
        check!(matches!(reply(ch)?, Reply::Error { .. }), "neplatny rezim prijat");
        Ok(())
    })
}

/// Ridici prikaz kompozitoru, odpoved `ok ...`.
fn command(ctl: &Channel, text: &str, handles: Vec<Handle>) -> Result<String, String> {
    command_within(ctl, text, handles, TIMEOUT)
}

/// Jako [`command`] s vlastni horni mezi cekani na odpoved.
fn command_within(ctl: &Channel, text: &str, handles: Vec<Handle>, limit: Duration) -> Result<String, String> {
    ctl.write(text.as_bytes(), handles).map_err(|e| fail(text, e))?;
    let m = ctl.read(Some(time::deadline_after(limit))).map_err(|e| fail(text, e))?;
    let r = String::from_utf8_lossy(&m.data).into_owned();
    match r.strip_prefix("ok") {
        Some(rest) => Ok(String::from(rest.trim())),
        None => Err(format!("{text}: {r}")),
    }
}

fn start_compd(startup: &Startup, display: Channel) -> Result<(Process, Channel, String), String> {
    let fs = Initfs::open(startup).map_err(|e| fail("initfs", e))?;
    let image = fs.find("compd").ok_or("compd chybi v initfs")?;
    let (p, ctl) = libos::spawn("compd", image, Vec::new()).map_err(|e| fail("spawn compd", e))?;
    // Jako prvni start systemu: na obrazovce je startovni obraz (prstenec
    // na 60 %), kompozitor na nej navaze a prejde do plochy.
    // Odpoved = displej pripojen a prvni snimek ukazany; horni mez je
    // velkorysa (pomaly stroj, zatizeny test), cas se meri v logu compd.
    let size = match command_within(&ctl, &format!("{} UTC {}600", control::DISPLAY, control::SPLASH_ARG), alloc::vec![display.into_handle()], Duration::from_secs(30)) {
        Ok(s) => s,
        Err(e) => {
            stop_compd(&p);
            return Err(e);
        }
    };
    Ok((p, ctl, size))
}

/// Ukonci kompozitor sady (i po selhani testu: jinak by drzel pamet
/// a dalsi sady by o ni prisly).
fn stop_compd(p: &Process) {
    let _ = p.kill();
    let _ = p.wait_exit(deadline());
}

/// Aktualni rezim z `outputs` (`WxH@mHz`).
fn current(ctl: &Channel) -> Result<String, String> {
    let text = command(ctl, control::OUTPUTS, Vec::new())?;
    let line = text.lines().next().ok_or("outputs je prazdne")?;
    Ok(String::from(line.split_whitespace().nth(3).unwrap_or("?")))
}

fn compd_mode_change(startup: &Startup) -> TestResult {
    with(|s| {
        // Kompozitor dostane duplikat konce (jako v systemu, kde konec drzi
        // init, `gui::display`): jeho konec pak neukonci ovladac.
        let ch = s.kit.client.as_ref().ok_or("kanal klienta chybi")?;
        let ch = Channel::from_handle(ch.handle().duplicate(None).map_err(|e| fail("duplikat konce klienta", e))?);
        let (p, ctl, size) = start_compd(startup, ch)?;
        println!("init: test displej: compd nad bochs-display {size}");
        let r = mode_change_checks(&ctl, &size);
        match r {
            Ok(()) => s.compd = Some((p, ctl)),
            Err(_) => stop_compd(&p),
        }
        r
    })
}

/// Prubeh testu nad bezicim kompozitorem (`compd_mode_change`).
fn mode_change_checks(ctl: &Channel, size: &str) -> Result<(), String> {
    {
        // Prechod ze startovniho obrazu: kompozitor mezitim odpovida
        // a plochu sklada dal (do bufferu). Konec pozna podle `splash=0`
        // ve `status` (zadna pevna lhuta podle ocekavane delky).
        let end = time::deadline_after(Duration::from_secs(30));
        loop {
            let st = command(&ctl, control::STATUS, Vec::new())?;
            let st = control::Status::parse(&st).ok_or("status behem prechodu")?;
            if !st.splash {
                break;
            }
            check!(time::now() < end, "prechod ze startovniho obrazu neskoncil do 30 s");
            time::sleep(Duration::from_millis(50));
        }
        // Vystup byl vypnuty: kompozitor nastavil preferovany rezim.
        let cur = current(&ctl)?;
        check!(cur.starts_with(&size.replace(' ', "x")), "aktualni rezim nesedi s velikosti kompozitoru");
        let r = command(&ctl, "mode 800 600", Vec::new())?;
        check!(r.starts_with("800 600"), "mode 800 600");
        check!(current(&ctl)?.starts_with("800x600@"), "rezim po zmene neni 800x600");
        let r = command(&ctl, "mode 1024 768 60000", Vec::new())?;
        check!(r.starts_with("1024 768"), "mode 1024 768");
        // Neplatny rezim: chyba, rezim zustava.
        check!(command(&ctl, "mode 1234 567", Vec::new()).is_err(), "neplatny rezim prijat");
        check!(current(&ctl)?.starts_with("1024x768@"), "rezim se zmenil po chybe");
        // Snimky se skladaji a potvrzuji (vsync ovladace): pocet zobrazenych roste.
        let presented = |ctl: &Channel| -> Result<u64, String> {
            let st = command(ctl, control::STATUS, Vec::new())?;
            let st = control::Status::parse(&st).ok_or("status nelze precist")?;
            Ok(st.presented)
        };
        let a = presented(&ctl)?;
        let _ = command(&ctl, "mode 800 600", Vec::new())?;
        let end = time::deadline_after(Duration::from_secs(3));
        while presented(&ctl)? <= a && time::now() < end {
            time::sleep(Duration::from_millis(50));
        }
        check!(presented(&ctl)? > a, "po zmene rezimu nepribyl zobrazeny snimek");
        Ok(())
    }
}

fn crash_restores_firmware(startup: &Startup) -> TestResult {
    let r = with(|s| {
        let (_, ctl) = s.compd.as_ref().ok_or("kompozitor nebezi (predchozi test selhal)")?;
        let first = s.kit.process.take().ok_or("prvni instance ovladace chybi")?;
        first.kill().map_err(|e| fail("kill", e))?;
        first.wait_exit(deadline()).map_err(|e| fail("konec ovladace", e))?;
        // Jako supervisor: nova instance v rezimu obnovy.
        let p = s.kit.spawn(startup, true).map_err(|e| fail("restart ovladace", e))?;
        // Kompozitor dostane RESET a znovu vybere vystup: rezim firmwaru
        // (OVMF zapina i druhou kartu), jediny rezim, zmena nejde.
        let fw = s.firmware.clone();
        let end = time::deadline_after(Duration::from_secs(5));
        let mut lines = command(ctl, control::OUTPUTS, Vec::new())?;
        let modes_of = |l: &str| l.lines().next().and_then(|l| l.split_whitespace().nth(4)).map(String::from).unwrap_or_default();
        // Firmware vystup zapnul (x86 OVMF) -> jediny rezim; nechal ho
        // vypnuty (aarch64) -> zadny rezim.
        let want = if fw == "-" { "0" } else { "1" };
        while modes_of(&lines) != want && time::now() < end {
            time::sleep(Duration::from_millis(100));
            lines = command(ctl, control::OUTPUTS, Vec::new())?;
        }
        let n = modes_of(&lines);
        check_eq!(n.as_str(), want, "pocet rezimu po obnove (jen rezim firmwaru)");
        let cur = current(ctl)?;
        check!(if fw == "-" { cur == "-" } else { cur.starts_with(&fw) }, "rezim po obnove neni rezim firmwaru");
        check!(command(ctl, "mode 800 600", Vec::new()).is_err(), "po obnove slo zmenit rezim");
        let caps_line = command(ctl, control::OUTPUTS, Vec::new())?;
        println!("init: test displej: po obnove: {}", caps_line.trim());
        s.kit.process = Some(p);
        Ok(())
    });
    // Uklid: kompozitor sady skonci, ovladac se vrati do stavu po startu
    // systemu (bezna instance, ne obnova) a zpet do seznamu, aby dalsi
    // kolo (`--repeat`) a dalsi sady nasly stejny ovladac.
    if let Some(mut st) = STATE.lock().take() {
        if let Some((p, ctl)) = st.compd.take() {
            drop(ctl);
            let _ = p.wait_exit(deadline());
        }
        if let Some(p) = st.kit.process.take() {
            let _ = p.kill();
            let _ = p.wait_exit(deadline());
        }
        match st.kit.spawn(startup, false) {
            Ok(p) => {
                st.kit.process = Some(p);
                NATIVES.lock().push(st.kit);
            }
            Err(e) => println!("init: test displej: ovladac {} nejde spustit znovu: {e}", st.kit.program),
        }
    }
    r
}
