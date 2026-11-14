//! Protokol zvuku: aplikace <-> zvukova sluzba `audiod` <-> ovladac
//! zvukove karty (`user/hda`).
//!
//! - [`Format`]: format PCM (vzorkovaci frekvence, kanaly, typ vzorku),
//! - [`ring`]: sdileny kruhovy buffer v objektu pameti (VMO) - data mezi
//!   aplikaci a sluzbou nejdou pres kanal, jen pozice (bez kopirovani),
//! - [`client`]: zpravy aplikace <-> `audiod` (stream, hlasitost, stav),
//! - [`device`]: zpravy `audiod` <-> ovladac a stranka stavu DMA,
//! - [`conn`] (feature `client`): klient nad kanaly pro programy.
//!
//! Zpravy pouzivaji kodovani `wire` (little-endian). Cteni neplatne zpravy
//! vraci `INVALID_ARGS`, nikdy paniku: sluzba cte zpravy od kohokoli.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod session_tests;

pub mod client;
pub mod device;
pub mod ring;
pub mod session;

#[cfg(feature = "client")]
pub mod conn;

use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

/// Typ vzorku PCM (little-endian, se znamenkem krome `U8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Sample {
    U8 = 1,
    S16 = 2,
    /// 24 bitu v nizsich bitech 32bitoveho slova (znamenko rozsirene).
    S24 = 3,
    S32 = 4,
    F32 = 5,
}

impl Sample {
    pub const ALL: [Sample; 5] = [Sample::U8, Sample::S16, Sample::S24, Sample::S32, Sample::F32];

    pub fn from_u8(v: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| *s as u8 == v)
    }

    /// Bajtu na jeden vzorek.
    pub const fn bytes(self) -> u32 {
        match self {
            Sample::U8 => 1,
            Sample::S16 => 2,
            Sample::S24 | Sample::S32 | Sample::F32 => 4,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Sample::U8 => "u8",
            Sample::S16 => "s16",
            Sample::S24 => "s24",
            Sample::S32 => "s32",
            Sample::F32 => "f32",
        }
    }
}

/// Nejnizsi a nejvyssi podporovana vzorkovaci frekvence (Hz).
pub const MIN_RATE: u32 = 8_000;
pub const MAX_RATE: u32 = 192_000;
/// Nejvic kanalu streamu (vic nez 2 se smichaji do sterea).
pub const MAX_CHANNELS: u8 = 8;

/// Format PCM: vzorky kanalu jsou prokladane (L, R, L, R, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    pub rate: u32,
    pub channels: u8,
    pub sample: Sample,
}

impl Format {
    /// Vychozi format zarizeni: 48 kHz, stereo, 16 bitu (umi ho kazdy kodek HDA).
    pub const DEVICE_DEFAULT: Format = Format { rate: 48_000, channels: 2, sample: Sample::S16 };

    pub const fn new(rate: u32, channels: u8, sample: Sample) -> Self {
        Self { rate, channels, sample }
    }

    /// Bajtu na snimek (jeden vzorek kazdeho kanalu).
    pub const fn frame_bytes(&self) -> u32 {
        self.channels as u32 * self.sample.bytes()
    }

    /// Je format v mezich protokolu?
    pub fn valid(&self) -> bool {
        (MIN_RATE..=MAX_RATE).contains(&self.rate) && (1..=MAX_CHANNELS).contains(&self.channels)
    }

    /// Snimky za `us` mikrosekund (zaokrouhleno nahoru).
    pub fn frames_for_us(&self, us: u64) -> u64 {
        (us * self.rate as u64).div_ceil(1_000_000)
    }

    /// Doba `frames` snimku v mikrosekundach.
    pub fn us_for_frames(&self, frames: u64) -> u64 {
        frames * 1_000_000 / self.rate as u64
    }

    pub fn encode(&self, w: &mut Writer) {
        w.u32(self.rate).u8(self.channels).u8(self.sample as u8);
    }

    pub fn decode(r: &mut Reader) -> Result<Self> {
        let rate = r.u32()?;
        let channels = r.u8()?;
        let sample = Sample::from_u8(r.u8()?).ok_or(Error::InvalidArgs)?;
        let f = Self { rate, channels, sample };
        if !f.valid() {
            return Err(Error::InvalidArgs);
        }
        Ok(f)
    }
}

/// Smer streamu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Direction {
    /// Prehravani: aplikace pise do bufferu, sluzba cte.
    Output = 0,
    /// Nahravani: sluzba pise, aplikace cte.
    Input = 1,
}

impl Direction {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Output),
            1 => Some(Self::Input),
            _ => None,
        }
    }
}

/// Hlasitost v procentech (0..=100) -> zesileni Q16 (65536 = 0 dB).
///
/// Krivka je kubicka (procenta^3): pribliz vnimane hlasitosti, 50 % je
/// asi -18 dB, 10 % -60 dB. Linearni krivka by mela vetsinu rozsahu
/// "skoro naplno".
pub const fn volume_gain(percent: u8) -> u32 {
    let p = if percent > 100 { 100 } else { percent } as u64;
    (p * p * p * 65_536 / 1_000_000) as u32
}

/// Odpoved: stav (0 = uspech, jinak kod chyby) a data.
pub fn encode_reply(r: Result<Vec<u8>>) -> Vec<u8> {
    let mut w = Writer::new();
    match r {
        Ok(data) => {
            w.i32(0).bytes(&data);
        }
        Err(e) => {
            w.i32(e.code());
        }
    }
    w.finish()
}

/// Rozlozi odpoved na data, nebo chybu.
pub fn decode_reply(m: &[u8]) -> Result<&[u8]> {
    let mut r = Reader::new(m);
    let status = r.i32().map_err(|_| Error::Corrupted)?;
    if status != 0 {
        return Err(Error::from_code(status as i64).unwrap_or(Error::Corrupted));
    }
    Ok(r.rest())
}
