//! Vystup kernelu: seriova linka a (pokud je) framebuffer.

use core::fmt::{self, Write};

use crate::sync::SpinLock;
use boot_info::Framebuffer;

use crate::arch::{self, SerialPort};
use crate::framebuffer::FbConsole;

struct Console {
    serial: SerialPort,
    fb: Option<FbConsole>,
    /// Popis framebufferu z firmware (pro objekt pameti pro userspace).
    desc: Option<Framebuffer>,
    /// Framebuffer prevzal userspace (`FB_CONTROL` `CLAIM`): id procesu;
    /// log jen na seriovou linku.
    claimed: Option<u64>,
    /// Startovni obraz z loaderu (`boot_flags::SPLASH`): jadro do
    /// framebufferu nekresli a nemaze ho (log jen na seriovou linku
    /// a do bufferu logu), dokud start neselze ([`leave_splash`]).
    splash: bool,
}

static CONSOLE: SpinLock<Option<Console>> = SpinLock::new(None);

/// Inicializuje konzoli. Volat jednou, co nejdrive po startu.
///
/// # Safety
///
/// `fb`, pokud je pritomny, musi popisovat platny namapovany framebuffer,
/// do ktereho nikdo jiny nezapisuje.
pub unsafe fn init(fb: &Framebuffer, splash: bool) {
    let mut serial = arch::debug_serial();
    arch::init_debug_serial(&mut serial);
    let splash = splash && fb.is_present();
    let desc = fb.is_present().then_some(*fb);
    // Se startovnim obrazem se textova konzole nezaklada (smazala by ho).
    // SAFETY: predano od volajiciho.
    let fb = (fb.is_present() && !splash).then(|| unsafe { FbConsole::new(fb) });
    *CONSOLE.lock() = Some(Console { serial, fb, desc, claimed: None, splash });
}

/// Je na obrazovce startovni obraz (jadro do framebufferu nekresli)?
pub fn splash() -> bool {
    CONSOLE.lock().as_ref().is_some_and(|c| c.splash)
}

/// Start selhal (init skoncil): konec startovniho obrazu, obrazovka se
/// smaze a jadro na ni dal pise log (jinak by obraz "zamrzl"). Kdyz
/// framebuffer drzi proces, nic se nemeni (obraz je jeho).
pub fn leave_splash() {
    let mut g = CONSOLE.lock();
    let Some(c) = g.as_mut() else { return };
    if !c.splash {
        return;
    }
    c.splash = false;
    if c.claimed.is_none()
        && let Some(desc) = c.desc
    {
        // SAFETY: framebuffer popsal loader, je namapovany po celou dobu
        // behu a nikdo jiny ho nedrzi (claimed je None).
        c.fb = Some(unsafe { FbConsole::new(&desc) });
    }
}

/// Zapise text na seriovou linku a pripadne i do framebufferu.
/// Pred `init` se text tise zahodi (zustane jen v bufferu logu).
pub fn write(text: &str, to_framebuffer: bool) {
    if let Some(console) = CONSOLE.lock().as_mut() {
        let _ = console.serial.write_str(text);
        if to_framebuffer && console.claimed.is_none() && let Some(fb) = console.fb.as_mut() {
            let _ = fb.write_str(text);
        }
    }
}

/// Framebuffer z firmware, kdyz je.
pub fn framebuffer() -> Option<Framebuffer> {
    CONSOLE.lock().as_ref().and_then(|c| c.desc)
}

/// Prevzeti framebufferu procesem `pid` (`FB_CONTROL` `CLAIM`); `false`
/// = drzi ho jiny proces (atomicky: kontrola i zapis pod jednim zamkem).
pub fn try_claim(pid: u64) -> bool {
    let mut g = CONSOLE.lock();
    let Some(c) = g.as_mut() else { return false };
    if c.claimed.is_some_and(|o| o != pid) {
        return false;
    }
    c.claimed = Some(pid);
    true
}

/// Proces skoncil (nebo vraci framebuffer): kdyz ho drzel, vrati se jadru
/// (pad konzole nenecha obrazovku zamrzlou). Vrati, jestli ho drzel. Vola
/// se mimo zamky procesu.
pub fn process_finished(pid: u64) -> bool {
    let mut g = CONSOLE.lock();
    let Some(c) = g.as_mut() else { return false };
    if c.claimed != Some(pid) {
        return false;
    }
    // Se startovnim obrazem zustane posledni snimek vlastnika (predani
    // dalsimu bez bliknuti); jinak se obrazovka smaze a pise se log.
    if !c.splash
        && let Some(fb) = c.fb.as_mut()
    {
        fb.clear();
    }
    c.claimed = None;
    true
}

/// Vystup z panic handleru: nesmi cekat na zamek (panika mohla nastat
/// uvnitr `_print`), proto pri obsazenem zamku pise primo na seriovou linku.
pub fn panic_print(args: fmt::Arguments) {
    match CONSOLE.try_lock() {
        Some(mut guard) if guard.is_some() => {
            let console = guard.as_mut().unwrap();
            let _ = console.serial.write_fmt(args);
            if let Some(fb) = console.fb.as_mut() {
                let _ = fb.write_fmt(args);
            }
        }
        _ => {
            let _ = arch::debug_serial().write_fmt(args);
        }
    }
}
