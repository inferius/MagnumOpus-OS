//! Tridy QoS: vahy, kvanta, prevody z ABI.

use abi::sched::class as raw;

/// Trida kvality sluzby. Mensi cislo = vyssi trida.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Class {
    Realtime = raw::REALTIME,
    Interactive = raw::INTERACTIVE,
    Normal = raw::NORMAL,
    Background = raw::BACKGROUND,
}

/// Pocet spravedlivych trid (bez Realtime).
pub const FAIR_CLASSES: usize = 3;

/// Spravedlive tridy v poradi indexu [`Class::fair_index`].
pub const FAIR: [Class; FAIR_CLASSES] = [Class::Interactive, Class::Normal, Class::Background];

/// Vahy spravedlivych trid: plne vytizene vsechny tri dostanou 75 %, 20 %
/// a 5 % casu CPU mimo Realtime.
pub const WEIGHTS: [u64; FAIR_CLASSES] = [15, 4, 1];

/// Spolecny nasobek vah: posun virtualniho casu tridy za 1 ns behu je
/// `WEIGHT_LCM / vaha`, takze vypocet je celociselny a presny.
pub const WEIGHT_LCM: u64 = 60;

/// Nejmensi beh bezici vlakna, nez ho smi probuzene vlakno stejne tridy
/// predbehnout (proti prehazovani dvou vlaken po mikrosekundach).
pub const MIN_GRANULARITY_NS: u64 = 500_000;

/// Jak dlouho plati zvednuti tridy zpravou (dedeni pres IPC).
pub const BOOST_WINDOW_NS: u64 = 50_000_000;

/// Nejvys podil jednoho CPU pro Realtime (v promile).
pub const RT_MAX_PERMILLE: u64 = 500;

impl Class {
    pub fn from_raw(v: u8) -> Option<Self> {
        Some(match v {
            raw::REALTIME => Self::Realtime,
            raw::INTERACTIVE => Self::Interactive,
            raw::NORMAL => Self::Normal,
            raw::BACKGROUND => Self::Background,
            _ => return None,
        })
    }

    /// Vaha tridy pro vyvazovani zateze mezi CPU (Realtime jako Interactive).
    pub fn load_weight(self) -> u64 {
        match self.fair_index() {
            Some(c) => WEIGHTS[c],
            None => WEIGHTS[0],
        }
    }

    /// Index ve spravedlivych tridach; Realtime nema.
    pub fn fair_index(self) -> Option<usize> {
        (self as usize).checked_sub(1)
    }

    /// Delka kvanta (pozadavku EEVDF) uvnitr tridy. Realtime bezi do
    /// vycerpani rozpoctu, tady jen pro jeho beh jako Interactive.
    pub fn slice_ns(self) -> u64 {
        match self {
            Self::Realtime | Self::Interactive => 3_000_000,
            Self::Normal => 6_000_000,
            Self::Background => 12_000_000,
        }
    }

    /// Vyssi ze dvou trid.
    pub fn max(self, other: Self) -> Self {
        if (other as u8) < (self as u8) { other } else { self }
    }

    /// Nizsi ze dvou trid (strop).
    pub fn min(self, other: Self) -> Self {
        if (other as u8) > (self as u8) { other } else { self }
    }

    /// Je `self` vyssi trida nez `other`?
    pub fn above(self, other: Self) -> bool {
        (self as u8) < (other as u8)
    }
}

/// Garantovany podil Background na vytizenem CPU (mimo Realtime), v promile.
pub const fn background_floor_permille() -> u64 {
    WEIGHTS[2] * 1000 / (WEIGHTS[0] + WEIGHTS[1] + WEIGHTS[2])
}
