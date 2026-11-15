//! Testovaci format medii `MTM1` pro dekoder bez FFmpeg (`mediatest`).
//!
//! Soubor = hlavicka ([`HEADER_LEN`] B) + zaznam kazdeho snimku
//! ([`RECORD_LEN`] B: pts, seminko obrazu, priznaky). Dekoder cte zaznamy
//! pres protokol zdroje, takze cesta bajtu (aplikace -> `mediad` ->
//! dekoder) je skutecne pouzita; obraz a ton generuje z udaju souboru.
//! Obraz je deterministicky: test overi obsah bufferu kontrolnim souctem.
//!
//! Hlavicka: magie "MTM1", verze u32, sirka u32, vyska u32, fps citatel
//! u32, jmenovatel u32, snimky u32, frekvence zvuku u32 (0 = bez zvuku),
//! kanaly u8 + 3 B vyplne, ton Hz u32, pad u32 (cislo snimku + 1, na kterem
//! dekoder schvalne spadne; 0 = nikdy), zbytek nuly.

use alloc::vec::Vec;

use crate::{Error, Info, Kind, Result, Stream};
use wire::{Reader, Writer};

pub const MAGIC: &[u8; 4] = b"MTM1";
pub const VERSION: u32 = 1;
pub const HEADER_LEN: u64 = 64;
pub const RECORD_LEN: u64 = 16;
/// Nejvetsi rozmer testovaciho videa.
pub const MAX_DIM: u32 = 4096;
/// Priznak zaznamu: klicovy snimek (cil `SEEK` bez `exact`).
pub const KEY: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub frames: u32,
    pub rate: u32,
    pub channels: u8,
    pub tone_hz: u32,
    pub crash_at: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub pts_us: u64,
    pub seed: u32,
    pub flags: u32,
}

impl Header {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.bytes(MAGIC).u32(VERSION).u32(self.width).u32(self.height);
        w.u32(self.fps_num).u32(self.fps_den).u32(self.frames).u32(self.rate);
        w.u8(self.channels).u8(0).u8(0).u8(0).u32(self.tone_hz).u32(self.crash_at);
        let mut v = w.finish();
        v.resize(HEADER_LEN as usize, 0);
        v
    }

    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < HEADER_LEN as usize || &b[..4] != MAGIC {
            return Err(Error::NotSupported);
        }
        let mut r = Reader::new(&b[4..HEADER_LEN as usize]);
        if r.u32()? != VERSION {
            return Err(Error::NotSupported);
        }
        let h = Header {
            width: r.u32()?,
            height: r.u32()?,
            fps_num: r.u32()?,
            fps_den: r.u32()?,
            frames: r.u32()?,
            rate: r.u32()?,
            channels: {
                let c = r.u8()?;
                r.take(3)?;
                c
            },
            tone_hz: r.u32()?,
            crash_at: r.u32()?,
        };
        let even = |v: u32| v != 0 && v % 2 == 0 && v <= MAX_DIM;
        if !even(h.width) || !even(h.height) || h.fps_num == 0 || h.fps_den == 0 || h.frames == 0 {
            return Err(Error::Corrupted);
        }
        if h.rate != 0 && (h.channels == 0 || h.channels > 8 || h.rate > 384_000) {
            return Err(Error::Corrupted);
        }
        Ok(h)
    }

    /// Trvani snimku v us.
    pub fn frame_us(&self) -> u32 {
        (1_000_000u64 * self.fps_den as u64 / self.fps_num as u64) as u32
    }

    /// Delka v us.
    pub fn duration_us(&self) -> u64 {
        self.frames as u64 * 1_000_000 * self.fps_den as u64 / self.fps_num as u64
    }

    /// Velikost souboru.
    pub fn file_len(&self) -> u64 {
        HEADER_LEN + self.frames as u64 * RECORD_LEN
    }

    pub fn info(&self) -> Info {
        let mut streams = alloc::vec![Stream {
            kind: Kind::Video,
            codec: "mtm-pattern".into(),
            width: self.width,
            height: self.height,
            fps_num: self.fps_num,
            fps_den: self.fps_den,
            rate: 0,
            channels: 0,
            lang: Default::default(),
            available: true,
        }];
        if self.rate != 0 {
            streams.push(Stream {
                kind: Kind::Audio,
                codec: "mtm-tone".into(),
                width: 0,
                height: 0,
                fps_num: 0,
                fps_den: 0,
                rate: self.rate,
                channels: self.channels,
                lang: Default::default(),
                available: true,
            });
        }
        Info { duration_us: self.duration_us(), streams }
    }
}

impl Record {
    pub fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let rec = Record { pts_us: r.u64()?, seed: r.u32()?, flags: r.u32()? };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(rec)
    }
}

/// Posun zaznamu snimku `i`.
pub fn record_offset(i: u32) -> u64 {
    HEADER_LEN + i as u64 * RECORD_LEN
}

/// Seminko obrazu snimku `i` (promichane cislo snimku).
pub fn seed(i: u32) -> u32 {
    let mut x = i.wrapping_mul(0x9E37_79B9) ^ 0x5bd1_e995;
    x ^= x >> 15;
    x.wrapping_mul(0x2c1b_3c6d)
}

/// Sestavi cely testovaci soubor.
pub fn build(h: &Header) -> Vec<u8> {
    let mut v = h.encode();
    let key_every = (h.fps_num / h.fps_den).max(1);
    for i in 0..h.frames {
        let mut w = Writer::new();
        let pts = i as u64 * 1_000_000 * h.fps_den as u64 / h.fps_num as u64;
        w.u64(pts).u32(seed(i)).u32(if i % key_every == 0 { KEY } else { 0 });
        v.extend_from_slice(&w.finish());
    }
    v
}

/// Vyplni snimek NV12: vodorovny prechod jasu, svisly pruh na pozici
/// podle seminka (pohyb) a barva chroma podle seminka. `y` ma radky po
/// `y_pitch`, `uv` (U a V proplete) po `uv_pitch`.
pub fn fill_nv12(w: u32, h: u32, y: &mut [u8], y_pitch: usize, uv: &mut [u8], uv_pitch: usize, seed: u32) {
    let (w, h) = (w as usize, h as usize);
    let bar_w = (w / 16).max(2);
    let bar_x = seed as usize % (w - bar_w + 1);
    for row in 0..h {
        let line = &mut y[row * y_pitch..row * y_pitch + w];
        for (x, p) in line.iter_mut().enumerate() {
            *p = if x >= bar_x && x < bar_x + bar_w { 235 } else { 16 + (x * 219 / w) as u8 };
        }
    }
    let u = 128u8.wrapping_add(((seed >> 8) & 0x3f) as u8).wrapping_sub(32);
    let v = 128u8.wrapping_add(((seed >> 16) & 0x3f) as u8).wrapping_sub(32);
    for row in 0..h / 2 {
        let line = &mut uv[row * uv_pitch..row * uv_pitch + w];
        for pair in line.chunks_exact_mut(2) {
            pair[0] = u;
            pair[1] = v;
        }
    }
}

/// Kontrolni soucet viditelnych bajtu snimku NV12 (FNV-1a, bez vyplne
/// radku).
pub fn checksum_nv12(w: u32, h: u32, y: &[u8], y_pitch: usize, uv: &[u8], uv_pitch: usize) -> u64 {
    let (w, h) = (w as usize, h as usize);
    let mut s = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: &[u8]| {
        for &x in b {
            s ^= x as u64;
            s = s.wrapping_mul(0x100_0000_01b3);
        }
    };
    for row in 0..h {
        eat(&y[row * y_pitch..row * y_pitch + w]);
    }
    for row in 0..h / 2 {
        eat(&uv[row * uv_pitch..row * uv_pitch + w]);
    }
    s
}
