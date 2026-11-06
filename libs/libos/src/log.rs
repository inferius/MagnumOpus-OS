//! Vystup do logu jadra (do vzniku sluzby logu).

use alloc::string::String;
use core::fmt;

#[doc(hidden)]
pub fn _print(args: fmt::Arguments) {
    let mut s = String::new();
    let _ = fmt::write(&mut s, args);
    crate::sys::debug_log(&s);
}

/// Zapise radek do logu jadra (s prefixem jmena procesu).
#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => { $crate::log::_print(format_args!($($arg)*)) };
}
