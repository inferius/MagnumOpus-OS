//! Ochrana cizich disku.
//!
//! Na skutecnem pocitaci (a ve VMware s pripojenym fyzickym diskem) vidi
//! ovladac NVMe nebo AHCI i disky, ktere nam nepatri: Windows, Linux,
//! zalohy. Jediny chybny zapis (napr. formatovani nebo prehrani logu FS na
//! spatnem disku) muze znicit cizi data. Proto ovladac pred obsluhou
//! klienta disk rozebere ([`decide`]) a podle vysledku:
//!
//! - **nas oddil GPT** (typ [`OUR_TYPE`] nebo jmeno [`OUR_LABEL`]): klient
//!   vidi jen tento oddil (sektor 0 klienta = prvni sektor oddilu) a smi do
//!   nej zapisovat; mimo oddil se zapsat neda,
//! - **cely disk s nasim FS** bez tabulky oddilu (starsi rozlozeni,
//!   datovy disk v QEMU): zapis povolen na cely disk,
//! - **cokoli jineho** (cizi GPT, MBR, prazdny disk, poskozena tabulka,
//!   chyba cteni): cely disk jen pro cteni. Zapis, DISCARD i nebezpecne
//!   operace vraci `READ_ONLY`.
//!
//! Rozhodnuti je zamerne opatrne: pri pochybnosti (prekryv oddilu, oddil
//! mimo disk, spatny kontrolni soucet) plati "cizi disk". Knihovna nezavisi
//! na `libos`, aby sla testovat na hostiteli (`tests/`) a aby ji mohl
//! pouzit i `xtask` (zapis GPT testovacich disku, [`gpt::write`]).

#![no_std]

extern crate alloc;

pub mod crc32;
pub mod gpt;
pub mod install;

/// Sektor protokolu klienta (`blockproto`). Disky mohou mit vetsi logicky
/// sektor (4 KiB); knihovna pracuje v sektorech disku, prepocet na 512 B
/// dela server ovladace.
pub const SECTOR: usize = 512;

/// Podporovana velikost logickeho sektoru disku: mocnina dvou 512 B..64 KiB.
pub const fn valid_sector_size(ss: usize) -> bool {
    ss.is_power_of_two() && ss >= 512 && ss <= 65536
}

/// GUID v binarni podobe GPT (prvni tri slozky little-endian).
pub type Guid = [u8; 16];

/// GUID z textoveho zapisu `aaaaaaaa-bbbb-cccc-dddd-dddddddddddd`.
pub const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

/// Typ oddilu GPT "datovy oddil MagnumOpus" (souborovy system OSFS):
/// C123ADA0-51BA-48C5-A536-C2CE6B2B3581. Vygenerovan nahodne, s jinym
/// systemem se nepotka.
pub const OUR_TYPE: Guid = guid(
    0xC123_ADA0,
    0x51BA,
    0x48C5,
    [0xA5, 0x36, 0xC2, 0xCE, 0x6B, 0x2B, 0x35, 0x81],
);

/// Jmeno oddilu, ktere oddil oznaci za nas i s jinym typem (napr. kdyz
/// nastroj na oddily neumi zadat vlastni GUID typu).
pub const OUR_LABEL: &str = "MagnumOpus";

/// Typ "EFI System Partition" (systemovy disk).
pub const ESP_TYPE: Guid = guid(
    0xC12A_7328,
    0xF81F,
    0x11D2,
    [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B],
);

/// Atribut oddilu "jen pro cteni" (bit 60, stejny vyznam jako u oddilu
/// Microsoft basic data). Respektuje se i u naseho typu.
pub const ATTR_READ_ONLY: u64 = 1 << 60;

/// Typ "Microsoft basic data" (FAT/NTFS; Windows ho pripoji).
pub const BASIC_DATA_TYPE: Guid = guid(
    0xEBD0_A0A2,
    0xB9E5,
    0x4433,
    [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7],
);
/// Jmeno oddilu pro logy na flashce (FAT32, cte ho Windows).
pub const LOG_LABEL: &str = "MO-LOGY";

/// Zacatek podpisu superbloku naseho FS (`OSFS\0\0v3`; verze se nekontroluje,
/// i novejsi format je nas).
pub const FS_MAGIC_PREFIX: &[u8; 7] = b"OSFS\0\0v";
/// Superbloky FS lezi na zacatku bloku 1..=8 po 4 KiB (bajty 4096..36864).
pub const FS_BLOCK: u64 = 4096;
pub const FS_SUPER_BLOCKS: u64 = 8;

/// Proc je disk (ne)nas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Oddil GPT s nasim typem nebo jmenem (`index` = poradi v tabulce od 0).
    OurPartition { index: u32 },
    /// Cely disk s nasim FS, bez tabulky oddilu.
    OurWholeDisk,
    /// GPT bez naseho (platneho) oddilu.
    ForeignGpt,
    /// Nas oddil existuje, ale je neplatny (mimo disk, prekryva jiny oddil).
    InvalidPartition,
    /// Tabulka MBR (bez GPT).
    Mbr,
    /// Ochranny MBR, ale obe hlavicky GPT poskozene.
    DamagedGpt,
    /// Bez tabulky oddilu a bez naseho FS (i prazdny disk).
    Unknown,
    /// Disk nejde precist.
    Unreadable,
    /// Oddil pro logy (FAT32 `MO-LOGY`, typ basic data) na disku s nasim
    /// oddilem - zive USB ([`log_partition`]).
    LogPartition { index: u32 },
    /// Cely disk pro instalator ("raw" kanal): zapis jen do struktur GPT
    /// a nasich oddilu ([`install::raw_write_allowed`]).
    Installer,
}

impl Kind {
    /// Kratky popis pro log (cesky).
    pub fn describe(&self) -> &'static str {
        match self {
            Kind::OurPartition { .. } => "nas oddil GPT",
            Kind::OurWholeDisk => "cely disk s nasim FS",
            Kind::ForeignGpt => "cizi disk (GPT bez naseho oddilu)",
            Kind::InvalidPartition => "cizi disk (nas oddil je neplatny)",
            Kind::Mbr => "cizi disk (MBR)",
            Kind::DamagedGpt => "cizi disk (poskozena GPT)",
            Kind::Unknown => "cizi nebo prazdny disk (bez tabulky oddilu)",
            Kind::Unreadable => "disk nejde precist",
            Kind::LogPartition { .. } => "oddil pro logy (FAT32 MO-LOGY)",
            Kind::Installer => "cely disk pro instalator",
        }
    }

    pub fn is_ours(&self) -> bool {
        matches!(self, Kind::OurPartition { .. } | Kind::OurWholeDisk)
    }
}

/// Vysledek rozboru: co klient uvidi a jestli smi zapisovat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub kind: Kind,
    /// Prvni sektor zarizeni, ktery klient vidi jako sektor 0.
    pub start: u64,
    /// Pocet sektoru zarizeni viditelnych klientem.
    pub sectors: u64,
    /// Velikost sektoru zarizeni v bajtech (`start` a `sectors` jsou v nich).
    pub sector_size: usize,
    /// Smi klient zapisovat (a posilat DISCARD)?
    pub writable: bool,
}

impl Decision {
    fn whole(kind: Kind, sectors: u64, sector_size: usize) -> Self {
        Self {
            kind,
            start: 0,
            sectors,
            sector_size,
            writable: kind == Kind::OurWholeDisk,
        }
    }

    /// Preklad rozsahu v sektorech zarizeni (relativne k videnemu useku)
    /// na absolutni sektor zarizeni. `None` = mimo videny rozsah (klient
    /// dostane `OUT_OF_RANGE`).
    pub fn translate(&self, lba: u64, count: u64) -> Option<u64> {
        let end = lba.checked_add(count)?;
        (end <= self.sectors).then_some(self.start + lba)
    }
}

/// Rozebere disk o `sectors` sektorech po `ss` bajtech. `read(lba, buf)`
/// precte `buf.len() / ss` sektoru od `lba` (`false` = chyba).
pub fn decide(sectors: u64, ss: usize, mut read: impl FnMut(u64, &mut [u8]) -> bool) -> Decision {
    if !valid_sector_size(ss) {
        return Decision::whole(Kind::Unreadable, sectors, ss);
    }
    let mut s0 = alloc::vec![0u8; ss];
    if sectors < 2 || !read(0, &mut s0) {
        return Decision::whole(Kind::Unreadable, sectors, ss);
    }
    let has_mbr_sig = s0[510] == 0x55 && s0[511] == 0xAA;
    let protective = has_mbr_sig && (0..4).any(|i| s0[446 + i * 16 + 4] == 0xEE);
    match gpt::read_table(sectors, ss, &mut read) {
        gpt::Read::Table(t) => return choose(&t, sectors, ss),
        gpt::Read::Damaged => return Decision::whole(Kind::DamagedGpt, sectors, ss),
        gpt::Read::Unreadable => return Decision::whole(Kind::Unreadable, sectors, ss),
        gpt::Read::None => {}
    }
    if protective {
        return Decision::whole(Kind::DamagedGpt, sectors, ss);
    }
    if has_mbr_sig {
        return Decision::whole(Kind::Mbr, sectors, ss);
    }
    if has_our_fs(sectors, ss, &mut read) {
        return Decision::whole(Kind::OurWholeDisk, sectors, ss);
    }
    Decision::whole(Kind::Unknown, sectors, ss)
}

/// Superblok naseho FS na zacatku nektereho z bloku 1..=8 (cely disk bez
/// oddilu). Pocita s libovolnou velikosti sektoru (blok muze lezet
/// uprostred sektoru vetsiho nez 4 KiB).
fn has_our_fs(sectors: u64, ss: usize, read: &mut impl FnMut(u64, &mut [u8]) -> bool) -> bool {
    let end = (FS_SUPER_BLOCKS + 1) * FS_BLOCK;
    if sectors.saturating_mul(ss as u64) < end {
        return false;
    }
    let mut buf = alloc::vec![0u8; ss];
    (1..=FS_SUPER_BLOCKS).any(|b| {
        let byte = b * FS_BLOCK;
        let off = (byte % ss as u64) as usize;
        read(byte / ss as u64, &mut buf) && buf[off..off + FS_MAGIC_PREFIX.len()] == FS_MAGIC_PREFIX[..]
    })
}

/// Oddil pro logy na disku s nasim oddilem: typ basic data a jmeno presne
/// [`LOG_LABEL`], cely v pouzitelne oblasti a bez prekryvu. Zapis do nej
/// smi jen pres knihovnu `fatw` (vlastni slozka). Disk bez platneho naseho
/// oddilu = `None` (cizi FAT oddil se jmenem MO-LOGY se nezapisuje).
pub fn log_partition(sectors: u64, ss: usize, mut read: impl FnMut(u64, &mut [u8]) -> bool) -> Option<Decision> {
    if !valid_sector_size(ss) {
        return None;
    }
    let gpt::Read::Table(t) = gpt::read_table(sectors, ss, &mut read) else {
        return None;
    };
    if !choose(&t, sectors, ss).kind.is_ours() {
        return None;
    }
    for (i, e) in t.entries().enumerate() {
        if e.is_empty() || e.type_guid != BASIC_DATA_TYPE || !e.name_is(LOG_LABEL) {
            continue;
        }
        let in_bounds = e.first <= e.last && e.first >= t.first_usable && e.last <= t.last_usable && e.last < sectors;
        let overlaps = t
            .entries()
            .enumerate()
            .any(|(j, o)| j != i && !o.is_empty() && o.first <= e.last && e.first <= o.last)
            || t.reserved.iter().any(|&(a, b)| a <= e.last && e.first <= b);
        if !in_bounds || overlaps {
            return None;
        }
        return Some(Decision {
            kind: Kind::LogPartition { index: i as u32 },
            start: e.first,
            sectors: e.last - e.first + 1,
            sector_size: ss,
            writable: e.attrs & ATTR_READ_ONLY == 0,
        });
    }
    None
}

/// Je oddil nas (typ nebo jmeno)?
pub fn is_our_entry(e: &gpt::Entry) -> bool {
    e.type_guid == OUR_TYPE || e.name_is(OUR_LABEL)
}

/// Vybere prvni platny nas oddil.
fn choose(t: &gpt::Table, sectors: u64, ss: usize) -> Decision {
    let mut saw_ours = false;
    for (i, e) in t.entries().enumerate() {
        if e.is_empty() || !is_our_entry(&e) {
            continue;
        }
        saw_ours = true;
        let in_bounds = e.first <= e.last
            && e.first >= t.first_usable
            && e.last <= t.last_usable
            && e.last < sectors;
        // Prekryv s jinym oddilem: zapis do "naseho" by prepsal cizi data.
        let overlaps = t
            .entries()
            .enumerate()
            .any(|(j, o)| j != i && !o.is_empty() && o.first <= e.last && e.first <= o.last)
            // Ani MBR, hlavicky a tabulky GPT (pouzitelna oblast z hlavicky
            // je jen udaj disku; chybny nastroj ji muze nastavit spatne).
            || t.reserved.iter().any(|&(a, b)| a <= e.last && e.first <= b);
        if !in_bounds || overlaps {
            continue;
        }
        return Decision {
            kind: Kind::OurPartition { index: i as u32 },
            start: e.first,
            sectors: e.last - e.first + 1,
            sector_size: ss,
            writable: e.attrs & ATTR_READ_ONLY == 0,
        };
    }
    let kind = if saw_ours {
        Kind::InvalidPartition
    } else {
        Kind::ForeignGpt
    };
    Decision::whole(kind, sectors, ss)
}
