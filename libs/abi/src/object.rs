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
    /// Menit signaly objektu (event) nebo posouvat plot (`FENCE_SIGNAL`).
    pub const SIGNAL: Self = Self(1 << 7);
    /// Ridit objekt: spoustet a ukoncovat vlakna a procesy.
    pub const MANAGE: Self = Self(1 << 8);
    /// Nastavovat realny cas (`CLOCK_SET`). Ma ho jen korenovy Resource;
    /// init ho preda jen sluzbe casu (prostredky ovladacu ho nemaji).
    pub const SET_CLOCK: Self = Self(1 << 9);
    /// Vypnout nebo restartovat pocitac pres firmware (`SYSTEM_POWER`). Ma
    /// ho jen korenovy Resource; init ho preda jen sluzbe ACPI.
    pub const POWER: Self = Self(1 << 10);
    /// Diagnostika: cist log jadra (`LOG_READ`) a zaznam padu
    /// (`CRASH_RECORD`). Ma ho jen korenovy Resource; init ho preda jen
    /// sluzbe logu.
    pub const DIAG: Self = Self(1 << 11);
    /// Zpristupnit objekt pameti zarizeni (`DMA_MAP`). Odesilatel bufferu
    /// tak rozhoduje, jestli ho smi cist nebo zapisovat zarizeni; ma ho
    /// vychozi handle objektu pameti.
    pub const DMA: Self = Self(1 << 12);

    pub const BASIC: Self = Self(Self::DUPLICATE.0 | Self::TRANSFER.0 | Self::WAIT.0);
    /// Vsechna prava (jen jako horni mez, napr. "zachovat prava" pri duplikaci).
    pub const ALL: Self = Self((1 << 13) - 1);

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
    /// Vlakno je pozastavene (`THREAD_SUSPEND`): stoji pred navratem do
    /// userspace nebo spi v syscallu; jeho kontext jde cist a zapsat.
    pub const SUSPENDED: Self = Self(1 << 4);
    /// Event: uzivatelske bity (vyznam urcuje sluzba, ktera event vydala;
    /// napr. pripravenost socketu u `netd`). Meni je `OBJECT_SIGNAL`
    /// s pravem `SIGNAL`, cekat na ne jde jako na ostatni signaly.
    pub const USER_0: Self = Self(1 << 24);
    pub const USER_1: Self = Self(1 << 25);
    pub const USER_2: Self = Self(1 << 26);
    pub const USER_3: Self = Self(1 << 27);
    pub const USER_4: Self = Self(1 << 28);
    pub const USER_5: Self = Self(1 << 29);
    pub const USER_6: Self = Self(1 << 30);
    pub const USER_7: Self = Self(1 << 31);
    /// Vsechny uzivatelske bity.
    pub const USER_ALL: Self = Self(0xFF00_0000);

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
    Resource = 6,
    Interrupt = 7,
    Port = 8,
    Job = 9,
    /// Token pozastaveni vlakna (`THREAD_SUSPEND`); zavreni = obnoveni.
    SuspendToken = 10,
    /// Token vyjimky (zprava kanalu vyjimek); `EXCEPTION_RESOLVE`.
    ExceptionToken = 11,
    /// Plot: casova osa `u64` (`FENCE_*`, `abi::fence`).
    Fence = 12,
    /// Mapovani pameti pro zarizeni (`DMA_MAP`); zavreni = odmapovani.
    DmaMap = 13,
}

/// Zprava MSI, kterou ovladac zapise do zarizeni (vystup `INTERRUPT_CREATE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct MsiMessage {
    pub address: u64,
    pub data: u32,
    pub _reserved: u32,
}

/// Paket portu (vystup `PORT_WAIT`, 16 B): klic zadany pri
/// `OBJECT_WAIT_ASYNC` a signaly objektu v okamziku, kdy podminka nastala.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct PortPacket {
    pub key: u64,
    pub signals: u32,
    pub _reserved: u32,
}

impl PortPacket {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&self.key.to_le_bytes());
        b[8..12].copy_from_slice(&self.signals.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; 16]) -> Self {
        Self {
            key: u64::from_le_bytes(b[..8].try_into().unwrap()),
            signals: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            _reserved: 0,
        }
    }
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
