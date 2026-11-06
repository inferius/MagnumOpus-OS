//! Startovni zprava procesu.
//!
//! Prvni vlakno procesu dostane v `rdi` handle kanalu (bootstrap). Ve
//! fronte kanalu ceka prave jedna zprava od rodice (u `init` od jadra):
//!
//! - data: [`Header`] a za ni `count` polozek [`Entry`],
//! - handly: ve stejnem poradi jako polozky; `Entry::kind` rika, co je handle.
//!
//! Proces si zpravu precte (libos to dela pred `main`) a kanal muze dal
//! pouzivat pro komunikaci s rodicem.

/// "OSBS" v ASCII.
pub const MAGIC: u32 = u32::from_le_bytes(*b"OSBS");
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Header {
    pub magic: u32,
    pub version: u32,
    pub count: u32,
    pub _reserved: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct Entry {
    pub kind: u32,
    /// Doplnujici hodnota podle druhu (napr. rozsah sbernic u PCI_ECAM).
    pub arg: u32,
}

/// Co znamena predany handle.
pub mod kind {
    /// Handle na vlastni proces (mapovani pameti, vlakna).
    pub const SELF_PROCESS: u32 = 1;
    /// Objekt pameti s obrazem initfs (jen pro cteni).
    pub const INITFS: u32 = 2;
    /// Volny handle predany rodicem podle dohody s programem.
    pub const USER: u32 = 3;
    /// Korenovy Resource (pristup k hardwaru).
    pub const ROOT_RESOURCE: u32 = 4;
    /// Fyzicky objekt pameti s oknem ECAM (konfigurace PCI Express);
    /// `arg` = prvni sbernice | posledni sbernice << 8 | segment << 16.
    pub const PCI_ECAM: u32 = 5;
}

pub const HEADER_SIZE: usize = core::mem::size_of::<Header>();
pub const ENTRY_SIZE: usize = core::mem::size_of::<Entry>();

impl Header {
    pub fn to_bytes(self) -> [u8; HEADER_SIZE] {
        let mut b = [0; HEADER_SIZE];
        b[0..4].copy_from_slice(&self.magic.to_le_bytes());
        b[4..8].copy_from_slice(&self.version.to_le_bytes());
        b[8..12].copy_from_slice(&self.count.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let u = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
        let h = Self {
            magic: u(0)?,
            version: u(4)?,
            count: u(8)?,
            _reserved: 0,
        };
        (h.magic == MAGIC && h.version == VERSION).then_some(h)
    }
}

impl Entry {
    pub fn to_bytes(self) -> [u8; ENTRY_SIZE] {
        let mut b = [0; ENTRY_SIZE];
        b[0..4].copy_from_slice(&self.kind.to_le_bytes());
        b[4..8].copy_from_slice(&self.arg.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let kind = u32::from_le_bytes(b.get(0..4)?.try_into().ok()?);
        let arg = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
        Some(Self { kind, arg })
    }
}
