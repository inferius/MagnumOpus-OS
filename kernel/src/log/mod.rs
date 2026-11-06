//! Logovani jadra.
//!
//! Kazdy zaznam nese cas od startu, CPU, uroven, cil (modul, ze ktereho
//! pochazi) a u chyb a varovani i misto ve zdrojaku:
//!
//! ```text
//! [    0.012345] cpu0 I sched: zprava
//! [    0.012400] cpu1 W acpi: tabulka ... (kernel/src/acpi.rs:120)
//! ```
//!
//! Zaznamy jdou do:
//! - seriove linky (vse povolene),
//! - framebufferu (jen do urovne `console_level`, standardne Info - zapis do
//!   framebufferu je pomaly),
//! - kruhoveho bufferu v pameti (posledni zaznamy pro pozdejsi cteni, napr.
//!   sluzbou logu v userspace nebo pri ladeni).
//!
//! Filtrovani:
//! - pri prekladu: `STATIC_MAX_LEVEL` (release bez Trace, s feature
//!   `log-trace` vse) - zakazane urovne se vubec neprelozi,
//! - za behu: globalni uroven a prepsani pro jednotlive moduly
//!   (`set_target_level("sched", Level::Trace)`).
//!
//! Zakazana uroven stoji jedno atomicke cteni, zadne formatovani.
//! Logovani nealokuje, takze funguje i v obsluze preruseni a pred
//! inicializaci heapu.

mod filter;
mod line;
mod ring;

use core::fmt;

#[allow(
    unused_imports,
    reason = "API pro nastaveni logu (prikazova radka jadra, syscally)"
)]
pub use filter::{
    clear_target_level, console_level, set_console_level, set_level, set_target_level,
};
pub use ring::snapshot;

use crate::{arch, console, sched};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl Level {
    const fn letter(self) -> char {
        match self {
            Self::Error => 'E',
            Self::Warn => 'W',
            Self::Info => 'I',
            Self::Debug => 'D',
            Self::Trace => 'T',
        }
    }

    const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Error),
            2 => Some(Self::Warn),
            3 => Some(Self::Info),
            4 => Some(Self::Debug),
            5 => Some(Self::Trace),
            _ => None,
        }
    }
}

/// Nejvyssi uroven, ktera se vubec prelozi.
pub const STATIC_MAX_LEVEL: Level = if cfg!(feature = "log-trace") || cfg!(debug_assertions) {
    Level::Trace
} else {
    Level::Debug
};

/// Cesta ke zdrojaku s jednotnym oddelovacem `/` (pri prekladu na Windows
/// dava `file!()` zpetna lomitka).
pub struct SourcePath<'a>(pub &'a str);

impl fmt::Display for SourcePath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, part) in self.0.split('\\').enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            f.write_str(part)?;
        }
        Ok(())
    }
}

/// Vstup maker. Nevolat primo.
#[doc(hidden)]
pub fn __log(
    level: Level,
    target: &'static str,
    file: &'static str,
    line: u32,
    args: fmt::Arguments,
) {
    let target = target.strip_prefix("kernel::").unwrap_or(target);
    if !filter::enabled(level, target) {
        return;
    }
    let mut buf = line::LineBuf::new();
    let ns = arch::now_ns();
    let _ = fmt::write(
        &mut buf,
        format_args!(
            "[{:5}.{:06}] cpu{} {} {}: ",
            ns / 1_000_000_000,
            (ns / 1_000) % 1_000_000,
            sched::try_current_cpu_index().unwrap_or(0),
            level.letter(),
            target
        ),
    );
    let _ = fmt::write(&mut buf, args);
    if level <= Level::Warn {
        let _ = fmt::write(&mut buf, format_args!(" ({}:{line})", SourcePath(file)));
    }
    buf.finish_line();

    ring::push(buf.as_bytes());
    console::write(buf.as_str(), level <= filter::console_level());
}

macro_rules! log_at {
    ($level:expr, $($arg:tt)+) => {
        if $level <= $crate::log::STATIC_MAX_LEVEL {
            $crate::log::__log($level, module_path!(), file!(), line!(), format_args!($($arg)+));
        }
    };
}

macro_rules! error {
    ($($arg:tt)+) => { $crate::log::log_at!($crate::log::Level::Error, $($arg)+) };
}

macro_rules! warn_ {
    ($($arg:tt)+) => { $crate::log::log_at!($crate::log::Level::Warn, $($arg)+) };
}

macro_rules! info {
    ($($arg:tt)+) => { $crate::log::log_at!($crate::log::Level::Info, $($arg)+) };
}

macro_rules! debug {
    ($($arg:tt)+) => { $crate::log::log_at!($crate::log::Level::Debug, $($arg)+) };
}

#[allow(unused_macros, reason = "API pro cele jadro")]
macro_rules! trace {
    ($($arg:tt)+) => { $crate::log::log_at!($crate::log::Level::Trace, $($arg)+) };
}

#[allow(
    unused_imports,
    reason = "makra jsou API pro cele jadro, ne vsechna se uz pouzivaji"
)]
pub(crate) use {debug, error, info, log_at, trace};
// Makro se nemuze jmenovat primo `warn` (kolize s vestavenym atributem
// `#[warn]` pri importu), proto alias.
pub(crate) use warn_ as warn;

/// Vypis pri fatalni chybe: neceka na zamky (zapis jde pres
/// `console::panic_print`), ale zaznam se ulozi i do bufferu logu.
pub fn emergency(args: fmt::Arguments) {
    let mut buf = line::LineBuf::new();
    let _ = fmt::write(&mut buf, args);
    buf.finish_line();
    ring::push(buf.as_bytes());
    console::panic_print(format_args!("{}", buf.as_str()));
}
