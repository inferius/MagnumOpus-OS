//! Kody chyb spolecne pro jadro i userspace.
//!
//! Syscall vraci v `rax` nezaporne cislo pri uspechu a zaporny kod chyby
//! pri neuspechu. Cisla existujicich chyb se nikdy nemeni ani nerecykluji;
//! nove chyby dostavaji dalsi volne cislo.
//!
//! Kazda chyba je podrobne popsana (vyznam, kdy vznika, co s ni) v
//! `docs/cs/chyby.md` a `docs/en/errors.md`. Test v tomto crate overuje,
//! ze je tam kazdy kod zdokumentovany.

use core::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Error {
    /// Handle neexistuje nebo uz byl zavren.
    BadHandle = -1,
    /// Handle nema prava potrebna pro operaci.
    AccessDenied = -2,
    /// Objekt za handlem je jineho typu, nez operace ocekava.
    WrongType = -3,
    /// Vyprsel cas cekani.
    TimedOut = -4,
    /// Nedostatek pameti.
    NoMemory = -5,
    /// Vycerpany limit (pocet handlu, delka fronty, ...).
    NoResources = -6,
    /// Neplatny argument (hodnota mimo povoleny rozsah, nepovolena kombinace).
    InvalidArgs = -7,
    /// Pozadovana vec neexistuje.
    NotFound = -8,
    /// Operace neni podporovana.
    NotSupported = -9,
    /// Data jsou poskozena nebo neplatna.
    Corrupted = -10,
    /// Ukazatel do pameti procesu je neplatny (nenamapovano, spatna prava).
    BadAddress = -11,
    /// Operace by musela cekat; zkusit znovu po signalu objektu.
    ShouldWait = -12,
    /// Druha strana kanalu je zavrena.
    PeerClosed = -13,
    /// Buffer je pro vysledek prilis maly.
    BufferTooSmall = -14,
    /// Offset nebo rozsah lezi mimo objekt.
    OutOfRange = -15,
    /// Objekt je ve stavu, ve kterem operace nedava smysl.
    BadState = -16,
    /// Neznamy syscall.
    BadSyscall = -17,
    /// Operace byla prerusena ukoncovanim vlakna nebo procesu.
    Canceled = -18,
    /// Polozka se zadanym jmenem uz existuje.
    AlreadyExists = -19,
    /// Cast cesty neni adresar.
    NotADirectory = -20,
    /// Operace pro soubor dostala adresar.
    IsADirectory = -21,
    /// Adresar neni prazdny.
    NotEmpty = -22,
    /// Na zarizeni neni misto.
    NoSpace = -23,
    /// Chyba cteni nebo zapisu zarizeni.
    IoError = -24,
    /// Zapis do objektu jen pro cteni (napr. snapshot).
    ReadOnly = -25,
    /// Objekt je otevreny v nekompatibilnim sdilecim rezimu nebo je
    /// rozsah zamceny jinym handlem.
    Locked = -26,
    /// Objekt se mezitim zmenil (podminena operace s ocekavanou revizi).
    Conflict = -27,
    /// Pozadovane zaznamy zurnalu zmen uz nejsou ulozene (retence):
    /// odberatel musi nacist novy vychozi stav (`VIEW`).
    JournalGap = -28,
}

/// Vsechny chyby (pro dokumentaci a testy).
pub const ALL: &[Error] = &[
    Error::BadHandle,
    Error::AccessDenied,
    Error::WrongType,
    Error::TimedOut,
    Error::NoMemory,
    Error::NoResources,
    Error::InvalidArgs,
    Error::NotFound,
    Error::NotSupported,
    Error::Corrupted,
    Error::BadAddress,
    Error::ShouldWait,
    Error::PeerClosed,
    Error::BufferTooSmall,
    Error::OutOfRange,
    Error::BadState,
    Error::BadSyscall,
    Error::Canceled,
    Error::AlreadyExists,
    Error::NotADirectory,
    Error::IsADirectory,
    Error::NotEmpty,
    Error::NoSpace,
    Error::IoError,
    Error::ReadOnly,
    Error::Locked,
    Error::Conflict,
    Error::JournalGap,
];

impl Error {
    /// Stabilni ciselny kod (vzdy zaporny).
    pub const fn code(self) -> i32 {
        self as i32
    }

    /// Chyba podle kodu; `None` pro neznamy kod.
    pub fn from_code(code: i64) -> Option<Self> {
        ALL.iter().copied().find(|e| e.code() as i64 == code)
    }

    /// Kratky nazev (stejny v logach i v dokumentaci).
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
            Self::BadAddress => "BAD_ADDRESS",
            Self::ShouldWait => "SHOULD_WAIT",
            Self::PeerClosed => "PEER_CLOSED",
            Self::BufferTooSmall => "BUFFER_TOO_SMALL",
            Self::OutOfRange => "OUT_OF_RANGE",
            Self::BadState => "BAD_STATE",
            Self::BadSyscall => "BAD_SYSCALL",
            Self::Canceled => "CANCELED",
            Self::AlreadyExists => "ALREADY_EXISTS",
            Self::NotADirectory => "NOT_A_DIRECTORY",
            Self::IsADirectory => "IS_A_DIRECTORY",
            Self::NotEmpty => "NOT_EMPTY",
            Self::NoSpace => "NO_SPACE",
            Self::IoError => "IO_ERROR",
            Self::ReadOnly => "READ_ONLY",
            Self::Locked => "LOCKED",
            Self::Conflict => "CONFLICT",
            Self::JournalGap => "JOURNAL_GAP",
        }
    }

    /// Jednovetny popis pro logy a chybova hlaseni.
    pub const fn description(self) -> &'static str {
        match self {
            Self::BadHandle => "handle neexistuje nebo uz byl zavren",
            Self::AccessDenied => "handle nema potrebna prava",
            Self::WrongType => "objekt je jineho typu, nez operace ocekava",
            Self::TimedOut => "vyprsel cas cekani",
            Self::NoMemory => "nedostatek pameti",
            Self::NoResources => "vycerpany limit (handly, fronta, ...)",
            Self::InvalidArgs => "neplatny argument",
            Self::NotFound => "pozadovana vec neexistuje",
            Self::NotSupported => "operace neni podporovana",
            Self::Corrupted => "data jsou poskozena nebo neplatna",
            Self::BadAddress => "neplatny ukazatel do pameti procesu",
            Self::ShouldWait => "operace by musela cekat",
            Self::PeerClosed => "druha strana kanalu je zavrena",
            Self::BufferTooSmall => "buffer je prilis maly",
            Self::OutOfRange => "offset nebo rozsah mimo objekt",
            Self::BadState => "objekt je ve stavu, kdy operace nedava smysl",
            Self::BadSyscall => "neznamy syscall",
            Self::Canceled => "operace prerusena ukoncovanim vlakna nebo procesu",
            Self::AlreadyExists => "polozka uz existuje",
            Self::NotADirectory => "cast cesty neni adresar",
            Self::IsADirectory => "je to adresar",
            Self::NotEmpty => "adresar neni prazdny",
            Self::NoSpace => "na zarizeni neni misto",
            Self::IoError => "chyba cteni nebo zapisu zarizeni",
            Self::ReadOnly => "jen pro cteni",
            Self::Locked => "objekt je otevreny nebo zamceny jinym handlem",
            Self::Conflict => "objekt se mezitim zmenil",
            Self::JournalGap => "pozadovane zaznamy zurnalu uz nejsou ulozene",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({}): {}",
            self.name(),
            self.code(),
            self.description()
        )
    }
}

pub type Result<T, E = Error> = core::result::Result<T, E>;

/// Zakoduje vysledek syscallu do registru `rax`.
pub fn encode(result: Result<u64>) -> u64 {
    match result {
        Ok(v) => {
            debug_assert!((v as i64) >= 0, "uspesna hodnota nesmi byt zaporna");
            v
        }
        Err(e) => e.code() as i64 as u64,
    }
}

/// Dekoduje navratovou hodnotu syscallu.
pub fn decode(raw: u64) -> Result<u64> {
    let v = raw as i64;
    if v >= 0 {
        Ok(raw)
    } else {
        Err(Error::from_code(v).unwrap_or(Error::BadSyscall))
    }
}
