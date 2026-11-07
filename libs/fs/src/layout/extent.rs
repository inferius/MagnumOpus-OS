//! Extent souboru: souvisly usek dat (polozka `item::EXTENT`, klic = prvni
//! logicky blok).
//!
//! ```text
//! 0   fyzicky zacatek u64       8   logickych bloku u32
//! 12  priznaky u32              16  generace vzniku u64
//! 24  [zacatek druhe kopie u64]               jen s EXTENT_COPY
//! ..  [fyzickych bloku u32, bajtu dat u32]    jen komprimovany
//! ..  soucty: nekomprimovany = jeden na blok, komprimovany = jeden za
//!     cela data na disku, nezapsany (predalokovany) = zadny
//! ```
//!
//! Priznaky: bit 0 `EXTENT_UNWRITTEN` (predalokovany, cte se jako nuly),
//! bit 1 `EXTENT_SHARED` (bloky mohou byt v tabulce sdileni - uvolnit az
//! s posledni referenci), bit 2 `EXTENT_COPY` (data maji druhou kopii),
//! bity 8-15 algoritmus komprese (`Compress`).
//!
//! **Generace vzniku je logicka:** u sdilenych bloku (okamzita kopie,
//! deduplikace) je to generace, kdy vznikl tento odkaz, ne kdy se bloky
//! zapsaly. Pravidla vlastnictvi (snapshot / deadlist, `Store::release`)
//! tak plati pro kazdy odkaz zvlast; fyzicke uvolneni hlida tabulka sdileni.
//!
//! Komprimovany extent je nedelitelny kus (nejvys `COMP_CHUNK` bloku):
//! zmena kterehokoli bloku prepise cely kus.

use alloc::vec::Vec;

use abi::{Error, Result};

use super::{BLOCK_SIZE, MAX_EXTENT, csum_at, u32_at, u64_at};
use crate::hash::Csum;

const EXTENT_HEADER: usize = 24;
/// Predalokovany, jeste nezapsany extent (cte se jako nuly, bez souctu).
pub const EXTENT_UNWRITTEN: u32 = 1 << 0;
/// Bloky extentu mohou byt sdilene (tabulka sdileni).
pub const EXTENT_SHARED: u32 = 1 << 1;
/// Extent ma druhou kopii dat (`Extent::copy`).
pub const EXTENT_COPY: u32 = 1 << 2;
const COMP_SHIFT: u32 = 8;
const COMP_MASK: u32 = 0xFF << COMP_SHIFT;
const KNOWN: u32 = EXTENT_UNWRITTEN | EXTENT_SHARED | EXTENT_COPY | COMP_MASK;

/// Kus komprese: 32 bloku = 128 KiB (nahodne cteni rozbali nejvys tolik).
pub const COMP_CHUNK: u64 = 32;

/// Algoritmus komprese (v extentu i v nastaveni datasetu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compress {
    /// LZ4 (blok, lz4_flex): rychla komprese i dekomprese.
    Lz4 = 1,
    /// Zstd (ramec, ruzstd): lepsi pomer, pomalejsi.
    Zstd = 2,
}

impl Compress {
    pub fn from_raw(v: u8) -> Result<Option<Self>> {
        match v {
            0 => Ok(None),
            1 => Ok(Some(Self::Lz4)),
            2 => Ok(Some(Self::Zstd)),
            _ => Err(Error::NotSupported),
        }
    }
}

/// Komprimovana podoba dat extentu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comp {
    pub algo: Compress,
    /// Fyzickych bloku na disku (< logickych).
    pub plen: u32,
    /// Delka komprimovanych dat v bajtech (konec posledniho bloku = nuly).
    pub bytes: u32,
}

/// Souvisly usek dat souboru: fyzicky zacatek, logicka delka, generace
/// vzniku (rozhoduje, zda bloky po prepsani uvolnit, nebo patri snapshotu)
/// a kontrolni soucty (nahodne cteni nekomprimovanych dat overi jen svuj
/// blok).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extent {
    pub phys: u64,
    pub birth: u64,
    pub csums: Vec<Csum>,
    /// Logickych bloku.
    pub blocks: u32,
    /// `EXTENT_UNWRITTEN`, `EXTENT_SHARED` (kopie a komprese maji pole).
    pub flags: u32,
    /// Zacatek druhe kopie (0 = bez kopie); kopie ma stejnou fyzickou delku.
    pub copy: u64,
    pub comp: Option<Comp>,
}

impl Extent {
    /// Nekomprimovany extent se souctem kazdeho bloku.
    pub fn plain(phys: u64, birth: u64, csums: Vec<Csum>) -> Self {
        Self {
            phys,
            birth,
            blocks: csums.len() as u32,
            csums,
            flags: 0,
            copy: 0,
            comp: None,
        }
    }

    /// Predalokovany extent `blocks` bloku (cte se jako nuly).
    pub fn unwritten(phys: u64, birth: u64, blocks: u32) -> Self {
        Self {
            phys,
            birth,
            csums: Vec::new(),
            blocks,
            flags: EXTENT_UNWRITTEN,
            copy: 0,
            comp: None,
        }
    }

    /// Logicka delka v blocich.
    pub fn len(&self) -> u64 {
        self.blocks as u64
    }

    /// Fyzicka delka v blocich (komprimovany extent je kratsi).
    pub fn plen(&self) -> u64 {
        self.comp.map_or(self.blocks as u64, |c| c.plen as u64)
    }

    pub fn is_unwritten(&self) -> bool {
        self.flags & EXTENT_UNWRITTEN != 0
    }

    pub fn is_shared(&self) -> bool {
        self.flags & EXTENT_SHARED != 0
    }

    pub fn is_compressed(&self) -> bool {
        self.comp.is_some()
    }

    /// Cast `from..to` (relativni logicke bloky) jako samostatny extent.
    /// Komprimovany extent rozdelit nejde (`None`).
    pub fn slice(&self, from: u64, to: u64) -> Option<Self> {
        if self.comp.is_some() || from >= to || to > self.len() {
            return None;
        }
        let csums = if self.is_unwritten() {
            Vec::new()
        } else {
            self.csums[from as usize..to as usize].to_vec()
        };
        Some(Self {
            phys: self.phys + from,
            birth: self.birth,
            csums,
            blocks: (to - from) as u32,
            flags: self.flags,
            copy: if self.copy != 0 { self.copy + from } else { 0 },
            comp: None,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut flags = self.flags & (EXTENT_UNWRITTEN | EXTENT_SHARED);
        if self.copy != 0 {
            flags |= EXTENT_COPY;
        }
        if let Some(c) = self.comp {
            flags |= (c.algo as u32) << COMP_SHIFT;
        }
        let mut b = Vec::with_capacity(EXTENT_HEADER + 16 + self.csums.len() * 16);
        b.extend_from_slice(&self.phys.to_le_bytes());
        b.extend_from_slice(&self.blocks.to_le_bytes());
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&self.birth.to_le_bytes());
        if self.copy != 0 {
            b.extend_from_slice(&self.copy.to_le_bytes());
        }
        if let Some(c) = self.comp {
            b.extend_from_slice(&c.plen.to_le_bytes());
            b.extend_from_slice(&c.bytes.to_le_bytes());
        }
        for c in &self.csums {
            b.extend_from_slice(&c.0);
        }
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        if b.len() < EXTENT_HEADER {
            return Err(Error::Corrupted);
        }
        let blocks = u32_at(b, 8);
        let flags = u32_at(b, 12);
        if flags & !KNOWN != 0 {
            return Err(Error::NotSupported);
        }
        if blocks == 0 || blocks as u64 > MAX_EXTENT {
            return Err(Error::Corrupted);
        }
        let mut o = EXTENT_HEADER;
        let copy = if flags & EXTENT_COPY != 0 {
            let c = b.get(o..o + 8).ok_or(Error::Corrupted)?;
            o += 8;
            let c = u64_at(c, 0);
            if c == 0 {
                return Err(Error::Corrupted);
            }
            c
        } else {
            0
        };
        let comp = match Compress::from_raw(((flags & COMP_MASK) >> COMP_SHIFT) as u8)? {
            None => None,
            Some(algo) => {
                let h = b.get(o..o + 8).ok_or(Error::Corrupted)?;
                o += 8;
                let plen = u32_at(h, 0);
                let bytes = u32_at(h, 4);
                // Kus: nejvys COMP_CHUNK bloku, kratsi nez nekomprimovany,
                // data konci v poslednim fyzickem bloku.
                let ok = blocks as u64 <= COMP_CHUNK
                    && plen >= 1
                    && plen < blocks
                    && bytes as u64 > (plen as u64 - 1) * BLOCK_SIZE as u64
                    && bytes as u64 <= plen as u64 * BLOCK_SIZE as u64;
                if !ok {
                    return Err(Error::Corrupted);
                }
                Some(Comp { algo, plen, bytes })
            }
        };
        let unwritten = flags & EXTENT_UNWRITTEN != 0;
        if unwritten && comp.is_some() {
            return Err(Error::Corrupted);
        }
        let n = if unwritten {
            0
        } else if comp.is_some() {
            1
        } else {
            blocks as usize
        };
        if b.len() != o + n * 16 {
            return Err(Error::Corrupted);
        }
        let csums = (0..n).map(|i| csum_at(b, o + i * 16)).collect();
        Ok(Self {
            phys: u64_at(b, 0),
            birth: u64_at(b, 16),
            csums,
            blocks,
            flags: flags & (EXTENT_UNWRITTEN | EXTENT_SHARED),
            copy,
            comp,
        })
    }
}
