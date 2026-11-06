//! Chyby jadra.
//!
//! `Error` je spolecny typ jadra i userspace (crate `abi`): maly kod se
//! stabilni ciselnou hodnotou, ktera je zaroven navratovym kodem syscallu.
//! Vyznam kazdeho kodu popisuje `docs/cs/chyby.md` / `docs/en/errors.md`.
//! Kontext (co se delalo a kde) se k chybe nepridava do typu, ale zaloguje
//! se v miste vzniku pres [`ResultExt::context`], kde je nejvic informaci.

use core::fmt;

pub use abi::Error;

pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Rozsireni `Result` o zalogovani chyby s kontextem a mistem volani.
pub trait ResultExt<T, E> {
    /// Pri chybe zaloguje (uroven Debug) "co se delalo" a misto volani,
    /// chybu vrati beze zmeny. Pro chyby, ktere se ocekavaji, ale je dobre
    /// je umet dohledat.
    #[allow(dead_code, reason = "API pro cele jadro")]
    fn context(self, what: &str) -> Self;

    /// Jako `context`, ale uroven Warn - pro chyby, ktere by se nemely stat.
    fn warn_context(self, what: &str) -> Self;
}

impl<T, E: fmt::Display> ResultExt<T, E> for core::result::Result<T, E> {
    #[track_caller]
    fn context(self, what: &str) -> Self {
        if let Err(e) = &self {
            let loc = core::panic::Location::caller();
            crate::log::debug!(
                "{what}: {e} ({}:{})",
                crate::log::SourcePath(loc.file()),
                loc.line()
            );
        }
        self
    }

    #[track_caller]
    fn warn_context(self, what: &str) -> Self {
        if let Err(e) = &self {
            let loc = core::panic::Location::caller();
            crate::log::warn!(
                "{what}: {e} (volano z {}:{})",
                crate::log::SourcePath(loc.file()),
                loc.line()
            );
        }
        self
    }
}
