//! Zaznamy dat na disku (faze 3): nastaveni dat datasetu, tabulka sdileni
//! bloku a index deduplikace.
//!
//! Tabulka sdileni a index lezi ve strome korenoveho datasetu (objekt
//! `META_OID`), takze se meni atomicky se zmenou, ke ktere patri (commit
//! vsech datasetu je jedna transakce). Cela cisla little-endian; neplatna
//! hodnota = `CORRUPTED`.

use alloc::vec::Vec;

use abi::{Error, Result};

use crate::hash::Csum;
use crate::layout::{Compress, csum_at, u32_at, u64_at};

/// Deduplikace datasetu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupMode {
    /// Zadna (ani na pozadi).
    Off = 0,
    /// Na pozadi (vychozi): nove zapsane kusy prochazi sluzba pri
    /// necinnosti, zapis se nezdrzuje.
    Background = 1,
    /// Okamzite pri zapisu (a k tomu na pozadi): shoda se pozna uz pri
    /// zapisu, ten je pomalejsi (hledani v indexu, u XXH3 porovnani bajtu).
    Inline = 2,
}

/// Nastaveni dat datasetu (`item::DATAOPT`, objekt `META_OID`, offset 0).
///
/// ```text
/// 0 komprese u8 (0 = vypnuto, Compress)   1 kopii dat u8 (1 nebo 2)
/// 2 deduplikace u8 (DedupMode)            3 rezerva (5 B)
/// ```
///
/// Chybi-li zaznam, plati `Default` (bez komprese, data 1x, deduplikace na
/// pozadi). Novy dataset dedi nastaveni rodice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataSettings {
    pub compress: Option<Compress>,
    pub copies: u8,
    pub dedup: DedupMode,
}

impl Default for DataSettings {
    fn default() -> Self {
        Self {
            compress: None,
            copies: 1,
            dedup: DedupMode::Background,
        }
    }
}

const DATAOPT_SIZE: usize = 8;

impl DataSettings {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; DATAOPT_SIZE];
        b[0] = self.compress.map_or(0, |c| c as u8);
        b[1] = self.copies;
        b[2] = self.dedup as u8;
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != DATAOPT_SIZE {
            return Err(Error::Corrupted);
        }
        let dedup = match b[2] {
            0 => DedupMode::Off,
            1 => DedupMode::Background,
            2 => DedupMode::Inline,
            _ => return Err(Error::Corrupted),
        };
        let s = Self {
            compress: Compress::from_raw(b[0]).map_err(|_| Error::Corrupted)?,
            copies: b[1],
            dedup,
        };
        s.validate().map_err(|_| Error::Corrupted)?;
        Ok(s)
    }

    pub fn validate(&self) -> Result<()> {
        if !(1..=2).contains(&self.copies) {
            return Err(Error::InvalidArgs);
        }
        Ok(())
    }
}

/// Zaznam tabulky sdileni (`item::SHARE`, objekt `META_OID`, offset =
/// fyzicky zacatek useku).
///
/// ```text
/// 0  delka u32        4  pocet odkazu u32 (>= 1)
/// 8  kopie u64 (zacatek druhe kopie, 0 = neni)
/// 16 priznaky u32 (SHARE_INDEXED)   20 rezerva u32
/// 24 [hash obsahu 16 B]             jen se SHARE_INDEXED
/// ```
///
/// Usek je v tabulce, dokud na nej ukazuje aspon jeden odkaz (kazdy odkaz
/// je extent nebo zaznam deadlistu, ktery se jednou definitivne uvolni).
/// Uvolneni odkazu snizi pocet; posledni usek uvolni (vcetne kopie).
/// Indexovany usek (deduplikace) ma navic polozku v indexu `item::DEDUP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareEntry {
    pub len: u32,
    pub refs: u32,
    pub copy: u64,
    pub hash: Option<Csum>,
}

pub const SHARE_INDEXED: u32 = 1;
const SHARE_HEADER: usize = 24;

impl ShareEntry {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(SHARE_HEADER + 16);
        b.extend_from_slice(&self.len.to_le_bytes());
        b.extend_from_slice(&self.refs.to_le_bytes());
        b.extend_from_slice(&self.copy.to_le_bytes());
        let flags = if self.hash.is_some() { SHARE_INDEXED } else { 0 };
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        if let Some(h) = self.hash {
            b.extend_from_slice(&h.0);
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < SHARE_HEADER {
            return Err(Error::Corrupted);
        }
        let flags = u32_at(b, 16);
        let indexed = flags & SHARE_INDEXED != 0;
        let want = SHARE_HEADER + if indexed { 16 } else { 0 };
        if flags & !SHARE_INDEXED != 0 || b.len() != want {
            return Err(Error::Corrupted);
        }
        let e = Self {
            len: u32_at(b, 0),
            refs: u32_at(b, 4),
            copy: u64_at(b, 8),
            hash: indexed.then(|| csum_at(b, SHARE_HEADER)),
        };
        if e.len == 0 || e.refs == 0 {
            return Err(Error::Corrupted);
        }
        Ok(e)
    }
}

/// Polozka indexu deduplikace (`item::DEDUP`, objekt `META_OID`, offset =
/// prvnich 8 bajtu hashe): seznam (hash 16 B, fyzicky zacatek u64) - vic
/// hashu se stejnym zacatkem sdili polozku.
pub fn encode_dedup(list: &[(Csum, u64)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(list.len() * 24);
    for (h, p) in list {
        b.extend_from_slice(&h.0);
        b.extend_from_slice(&p.to_le_bytes());
    }
    b
}

pub fn decode_dedup(b: &[u8]) -> Result<Vec<(Csum, u64)>> {
    if b.is_empty() || !b.len().is_multiple_of(24) {
        return Err(Error::Corrupted);
    }
    Ok(b.chunks(24).map(|c| (csum_at(c, 0), u64_at(c, 16))).collect())
}

/// Offset polozky indexu pro hash.
pub fn dedup_key(h: &Csum) -> u64 {
    u64::from_le_bytes(h.0[..8].try_into().unwrap())
}

/// Stav scrubu (`item::SCRUB`, strom korenoveho datasetu, offset 0).
///
/// ```text
/// 0  stav u8 (0 = neni, 1 = bezi, 2 = zastaveny)   1 rezerva (7 B)
/// 8  zacatek behu (cas 12 B)      20 konec posledniho dokonceneho (12 B)
/// 32 pozice: dataset u64          40 snapshot u64 (u64::MAX = ziva verze)
/// 48 klic: oid u64, 56 druh u8 (+ 7 B rezerva), 64 offset u64
/// 72 bloku prectenych u64         80 spatnych kopii u64
/// 88 opraveno u64                 96 neopravitelnych u64
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScrubState {
    pub state: u8,
    pub started: crate::layout::Timestamp,
    pub finished: crate::layout::Timestamp,
    pub dataset: u64,
    pub snapshot: u64,
    pub key: crate::layout::Key,
    pub scanned: u64,
    pub bad: u64,
    pub repaired: u64,
    pub lost: u64,
}

pub const SCRUB_IDLE: u8 = 0;
pub const SCRUB_RUNNING: u8 = 1;
pub const SCRUB_STOPPED: u8 = 2;
const SCRUB_SIZE: usize = 104;
/// Nejvic zaznamu neopravitelnych chyb jednoho behu.
pub const SCRUB_MAX_ERRORS: u64 = 64;

fn put_ts(b: &mut [u8], o: usize, t: crate::layout::Timestamp) {
    b[o..o + 8].copy_from_slice(&t.secs.to_le_bytes());
    b[o + 8..o + 12].copy_from_slice(&t.nanos.to_le_bytes());
}

fn get_ts(b: &[u8], o: usize) -> Result<crate::layout::Timestamp> {
    let nanos = u32_at(b, o + 8);
    if nanos >= 1_000_000_000 {
        return Err(Error::Corrupted);
    }
    Ok(crate::layout::Timestamp { secs: u64_at(b, o) as i64, nanos })
}

impl ScrubState {
    pub fn encode(&self) -> Vec<u8> {
        let mut b = alloc::vec![0u8; SCRUB_SIZE];
        b[0] = self.state;
        put_ts(&mut b, 8, self.started);
        put_ts(&mut b, 20, self.finished);
        let words = [
            (32, self.dataset),
            (40, self.snapshot),
            (48, self.key.oid),
            (64, self.key.offset),
            (72, self.scanned),
            (80, self.bad),
            (88, self.repaired),
            (96, self.lost),
        ];
        for (o, v) in words {
            b[o..o + 8].copy_from_slice(&v.to_le_bytes());
        }
        b[56] = self.key.kind;
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != SCRUB_SIZE || b[0] > SCRUB_STOPPED {
            return Err(Error::Corrupted);
        }
        Ok(Self {
            state: b[0],
            started: get_ts(b, 8)?,
            finished: get_ts(b, 20)?,
            dataset: u64_at(b, 32),
            snapshot: u64_at(b, 40),
            key: crate::layout::Key::new(u64_at(b, 48), b[56], u64_at(b, 64)),
            scanned: u64_at(b, 72),
            bad: u64_at(b, 80),
            repaired: u64_at(b, 88),
            lost: u64_at(b, 96),
        })
    }
}
