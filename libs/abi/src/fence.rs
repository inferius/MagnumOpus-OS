//! Plot (fence): objekt jadra s casovou osou `u64` pro explicitni
//! synchronizaci mezi procesy (GPU, video, kompozitor).
//!
//! - Hodnota roste monotonne; binarni plot = osa s cilem 1.
//! - Posouvat ji smi jen handle s pravem `SIGNAL` (producent), ostatni
//!   dostanou jen `WAIT` (cekat, dotaz).
//! - Chyba je trvala: po `FENCE_SIGNAL` s [`signal::FAILED`] kazde cekani na
//!   nedosazenou hodnotu skonci `FENCE_FAILED` a dalsi posun je `BAD_STATE`.
//! - Opusteni: kdyz zanikne posledni handle s pravem `SIGNAL` (i ve zprave
//!   kanalu nebo pri padu procesu), kazde cekani na nedosazenou hodnotu
//!   skonci `FENCE_ABANDONED`. Nikdo tak nevisi na plotu mrtveho producenta.
//! - Signal `SIGNALED` (pro `OBJECT_WAIT_*` a port) plati, kdyz hodnota
//!   >= 1, nebo plot skoncil chybou ci byl opusten.
//!
//! Stavovy automat [`Timeline`] je spolecny pro jadro i testy na hostiteli.
//! Popis: `docs/cs/syscally.md` (`FENCE_*`) a `docs/cs/ploty.md`.

use crate::error::{Error, Result};

/// Priznaky `FENCE_SIGNAL`.
pub mod signal {
    /// Plot skoncil chybou: hodnota v argumentu je prvni neuspesny bod
    /// (musi byt vetsi nez dosazena hodnota); dosazena hodnota se nemeni.
    pub const FAILED: u64 = 1 << 0;
    /// Vsechny zname priznaky.
    pub const ALL: u64 = FAILED;
}

/// Priznaky `FENCE_WAIT`.
pub mod wait {
    /// Cekat, az budou splneny vsechny polozky (jinak staci jedna).
    pub const ALL: u64 = 1 << 0;
    /// Vsechny zname priznaky.
    pub const KNOWN: u64 = ALL;
}

/// Polozka `FENCE_WAIT` (16 B): handle plotu a hodnota, na kterou se ceka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct WaitItem {
    pub handle: u32,
    pub _reserved: u32,
    pub value: u64,
}

impl WaitItem {
    pub const SIZE: usize = 16;

    pub const fn new(handle: u32, value: u64) -> Self {
        Self {
            handle,
            _reserved: 0,
            value,
        }
    }

    pub fn encode(&self) -> [u8; 16] {
        let mut b = [0u8; 16];
        b[..4].copy_from_slice(&self.handle.to_le_bytes());
        b[8..].copy_from_slice(&self.value.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; 16]) -> Self {
        Self {
            handle: u32::from_le_bytes(b[..4].try_into().unwrap()),
            _reserved: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            value: u64::from_le_bytes(b[8..].try_into().unwrap()),
        }
    }
}

/// Vystup `FENCE_QUERY` (24 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct Info {
    /// Dosazena hodnota.
    pub value: u64,
    /// Prvni neuspesny bod (`FENCE_SIGNAL` s `FAILED`); 0 = bez chyby.
    pub failed_at: u64,
    /// 0, `FENCE_FAILED` (-37) nebo `FENCE_ABANDONED` (-38).
    pub status: i32,
    pub _reserved: u32,
}

impl Info {
    pub const SIZE: usize = 24;

    pub fn encode(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[..8].copy_from_slice(&self.value.to_le_bytes());
        b[8..16].copy_from_slice(&self.failed_at.to_le_bytes());
        b[16..20].copy_from_slice(&self.status.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; 24]) -> Self {
        Self {
            value: u64::from_le_bytes(b[..8].try_into().unwrap()),
            failed_at: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            status: i32::from_le_bytes(b[16..20].try_into().unwrap()),
            _reserved: 0,
        }
    }

    /// Chyba plotu (`None` = bez chyby).
    pub fn error(&self) -> Option<Error> {
        Error::from_code(self.status as i64)
    }
}

/// Stav casove osy plotu (bez cekajicich; ty drzi jadro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    value: u64,
    /// Prvni neuspesny bod; `Some` = trvala chyba.
    failed_at: Option<u64>,
    /// Zanikl posledni handle s pravem `SIGNAL`.
    abandoned: bool,
}

impl Timeline {
    pub const fn new(initial: u64) -> Self {
        Self {
            value: initial,
            failed_at: None,
            abandoned: false,
        }
    }

    pub const fn value(&self) -> u64 {
        self.value
    }

    /// Vysledek cekani na `target`: `None` = jeste cekat, `Some(Ok)` =
    /// dosazeno, `Some(Err)` = uz nikdy nenastane (chyba, opusteni).
    /// Dosazena hodnota ma prednost: na bod pred chybou se uspesne doceka.
    pub const fn check(&self, target: u64) -> Option<Result<()>> {
        if self.value >= target {
            Some(Ok(()))
        } else if self.failed_at.is_some() {
            Some(Err(Error::FenceFailed))
        } else if self.abandoned {
            Some(Err(Error::FenceAbandoned))
        } else {
            None
        }
    }

    /// Posun (`FENCE_SIGNAL`). Bez `failed`: `value` >= dosazene hodnoty
    /// (rovna = nic). S `failed`: `value` = prvni neuspesny bod, vetsi nez
    /// dosazena hodnota. Po chybe uz posun nejde (`BAD_STATE`).
    pub fn signal(&mut self, value: u64, failed: bool) -> Result<()> {
        if self.failed_at.is_some() {
            return Err(Error::BadState);
        }
        if failed {
            if value <= self.value {
                return Err(Error::InvalidArgs);
            }
            self.failed_at = Some(value);
            return Ok(());
        }
        if value < self.value {
            return Err(Error::InvalidArgs);
        }
        self.value = value;
        Ok(())
    }

    /// Zanikl posledni handle s pravem `SIGNAL`.
    pub fn abandon(&mut self) {
        self.abandoned = true;
    }

    /// Trvala chyba plotu, ktera dopadne na nedosazene hodnoty.
    pub const fn status(&self) -> Option<Error> {
        if self.failed_at.is_some() {
            Some(Error::FenceFailed)
        } else if self.abandoned {
            Some(Error::FenceAbandoned)
        } else {
            None
        }
    }

    /// Plati signal `SIGNALED` (binarni pohled: hodnota >= 1 nebo konec).
    pub const fn signaled(&self) -> bool {
        self.value >= 1 || self.failed_at.is_some() || self.abandoned
    }

    pub fn info(&self) -> Info {
        Info {
            value: self.value,
            failed_at: self.failed_at.unwrap_or(0),
            status: self.status().map_or(0, |e| e.code()),
            _reserved: 0,
        }
    }
}
