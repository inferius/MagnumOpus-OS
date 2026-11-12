//! Startovni obrazovka: rizeni procesu `splash` (kresli `user/splash`).
//!
//! Loader vykreslil prvni snimek, jadro do framebufferu nekreslilo
//! (`bootstrap::flags::SPLASH`). Init spusti proces `splash` z initfs
//! (framebuffer a ridici kanal), hlasi mu kroky startu ([`step`]) a pred
//! spustenim displeje kompozitoru (nebo zalozni konzole) ho ukonci
//! ([`finish`]): proces dokresli plny prstenec, odpovi a skonci;
//! framebuffer zustava bez smazani pro dalsiho vlastnika. Kdyz start
//! selze, [`fail`] mu rekne, at ukaze obrazovku chyby.
//!
//! Kresleni je zamerne mimo init: chyba v nem shodi jen proces `splash`
//! (init ho ukonci a start pokracuje), ne cely system.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Process, Startup, Vmo, println, time};

/// Bezici proces a ridici kanal.
static SPLASH: spin::Mutex<Option<(Process, Channel)>> = spin::Mutex::new(None);
/// `konec` uz odeslany ([`finish_begin`]).
static ENDING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Jak dlouho cekat na odpoved (konec trva ~200 ms dobehu prstence).
const REPLY_TIMEOUT: Duration = Duration::from_millis(1500);

/// Spusti animaci, kdyz loader vykreslil startovni obraz (bezny beh, ne
/// testy ani vytazeni logu).
pub fn start(startup: &Startup) {
    use libos::abi::bootstrap::flags;
    if startup.flags & flags::SPLASH == 0 || startup.flags & (flags::TEST_RUN | flags::RESCUE_LOGS) != 0 {
        return;
    }
    let Some(fb) = startup.framebuffer.as_ref() else { return };
    let r = (|| -> libos::Result<(Process, Channel)> {
        let fs = Initfs::open(startup)?;
        let image = fs.find("splash").ok_or(libos::Error::NotFound)?;
        let fb = Vmo::from_handle(fb.handle().duplicate(None)?);
        let (mine, theirs) = Channel::create()?;
        let (process, _boot) = libos::spawn(
            "splash",
            image,
            vec![StartHandle::user(fb.into_handle()), StartHandle::user(theirs.into_handle())],
        )?;
        let _ = libos::sched::set_process(&process, libos::sched::Qos::Background);
        Ok((process, mine))
    })();
    match r {
        Ok(s) => *SPLASH.lock() = Some(s),
        Err(e) => println!("init: startovni obraz nelze spustit: {e}"),
    }
}

/// Hotovo `n` kroku z `bootsplash::STEPS` (2 = zarizeni, 3 = soubory,
/// 4 = sit; 1 = jadro je hotove startem initu).
pub fn step(n: usize) {
    if let Some((_, ch)) = SPLASH.lock().as_ref() {
        let _ = ch.write(alloc::format!("krok {n}").as_bytes(), Vec::new());
    }
}

/// Odpoved procesu (text), `None` = neprisla vcas nebo kanal zavreny.
fn reply(ch: &Channel) -> Option<String> {
    let m = ch.read(Some(time::deadline_after(REPLY_TIMEOUT))).ok()?;
    Some(String::from_utf8_lossy(&m.data).into_owned())
}

/// Konec procesu: pocka na nej, jinak ho ukonci (obraz zustava, jak je).
fn end(p: &Process) {
    if p.wait_exit(Some(time::deadline_after(Duration::from_millis(500)))).is_err() {
        println!("init: proces startovniho obrazu neskoncil, ukoncuji ho");
        let _ = p.kill();
        let _ = p.wait_exit(Some(time::deadline_after(Duration::from_millis(500))));
    }
}

/// Zacne konec predem (prstenec dobiha do plna, ~200 ms), aby [`finish`]
/// pred displejem uz necekal.
pub fn finish_begin() {
    if let Some((_, ch)) = SPLASH.lock().as_ref()
        && !ENDING.swap(true, core::sync::atomic::Ordering::Relaxed)
    {
        let _ = ch.write(b"konec", Vec::new());
    }
}

/// Ukonci animaci (plny prstenec) a vrati framebuffer bez smazani.
/// Vraci prstenec pro kompozitor (promile), `None` = obraz nebezel nebo
/// proces neodpovedel. Dalsi volani nic nedela.
pub fn finish() -> Option<u32> {
    let (p, ch) = SPLASH.lock().take()?;
    let t = time::now();
    let sent = ENDING.swap(true, core::sync::atomic::Ordering::Relaxed) || ch.write(b"konec", Vec::new()).is_ok();
    let r = if sent { reply(&ch) } else { None };
    end(&p);
    let permille = r.as_deref().and_then(|t| t.strip_prefix("hotovo ")).and_then(|v| v.trim().parse().ok());
    println!("init: startovni obraz ukoncen za {} ms ({permille:?})", (time::now() - t) / 1_000_000);
    permille
}

/// Start selhal: obrazovka chyby s duvodem (kresli proces `splash`, kdyz
/// jeste bezi). Framebuffer pak nikdo neprevezme.
pub fn fail(reason: &str) {
    let Some((p, ch)) = SPLASH.lock().take() else {
        println!("init: start selhal ({reason}), startovni obraz nebezi");
        return;
    };
    let _ = ch.write(alloc::format!("chyba {reason}").as_bytes(), Vec::new());
    let _ = reply(&ch);
    end(&p);
}
