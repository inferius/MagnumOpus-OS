//! Spolecny typ chyb jadra.
//!
//! `Error` je maly kopirovatelny kod se stabilni ciselnou hodnotou - stane se
//! z nej navratovy kod syscallu, takze se nemeni cisla existujicich variant.
//! Kontext (co se delalo a kde) se k chybe nepridava do typu, ale zaloguje
//! se v miste vzniku pres [`ResultExt::context`], kde je nejvic informaci.

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Error {
    /// Handle neexistuje (nebo uz byl zavren).
    BadHandle = -1,
    /// Handle nema potrebna prava.
    AccessDenied = -2,
    /// Objekt je jineho typu, nez operace ocekava.
    WrongType = -3,
    /// Vyprsel cas cekani.
    TimedOut = -4,
    /// Nedostatek pameti.
    NoMemory = -5,
    /// Vycerpany limit (napr. pocet handlu).
    NoResources = -6,
    /// Neplatne argumenty.
    #[allow(dead_code, reason = "kod ABI syscallu (milnik 4)")]
    InvalidArgs = -7,
    /// Pozadovana vec neexistuje.
    NotFound = -8,
    /// Operace neni podporovana.
    #[allow(dead_code, reason = "kod ABI syscallu (milnik 4)")]
    NotSupported = -9,
    /// Data jsou poskozena nebo neplatna (napr. tabulka firmwaru).
    Corrupted = -10,
}

impl Error {
    /// Stabilni ciselny kod (pro syscally a logy).
    pub const fn code(self) -> i32 {
        self as i32
    }

    /// Kratky nazev pro logy.
    pub const fn name(self) -> &'static str {
        match self {
            Self::BadHandle => "BAD_HANDLE",
            Self::AccessDenied => "ACCESS_DENIED",
            Self::WrongType => "WRONG_TYPE",
            Self::TimedOut => "TIMED_OUT",
            Self::NoMemory => "NO_MEMORY",
            Self::NoResources => "NO_RESOURCES",
            Self::InvalidArgs => "INVALID_ARGS",
            Self::NotFound => "NOT_FOUND",
            Self::NotSupported => "NOT_SUPPORTED",
            Self::Corrupted => "CORRUPTED",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name(), self.code())
    }
}

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
