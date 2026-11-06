//! Handly, prava, signaly a typy objektu.

use core::ops::{BitAnd, BitOr, Not};

/// Handle: 32bitova hodnota platna jen v tabulce jednoho procesu.
/// Dolnich 20 bitu = index + 1, hornich 12 bitu = generace slotu.
/// Hodnota 0 neni nikdy platny handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Handle(pub u32);

impl Handle {
    pub const INVALID: Self = Self(0);

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// Prava handlu. Pri duplikaci a predani lze prava jen ubirat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Rights(pub u32);

impl Rights {
    pub const NONE: Self = Self(0);
    /// Cist data (kanal, objekt pameti).
    pub const READ: Self = Self(1 << 0);
    /// Zapisovat data (kanal, objekt pameti, adresni prostor procesu).
    pub const WRITE: Self = Self(1 << 1);
    /// Mapovat jako spustitelne.
    pub const EXECUTE: Self = Self(1 << 2);
    /// Mapovat objekt pameti do adresniho prostoru.
    pub const MAP: Self = Self(1 << 3);
    /// Vytvorit dalsi handle na stejny objekt.
    pub const DUPLICATE: Self = Self(1 << 4);
    /// Predat handle jinemu procesu (kanalem).
    pub const TRANSFER: Self = Self(1 << 5);
    /// Cekat na signaly objektu.
    pub const WAIT: Self = Self(1 << 6);
    /// Menit signaly objektu (event).
    pub const SIGNAL: Self = Self(1 << 7);
    /// Ridit objekt: spoustet a ukoncovat vlakna a procesy.
    pub const MANAGE: Self = Self(1 << 8);

    pub const BASIC: Self = Self(Self::DUPLICATE.0 | Self::TRANSFER.0 | Self::WAIT.0);
    /// Vsechna prava (jen jako horni mez, napr. "zachovat prava" pri duplikaci).
    pub const ALL: Self = Self((1 << 9) - 1);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Rights {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for Rights {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/// Bitova maska stavu objektu, na ktere lze cekat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[repr(transparent)]
pub struct Signals(pub u32);

impl Signals {
    pub const NONE: Self = Self(0);
    /// Event je signalizovany.
    pub const SIGNALED: Self = Self(1 << 0);
    /// Vlakno nebo proces skoncil.
    pub const TERMINATED: Self = Self(1 << 1);
    /// Kanal: ve fronte je alespon jedna zprava.
    pub const READABLE: Self = Self(1 << 2);
    /// Kanal: druha strana je zavrena.
    pub const PEER_CLOSED: Self = Self(1 << 3);

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Signals {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for Signals {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl Not for Signals {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ObjectType {
    Event = 1,
    Thread = 2,
    Process = 3,
    Channel = 4,
    MemoryObject = 5,
}

/// Polozka pro `OBJECT_WAIT_MANY`: vstupem handle a maska, vystupem
/// signaly objektu v okamziku probuzeni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct WaitItem {
    pub handle: u32,
    pub mask: u32,
    pub observed: u32,
}
