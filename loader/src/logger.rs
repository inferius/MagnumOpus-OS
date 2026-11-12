//! Vypis loaderu (`log`): do textove konzole firmwaru, a od chvile, kdy
//! je na obrazovce startovni obraz, uz jen na seriovou linku (protokol
//! UEFI Serial I/O). Textova konzole firmwaru by jinak psala pres obraz.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use uefi::boot::{self, OpenProtocolAttributes, OpenProtocolParams};
use uefi::proto::console::serial::Serial;

struct Logger;

static LOGGER: Logger = Logger;
/// Obrazovku drzi startovni obraz: text jen na seriovou linku.
static SERIAL_ONLY: AtomicBool = AtomicBool::new(false);
/// Boot services skoncily: uz nic.
static OFF: AtomicBool = AtomicBool::new(false);

pub fn init() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Od ted nepsat do textove konzole firmwaru.
pub fn serial_only() {
    SERIAL_ONLY.store(true, Ordering::Relaxed);
}

/// Pred `ExitBootServices`.
pub fn off() {
    OFF.store(true, Ordering::Relaxed);
}

/// Zapis na prvni seriovou linku firmwaru (kdyz zadna neni, zahodi se).
fn write_serial(text: &str) {
    let Ok(handle) = boot::get_handle_for_protocol::<Serial>() else { return };
    let params = OpenProtocolParams { handle, agent: boot::image_handle(), controller: None };
    // SAFETY: GetProtocol neodpoji ostatni uzivatele linky (textovou
    // konzoli firmwaru), jen zapisujeme a protokol hned zavreme.
    let serial = unsafe { boot::open_protocol::<Serial>(params, OpenProtocolAttributes::GetProtocol) };
    if let Ok(mut s) = serial {
        let _ = s.write_exact(text.as_bytes());
    }
}

impl log::Log for Logger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, r: &log::Record) {
        if OFF.load(Ordering::Relaxed) || !self.enabled(r.metadata()) {
            return;
        }
        let mut line = alloc::string::String::new();
        let _ = write!(line, "[{:>5}]: loader: {}\r\n", r.level(), r.args());
        if SERIAL_ONLY.load(Ordering::Relaxed) {
            write_serial(&line);
        } else {
            uefi::system::with_stdout(|o| {
                let _ = o.write_str(&line);
            });
        }
    }

    fn flush(&self) {}
}
