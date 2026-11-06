//! Vystup kernelu: seriova linka a (pokud je) framebuffer.

use core::fmt::{self, Write};

use crate::sync::SpinLock;
use boot_info::Framebuffer;

use crate::arch::{self, SerialPort};
use crate::framebuffer::FbConsole;

struct Console {
    serial: SerialPort,
    fb: Option<FbConsole>,
}

static CONSOLE: SpinLock<Option<Console>> = SpinLock::new(None);

/// Inicializuje konzoli. Volat jednou, co nejdrive po startu.
///
/// # Safety
///
/// `fb`, pokud je pritomny, musi popisovat platny namapovany framebuffer,
/// do ktereho nikdo jiny nezapisuje.
pub unsafe fn init(fb: &Framebuffer) {
    let mut serial = arch::debug_serial();
    serial.init();
    // SAFETY: predano od volajiciho.
    let fb = fb.is_present().then(|| unsafe { FbConsole::new(fb) });
    *CONSOLE.lock() = Some(Console { serial, fb });
}

/// Zapise text na seriovou linku a pripadne i do framebufferu.
/// Pred `init` se text tise zahodi (zustane jen v bufferu logu).
pub fn write(text: &str, to_framebuffer: bool) {
    if let Some(console) = CONSOLE.lock().as_mut() {
        let _ = console.serial.write_str(text);
        if to_framebuffer && let Some(fb) = console.fb.as_mut() {
            let _ = fb.write_str(text);
        }
    }
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
