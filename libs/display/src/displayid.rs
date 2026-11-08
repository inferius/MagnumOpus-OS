//! DisplayID 1.3 a 2.0 (VESA): samostatna struktura nebo rozsireni EDID
//! (znacka 0x70). Cteme casovani typu I (1.x, blok 0x03) a typu VII
//! (2.0, blok 0x22) a jmeno produktu (bloky 0x00 a 0x20).
//!
//! ```text
//! sekce:  verze | delka dat N | typ produktu | pocet rozsireni | N B bloku | soucet
//! blok:   znacka | revize | delka L | L B dat
//! casovani (20 B): clock-1 (3 B) | priznaky | hact-1 | hblank-1 | hfront-1 (bit 15 = +hsync)
//!                  | hsync-1 | vact-1 | vblank-1 | vfront-1 (bit 15 = +vsync) | vsync-1
//! ```
//!
//! Typ I ma pixel clock v jednotkach 10 kHz, typ VII v kHz (stejne jako
//! `drm_displayid.c` / `drm_edid.c`).

use alloc::string::String;
use alloc::vec::Vec;

use crate::timing::{Timing, flags};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayId {
    /// Verze a revize (0x12 = 1.2, 0x13 = 1.3, 0x20 = 2.0).
    pub version: u8,
    pub product_type: u8,
    pub name: Option<String>,
    pub timings: Vec<Timing>,
    /// Pocet znamych, ale necteni bloku (dlazdice, parametry, ...).
    pub skipped_blocks: u32,
}

impl DisplayId {
    /// Rozebere sekci DisplayID (od bajtu verze; u rozsireni EDID bajty
    /// 1..127 bloku).
    pub fn parse(s: &[u8]) -> Result<Self> {
        if s.len() < 5 {
            return Err(Error::Truncated);
        }
        let len = s[1] as usize;
        let end = 4 + len;
        if end >= s.len() {
            return Err(Error::Truncated);
        }
        if s[..=end].iter().fold(0u8, |a, &b| a.wrapping_add(b)) != 0 {
            return Err(Error::BadChecksum);
        }
        let mut d = DisplayId {
            version: s[0],
            product_type: s[2],
            name: None,
            timings: Vec::new(),
            skipped_blocks: 0,
        };
        let mut i = 4;
        while i + 3 <= end {
            let (tag, blen) = (s[i], s[i + 2] as usize);
            // Vyplne nulami za poslednim blokem.
            if tag == 0 && blen == 0 && s[i + 1] == 0 {
                break;
            }
            let p = s.get(i + 3..i + 3 + blen).ok_or(Error::Truncated)?;
            if i + 3 + blen > end {
                return Err(Error::Truncated);
            }
            match tag {
                0x03 => d.timings.extend(p.chunks_exact(20).filter_map(|t| timing(t, false))),
                0x22 => d.timings.extend(p.chunks_exact(20).filter_map(|t| timing(t, true))),
                0x00 | 0x20 => d.name = product_name(p),
                _ => d.skipped_blocks += 1,
            }
            i += 3 + blen;
        }
        Ok(d)
    }
}

fn le16(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

/// Detailni casovani typu I (`kilohertz = false`) nebo VII.
pub fn timing(b: &[u8], kilohertz: bool) -> Option<Timing> {
    let raw = b[0] as u32 | (b[1] as u32) << 8 | (b[2] as u32) << 16;
    let clock_khz = if kilohertz { raw + 1 } else { (raw + 1) * 10 };
    let f = b[3];
    let hactive = le16(&b[4..]) + 1;
    let hblank = le16(&b[6..]) + 1;
    let hfront_raw = le16(&b[8..]);
    let hfront = (hfront_raw & 0x7FFF) + 1;
    let hsync = le16(&b[10..]) + 1;
    let vactive = le16(&b[12..]) + 1;
    let vblank = le16(&b[14..]) + 1;
    let vfront_raw = le16(&b[16..]);
    let vfront = (vfront_raw & 0x7FFF) + 1;
    let vsync = le16(&b[18..]) + 1;
    if hblank < hfront + hsync || vblank < vfront + vsync {
        return None;
    }
    let mut fl = 0;
    if hfront_raw & 0x8000 != 0 {
        fl |= flags::HSYNC_POS;
    }
    if vfront_raw & 0x8000 != 0 {
        fl |= flags::VSYNC_POS;
    }
    if f & 0x80 != 0 {
        fl |= flags::PREFERRED;
    }
    if f & 0x10 != 0 {
        fl |= flags::INTERLACED;
    }
    Some(Timing {
        clock_khz,
        hactive,
        hfront,
        hsync,
        hback: hblank - hfront - hsync,
        vactive,
        vfront,
        vsync,
        vback: vblank - vfront - vsync,
        flags: fl,
    })
}

/// Zakoduje casovani do 20 B deskriptoru (testy a simulace monitoru).
pub fn encode_timing(t: &Timing, kilohertz: bool) -> [u8; 20] {
    let mut b = [0u8; 20];
    let raw = if kilohertz { t.clock_khz - 1 } else { t.clock_khz / 10 - 1 };
    b[0..3].copy_from_slice(&raw.to_le_bytes()[..3]);
    b[3] = if t.preferred() { 0x80 } else { 0 } | if t.interlaced() { 0x10 } else { 0 };
    let put = |b: &mut [u8; 20], at: usize, v: u16| b[at..at + 2].copy_from_slice(&v.to_le_bytes());
    put(&mut b, 4, t.hactive - 1);
    put(&mut b, 6, t.hfront + t.hsync + t.hback - 1);
    put(&mut b, 8, (t.hfront - 1) | if t.flags & flags::HSYNC_POS != 0 { 0x8000 } else { 0 });
    put(&mut b, 10, t.hsync - 1);
    put(&mut b, 12, t.vactive - 1);
    put(&mut b, 14, t.vfront + t.vsync + t.vback - 1);
    put(&mut b, 16, (t.vfront - 1) | if t.flags & flags::VSYNC_POS != 0 { 0x8000 } else { 0 });
    put(&mut b, 18, t.vsync - 1);
    b
}

/// Jmeno z bloku identifikace produktu: id (3 B), kod (2), seriove cislo
/// (4), tyden, rok, delka jmena, jmeno.
fn product_name(p: &[u8]) -> Option<String> {
    let n = *p.get(11)? as usize;
    let name = p.get(12..12 + n)?;
    let s: String = name.iter().map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '?' }).collect();
    let s = String::from(s.trim());
    if s.is_empty() { None } else { Some(s) }
}
