//! Protokol v2: data (faze 3 FS) - okamzite kopie, predalokace, nastaveni
//! dat datasetu (komprese, kopie, deduplikace) a scrub.
//!
//! Cesty jsou cesty relace. Prava kontroluje FS: kopie = cist zdroj
//! a vytvorit/zapsat cil, predalokace = zapis, nastaveni dat = `MANAGE`
//! na datasetu, scrub = `MANAGE` na koreni svazku ("/").

use alloc::string::String;
use alloc::vec::Vec;

use libos::time::WallTime;
use libos::{Error, Result};

use crate::{FsClient, Reader, Writer, get_time, put_time};

/// Kody operaci (navazuji na `hist::op`; 87-89 patri vnorenym datasetum).
pub mod op {
    /// zdroj str, cil str -> sdilenych bajtu u64 (soubor, odkaz, slozka)
    pub const CLONE: u32 = 90;
    /// zdroj str, posun u64, cil str, posun u64, delka u64
    pub const CLONE_RANGE: u32 = 91;
    /// cesta str, posun u64, delka u64, zachovat velikost u8
    pub const FALLOCATE: u32 = 92;
    /// dataset str -> `DataSettings`
    pub const DATA_SETTINGS: u32 = 93;
    /// dataset str, `DataSettings`
    pub const SET_DATA_SETTINGS: u32 = 94;
    /// (nic) - spusti scrub (zastaveny pokracuje)
    pub const SCRUB_START: u32 = 95;
    /// (nic) - zastavi scrub (pozice zustane)
    pub const SCRUB_STOP: u32 = 96;
    /// (nic) -> `ScrubStatus`
    pub const SCRUB_STATUS: u32 = 97;
}

/// Komprese (`DataSettings::compress`).
pub mod compress {
    pub const NONE: u8 = 0;
    pub const LZ4: u8 = 1;
    pub const ZSTD: u8 = 2;
}

/// Deduplikace (`DataSettings::dedup`).
pub mod dedup {
    pub const OFF: u8 = 0;
    pub const BACKGROUND: u8 = 1;
    pub const INLINE: u8 = 2;
}

/// Nastaveni dat datasetu: komprese (u8), kopii dat (u8, 1-2),
/// deduplikace (u8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataSettings {
    pub compress: u8,
    pub copies: u8,
    pub dedup: u8,
}

impl DataSettings {
    pub fn put(&self, w: &mut Writer) {
        w.u8(self.compress).u8(self.copies).u8(self.dedup);
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            compress: r.u8()?,
            copies: r.u8()?,
            dedup: r.u8()?,
        })
    }
}

/// Stav scrubu.
///
/// ```text
/// stav u8 (0 neni, 1 bezi, 2 zastaveny), zacatek (cas), konec posledniho
/// dokonceneho (cas), prectenych bloku u64, spatnych kopii u64,
/// opraveno u64, neopravitelnych u64, pocet u32 + chyby (str)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrubStatus {
    pub state: u8,
    pub started: WallTime,
    pub finished: WallTime,
    pub scanned: u64,
    pub bad: u64,
    pub repaired: u64,
    pub lost: u64,
    pub errors: Vec<String>,
}

pub const SCRUB_IDLE: u8 = 0;
pub const SCRUB_RUNNING: u8 = 1;
pub const SCRUB_STOPPED: u8 = 2;

impl ScrubStatus {
    pub fn put(&self, w: &mut Writer) {
        w.u8(self.state);
        put_time(w, self.started);
        put_time(w, self.finished);
        w.u64(self.scanned).u64(self.bad).u64(self.repaired).u64(self.lost);
        w.u32(self.errors.len() as u32);
        for e in &self.errors {
            w.str(e);
        }
    }

    pub fn get(r: &mut Reader) -> Result<Self> {
        let state = r.u8()?;
        let started = get_time(r)?;
        let finished = get_time(r)?;
        let (scanned, bad, repaired, lost) = (r.u64()?, r.u64()?, r.u64()?, r.u64()?);
        let n = r.u32()? as usize;
        if n > crate::hist::MAX_ITEMS {
            return Err(Error::InvalidArgs);
        }
        let errors = (0..n).map(|_| r.str()).collect::<Result<_>>()?;
        Ok(Self {
            state,
            started,
            finished,
            scanned,
            bad,
            repaired,
            lost,
            errors,
        })
    }
}

impl FsClient {
    /// Okamzita kopie souboru, odkazu nebo slozky (sdileni bloku).
    pub fn clone_path(&self, src: &str, dst: &str) -> Result<u64> {
        let mut w = Self::req(op::CLONE);
        w.str(src).str(dst);
        Reader::new(&self.call(w)?).u64()
    }

    /// Okamzita kopie rozsahu (posuny a delka po 4 KiB, delka smi koncit
    /// na konci zdroje).
    pub fn clone_range(&self, src: &str, src_off: u64, dst: &str, dst_off: u64, len: u64) -> Result<()> {
        let mut w = Self::req(op::CLONE_RANGE);
        w.str(src).u64(src_off).str(dst).u64(dst_off).u64(len);
        self.call(w).map(|_| ())
    }

    /// Predalokace (jako fallocate).
    pub fn fallocate(&self, path: &str, offset: u64, len: u64, keep_size: bool) -> Result<()> {
        let mut w = Self::req(op::FALLOCATE);
        w.str(path).u64(offset).u64(len).u8(keep_size as u8);
        self.call(w).map(|_| ())
    }

    pub fn data_settings(&self, dataset: &str) -> Result<DataSettings> {
        DataSettings::get(&mut Reader::new(&self.path_op(op::DATA_SETTINGS, dataset)?))
    }

    pub fn set_data_settings(&self, dataset: &str, s: &DataSettings) -> Result<()> {
        let mut w = Self::req(op::SET_DATA_SETTINGS);
        w.str(dataset);
        s.put(&mut w);
        self.call(w).map(|_| ())
    }

    pub fn scrub_start(&self) -> Result<()> {
        self.call(Self::req(op::SCRUB_START)).map(|_| ())
    }

    pub fn scrub_stop(&self) -> Result<()> {
        self.call(Self::req(op::SCRUB_STOP)).map(|_| ())
    }

    pub fn scrub_status(&self) -> Result<ScrubStatus> {
        ScrubStatus::get(&mut Reader::new(&self.call(Self::req(op::SCRUB_STATUS))?))
    }
}
