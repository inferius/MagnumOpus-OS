//! Trida mass storage: Bulk-Only Transport (BOT, "USB Mass Storage Class
//! Bulk-Only Transport" rev. 1.0) a prikazy SCSI (SPC-4, SBC-3), ktere
//! ovladac `usb-storage` potrebuje.
//!
//! Jen kodovani a rozbor (bez I/O), aby slo vse testovat na hostiteli.
//!
//! Prubeh jednoho prikazu BOT:
//! 1. CBW (31 B) na bulk OUT,
//! 2. data na bulk IN nebo OUT (muze chybet),
//! 3. CSW (13 B) na bulk IN.
//!
//! Kdyz zarizeni datovou fazi zastavi (STALL), hostitel endpoint uvolni
//! (CLEAR_FEATURE ENDPOINT_HALT) a CSW precte stejne. Kdyz CSW nejde
//! precist ani podruhe nebo je neplatne, nasleduje Reset Recovery
//! (Bulk-Only Mass Storage Reset a uvolneni obou endpointu).

use crate::desc::{Setup, request, rt};

/// Podtrida "SCSI transparent command set".
pub const SUBCLASS_SCSI: u8 = 0x06;
/// Protokol Bulk-Only Transport.
pub const PROTOCOL_BOT: u8 = 0x50;
/// Protokol USB Attached SCSI (UAS; zatim nepodporujeme, zarizeni UAS ma
/// skoro vzdy i alternativu BOT).
pub const PROTOCOL_UAS: u8 = 0x62;

/// Podpis CBW ("USBC").
pub const CBW_SIGNATURE: u32 = 0x4342_5355;
/// Podpis CSW ("USBS").
pub const CSW_SIGNATURE: u32 = 0x5342_5355;
pub const CBW_LEN: usize = 31;
pub const CSW_LEN: usize = 13;

/// Command Block Wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cbw {
    pub tag: u32,
    /// Ocekavana delka dat.
    pub length: u32,
    /// Smer dat zarizeni -> hostitel.
    pub data_in: bool,
    pub lun: u8,
    /// Prikaz SCSI (1-16 B).
    pub cdb: [u8; 16],
    pub cdb_len: u8,
}

impl Cbw {
    pub fn new(tag: u32, lun: u8, length: u32, data_in: bool, cdb: &[u8]) -> Self {
        let mut c = [0u8; 16];
        let n = cdb.len().min(16);
        c[..n].copy_from_slice(&cdb[..n]);
        Self {
            tag,
            length,
            data_in,
            lun,
            cdb: c,
            cdb_len: n as u8,
        }
    }

    pub fn encode(&self) -> [u8; CBW_LEN] {
        let mut b = [0u8; CBW_LEN];
        b[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        b[4..8].copy_from_slice(&self.tag.to_le_bytes());
        b[8..12].copy_from_slice(&self.length.to_le_bytes());
        b[12] = if self.data_in { 0x80 } else { 0 };
        b[13] = self.lun & 0x0F;
        b[14] = self.cdb_len & 0x1F;
        b[15..31].copy_from_slice(&self.cdb);
        b
    }
}

/// Stav v CSW.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CswStatus {
    /// Prikaz prosel.
    Passed,
    /// Prikaz selhal: duvod zjisti REQUEST SENSE.
    Failed,
    /// Chyba faze: nutny Reset Recovery.
    PhaseError,
}

/// Command Status Wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Csw {
    pub tag: u32,
    /// Kolik ocekavanych dat zarizeni neprenieslo.
    pub residue: u32,
    pub status: CswStatus,
}

impl Csw {
    /// Rozebere CSW; `None` = neplatna delka, podpis nebo stav (zarizeni je
    /// mimo synchronizaci, nutny Reset Recovery). Tag kontroluje volajici.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() != CSW_LEN {
            return None;
        }
        let w = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        if w(0) != CSW_SIGNATURE {
            return None;
        }
        let status = match b[12] {
            0 => CswStatus::Passed,
            1 => CswStatus::Failed,
            2 => CswStatus::PhaseError,
            _ => return None,
        };
        Some(Self {
            tag: w(4),
            residue: w(8),
            status,
        })
    }

    pub fn encode(&self) -> [u8; CSW_LEN] {
        let mut b = [0u8; CSW_LEN];
        b[0..4].copy_from_slice(&CSW_SIGNATURE.to_le_bytes());
        b[4..8].copy_from_slice(&self.tag.to_le_bytes());
        b[8..12].copy_from_slice(&self.residue.to_le_bytes());
        b[12] = match self.status {
            CswStatus::Passed => 0,
            CswStatus::Failed => 1,
            CswStatus::PhaseError => 2,
        };
        b
    }
}

/// Bulk-Only Mass Storage Reset (pozadavek tridy na rozhrani).
pub const fn reset(interface: u8) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::CLASS | rt::TO_INTERFACE,
        request: 0xFF,
        value: 0,
        index: interface as u16,
        length: 0,
    }
}

/// Get Max LUN: odpoved 1 B = nejvyssi cislo LUN (zarizeni s jednim LUN
/// muze odpovedet STALL, to znamena 0).
pub const fn get_max_lun(interface: u8) -> Setup {
    Setup {
        request_type: rt::DEVICE_TO_HOST | rt::CLASS | rt::TO_INTERFACE,
        request: 0xFE,
        value: 0,
        index: interface as u16,
        length: 1,
    }
}

/// CLEAR_FEATURE(ENDPOINT_HALT) na endpoint `address`.
pub const fn clear_halt(address: u8) -> Setup {
    Setup {
        request_type: rt::HOST_TO_DEVICE | rt::STANDARD | rt::TO_ENDPOINT,
        request: request::CLEAR_FEATURE,
        value: 0,
        index: address as u16,
        length: 0,
    }
}

/// Prikazy SCSI.
pub mod scsi {
    pub const TEST_UNIT_READY: u8 = 0x00;
    pub const REQUEST_SENSE: u8 = 0x03;
    pub const INQUIRY: u8 = 0x12;
    pub const START_STOP_UNIT: u8 = 0x1B;
    pub const PREVENT_ALLOW_REMOVAL: u8 = 0x1E;
    pub const READ_CAPACITY_10: u8 = 0x25;
    pub const READ_10: u8 = 0x28;
    pub const WRITE_10: u8 = 0x2A;
    pub const SYNCHRONIZE_CACHE_10: u8 = 0x35;
    pub const MODE_SENSE_6: u8 = 0x1A;
    pub const READ_16: u8 = 0x88;
    pub const WRITE_16: u8 = 0x8A;
    pub const SERVICE_ACTION_IN_16: u8 = 0x9E;
    /// Service action READ CAPACITY (16).
    pub const SA_READ_CAPACITY_16: u8 = 0x10;

    /// Delka odpovedi INQUIRY, kterou zadame (standardni cast).
    pub const INQUIRY_LEN: u8 = 36;
    /// Delka odpovedi REQUEST SENSE (fixni format).
    pub const SENSE_LEN: u8 = 18;

    pub fn test_unit_ready() -> [u8; 6] {
        [TEST_UNIT_READY, 0, 0, 0, 0, 0]
    }

    pub fn request_sense() -> [u8; 6] {
        [REQUEST_SENSE, 0, 0, 0, SENSE_LEN, 0]
    }

    pub fn inquiry() -> [u8; 6] {
        [INQUIRY, 0, 0, 0, INQUIRY_LEN, 0]
    }

    pub fn read_capacity_10() -> [u8; 10] {
        [READ_CAPACITY_10, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    }

    pub fn read_capacity_16() -> [u8; 16] {
        let mut c = [0u8; 16];
        c[0] = SERVICE_ACTION_IN_16;
        c[1] = SA_READ_CAPACITY_16;
        c[13] = 32; // delka odpovedi
        c
    }

    /// MODE SENSE (6), vsechny stranky jen hlavicka: bit WP (ochrana proti
    /// zapisu) je v bajtu 2 odpovedi.
    pub fn mode_sense_6() -> [u8; 6] {
        [MODE_SENSE_6, 0, 0x3F, 0, 4, 0]
    }

    /// READ/WRITE: (10) pro LBA a pocet, ktere se vejdou, jinak (16).
    pub fn rw(write: bool, lba: u64, count: u32) -> ([u8; 16], usize) {
        let mut c = [0u8; 16];
        if lba + count as u64 <= u32::MAX as u64 + 1 && count <= u16::MAX as u32 {
            c[0] = if write { WRITE_10 } else { READ_10 };
            c[2..6].copy_from_slice(&(lba as u32).to_be_bytes());
            c[7..9].copy_from_slice(&(count as u16).to_be_bytes());
            (c, 10)
        } else {
            c[0] = if write { WRITE_16 } else { READ_16 };
            c[2..10].copy_from_slice(&lba.to_be_bytes());
            c[10..14].copy_from_slice(&count.to_be_bytes());
            (c, 16)
        }
    }

    pub fn synchronize_cache() -> [u8; 10] {
        [SYNCHRONIZE_CACHE_10, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    }

    /// Kapacita: (posledni LBA, velikost bloku). READ CAPACITY (10) vraci
    /// posledni LBA 0xFFFF_FFFF, kdyz je disk vetsi (pak (16)).
    pub fn parse_capacity_10(b: &[u8]) -> Option<(u64, u32)> {
        let b = b.get(..8)?;
        let last = u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64;
        let bs = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
        Some((last, bs))
    }

    pub fn parse_capacity_16(b: &[u8]) -> Option<(u64, u32)> {
        let b = b.get(..12)?;
        let last = u64::from_be_bytes(b[..8].try_into().ok()?);
        let bs = u32::from_be_bytes([b[8], b[9], b[10], b[11]]);
        Some((last, bs))
    }

    /// Udaje z INQUIRY.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Inquiry {
        /// Typ zarizeni (0 = primy pristup, disk).
        pub device_type: u8,
        pub removable: bool,
        pub vendor: alloc::string::String,
        pub product: alloc::string::String,
        pub revision: alloc::string::String,
    }

    fn text(b: &[u8]) -> alloc::string::String {
        let s: alloc::string::String = b
            .iter()
            .map(|&c| if c.is_ascii_graphic() || c == b' ' { c as char } else { '?' })
            .collect();
        alloc::string::String::from(s.trim())
    }

    pub fn parse_inquiry(b: &[u8]) -> Option<Inquiry> {
        if b.len() < 36 {
            return None;
        }
        Some(Inquiry {
            device_type: b[0] & 0x1F,
            removable: b[1] & 0x80 != 0,
            vendor: text(&b[8..16]),
            product: text(&b[16..32]),
            revision: text(&b[32..36]),
        })
    }

    /// Klic, ASC a ASCQ z REQUEST SENSE (fixni format 0x70/0x71, popisny
    /// 0x72/0x73).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Sense {
        pub key: u8,
        pub asc: u8,
        pub ascq: u8,
    }

    pub mod key {
        pub const NO_SENSE: u8 = 0x0;
        pub const NOT_READY: u8 = 0x2;
        pub const MEDIUM_ERROR: u8 = 0x3;
        pub const ILLEGAL_REQUEST: u8 = 0x5;
        pub const UNIT_ATTENTION: u8 = 0x6;
        pub const DATA_PROTECT: u8 = 0x7;
    }

    pub fn parse_sense(b: &[u8]) -> Option<Sense> {
        match b.first()? & 0x7F {
            0x70 | 0x71 if b.len() >= 14 => Some(Sense {
                key: b[2] & 0x0F,
                asc: b[12],
                ascq: b[13],
            }),
            0x72 | 0x73 if b.len() >= 4 => Some(Sense {
                key: b[1] & 0x0F,
                asc: b[2],
                ascq: b[3],
            }),
            _ => None,
        }
    }

    impl Sense {
        /// Ma smysl prikaz zopakovat (zarizeni se teprve pripravuje, medium
        /// vymeneno, reset)?
        pub fn retryable(&self) -> bool {
            matches!(self.key, key::NOT_READY | key::UNIT_ATTENTION | key::NO_SENSE)
                && !(self.key == key::NOT_READY && self.asc == 0x3A) // medium chybi
        }

        /// Medium chybi (prazdna ctecka karet).
        pub fn no_medium(&self) -> bool {
            self.key == key::NOT_READY && self.asc == 0x3A
        }
    }

    /// Bit WP z hlavicky MODE SENSE (6).
    pub fn write_protected(mode_header: &[u8]) -> bool {
        mode_header.get(2).is_some_and(|b| b & 0x80 != 0)
    }
}

/// Je rozhrani disk, ktery umime (mass storage, SCSI, BOT)?
pub fn is_bot_disk(class: u8, subclass: u8, protocol: u8) -> bool {
    class == crate::desc::class::MASS_STORAGE && subclass == SUBCLASS_SCSI && protocol == PROTOCOL_BOT
}
