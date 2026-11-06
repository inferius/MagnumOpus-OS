//! Vystup kernelu: seriova linka a (pokud je) framebuffer.

use core::fmt::{self, Write};

use boot_info::Framebuffer;
use spin::Mutex;

use crate::arch::{self, SerialPort};
use crate::framebuffer::FbConsole;

struct Console {
    serial: SerialPort,
    fb: Option<FbConsole>,
}

static CONSOLE: Mutex<Option<Console>> = Mutex::new(None);

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

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    if let Some(console) = CONSOLE.lock().as_mut() {
        let _ = console.serial.write_fmt(args);
        if let Some(fb) = console.fb.as_mut() {
            let _ = fb.write_fmt(args);
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

#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => ($crate::console::_print(format_args!($($arg)*)));
}

#[macro_export]
macro_rules! kprintln {
    () => ($crate::kprint!("\n"));
    ($($arg:tt)*) => ($crate::console::_print(format_args!("{}\n", format_args!($($arg)*))));
}
