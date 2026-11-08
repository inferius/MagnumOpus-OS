//! Soubory WAV (RIFF): cteni PCM 8/16/24/32 bitu a float 32, zapis
//! hlavicky 16bitoveho PCM.
//!
//! Neznama a nadbytecna data (LIST, fact, ...) se preskoci. Delka dat
//! vetsi nez soubor (nedokonceny zapis, napr. QEMU pri preruseni) se
//! zkrati na to, co v souboru opravdu je.

use alloc::vec::Vec;

use audioproto::{Format, Sample};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WavError {
    NotRiff,
    /// Chybi `fmt ` nebo `data`.
    Missing,
    /// Format, ktery neumime (komprese, neobvykla bitova hloubka).
    Unsupported,
}

/// Rozebrany soubor: format a data (cele snimky).
#[derive(Debug, PartialEq, Eq)]
pub struct Wav<'a> {
    pub format: Format,
    pub data: &'a [u8],
}

const PCM: u16 = 1;
const FLOAT: u16 = 3;
const EXTENSIBLE: u16 = 0xFFFE;

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub fn parse(b: &[u8]) -> Result<Wav<'_>, WavError> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(WavError::NotRiff);
    }
    let mut fmt: Option<Format> = None;
    let mut pos = 12;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let len = u32_at(b, pos + 4).ok_or(WavError::Missing)? as usize;
        let body = pos + 8;
        let end = body.saturating_add(len).min(b.len());
        if id == b"fmt " {
            fmt = Some(parse_fmt(&b[body..end])?);
        } else if id == b"data" {
            let f = fmt.ok_or(WavError::Missing)?;
            let data = &b[body..end];
            let whole = data.len() / f.frame_bytes() as usize * f.frame_bytes() as usize;
            return Ok(Wav { format: f, data: &data[..whole] });
        }
        // Kusy maji sudou delku (vypln).
        pos = body.saturating_add(len).saturating_add(len & 1);
    }
    Err(WavError::Missing)
}

fn parse_fmt(f: &[u8]) -> Result<Format, WavError> {
    let mut tag = u16_at(f, 0).ok_or(WavError::Missing)?;
    let channels = u16_at(f, 2).ok_or(WavError::Missing)?;
    let rate = u32_at(f, 4).ok_or(WavError::Missing)?;
    let block = u16_at(f, 12).ok_or(WavError::Missing)?;
    let bits = u16_at(f, 14).ok_or(WavError::Missing)?;
    if tag == EXTENSIBLE {
        // Podformat: prvni dva bajty GUID = kod formatu.
        tag = u16_at(f, 24).ok_or(WavError::Missing)?;
    }
    let sample = match (tag, bits, block.checked_div(channels)) {
        (PCM, 8, Some(1)) => Sample::U8,
        (PCM, 16, Some(2)) => Sample::S16,
        (PCM, 24, Some(3)) => return Err(WavError::Unsupported), // 3 bajty: zatim ne
        (PCM, 24, Some(4)) => Sample::S24,
        (PCM, 32, Some(4)) => Sample::S32,
        (FLOAT, 32, Some(4)) => Sample::F32,
        _ => return Err(WavError::Unsupported),
    };
    if channels > u8::MAX as u16 {
        return Err(WavError::Unsupported);
    }
    let format = Format::new(rate, channels as u8, sample);
    if !format.valid() {
        return Err(WavError::Unsupported);
    }
    Ok(format)
}

/// Hlavicka WAV (44 bajtu) pro 16bitove PCM s `data_len` bajty dat.
pub fn header_s16(rate: u32, channels: u16, data_len: u32) -> Vec<u8> {
    let block = channels * 2;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data_len).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&PCM.to_le_bytes());
    h.extend_from_slice(&channels.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * block as u32).to_le_bytes());
    h.extend_from_slice(&block.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data_len.to_le_bytes());
    h
}
