//! Startovni zprava procesu.
//!
//! Prvni vlakno procesu dostane v `rdi` handle kanalu (bootstrap). Ve
//! fronte kanalu ceka prave jedna zprava od rodice (u `init` od jadra):
//!
//! - data: [`Header`] a za ni `count` polozek [`Entry`],
//! - handly: ve stejnem poradi jako polozky; `Entry::kind` rika, co je handle.
//!
//! Za polozkami mohou nasledovat doplnkova data (napr. [`FramebufferInfo`]);
//! polozka na ne ukazuje offsetem v `arg`.
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

/// Priznaky startu v `arg` polozky `SELF_PROCESS`.
pub mod flags {
    /// Testovaci beh (jadro s feature `qemu-test`): init spusti testy
    /// userspace a skonci s jejich vysledkem. Bez priznaku bezi init jako
    /// supervisor stale.
    pub const TEST_RUN: u32 = 1 << 0;
}

/// Co znamena predany handle.
pub mod kind {
    /// Handle na vlastni proces (mapovani pameti, vlakna). `arg` = priznaky
    /// startu ([`super::flags`]); nastavuje je jen jadro pro init.
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
    /// Job, ve kterem proces bezi (prava bez `WRITE`: proces v nem smi
    /// spoustet dalsi procesy a zakladat podrizene joby, ale nemeni si
    /// limity). Init dostane korenovy job se vsemi pravy.
    pub const JOB: u32 = 6;
    /// Fyzicky objekt pameti s linearnim framebufferem z firmwaru (UEFI
    /// GOP), prava `READ | WRITE | MAP` (cist a psat jde jen pres
    /// mapovani, ne syscally), mapuje se write-combining. `arg` = offset
    /// [`super::FramebufferInfo`] v datech zpravy (za polozkami). Jadro
    /// po predani do framebufferu nekresli (jen pri panice).
    pub const FRAMEBUFFER: u32 = 7;
}

/// Format pixelu framebufferu (vzdy 32 bitu na pixel).
pub mod pixel {
    /// Bajty v pameti: R, G, B, rezerva.
    pub const RGB: u32 = 1;
    /// Bajty v pameti: B, G, R, rezerva.
    pub const BGR: u32 = 2;
}

/// Geometrie framebufferu (polozka `kind::FRAMEBUFFER`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramebufferInfo {
    /// Sirka v pixelech.
    pub width: u32,
    /// Vyska v pixelech.
    pub height: u32,
    /// Pocet pixelu na radek (>= `width`).
    pub stride: u32,
    /// [`pixel::RGB`] nebo [`pixel::BGR`].
    pub format: u32,
}

pub const FRAMEBUFFER_INFO_SIZE: usize = 16;

impl FramebufferInfo {
    pub fn to_bytes(self) -> [u8; FRAMEBUFFER_INFO_SIZE] {
        let mut b = [0; FRAMEBUFFER_INFO_SIZE];
        b[0..4].copy_from_slice(&self.width.to_le_bytes());
        b[4..8].copy_from_slice(&self.height.to_le_bytes());
        b[8..12].copy_from_slice(&self.stride.to_le_bytes());
        b[12..16].copy_from_slice(&self.format.to_le_bytes());
        b
    }

    /// Precte a overi geometrii (nenulove rozmery, `stride >= width`,
    /// znamy format).
    pub fn from_bytes(b: &[u8]) -> Option<Self> {
        let u = |o: usize| Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?));
        let i = Self {
            width: u(0)?,
            height: u(4)?,
            stride: u(8)?,
            format: u(12)?,
        };
        let ok = i.width > 0
            && i.height > 0
            && i.stride >= i.width
            && matches!(i.format, pixel::RGB | pixel::BGR);
        ok.then_some(i)
    }

    /// Velikost v bajtech (`stride * height * 4`).
    pub fn bytes(&self) -> u64 {
        self.stride as u64 * self.height as u64 * 4
    }
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
