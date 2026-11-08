//! Rozsireni CTA-861 (znacka 0x02): datove bloky (video, audio,
//! reproduktory, vyrobce: HDMI 1.4 VSDB a HDMI Forum HF-VSDB) a dalsi
//! detailni casovani. Kody VIC prevadi na casovani [`vic`].
//!
//! ```text
//! 0      znacka 0x02
//! 1      revize (3 = CTA-861-B a novejsi)
//! 2      offset prvniho DTD (d); 0 = zadne DTD ani datove bloky
//! 3      bit 7 underscan, 6 zakladni audio, 5 YCbCr 4:4:4, 4 YCbCr 4:2:2,
//!        3-0 pocet nativnich DTD
//! 4..d   datove bloky: hlavicka (znacka << 5 | delka), data
//! d..127 DTD po 18 B, 127 kontrolni soucet
//! ```

use alloc::vec::Vec;

use crate::edid::{Detailed, parse_dtd};
use crate::timing::{Timing, flags};
use crate::{Error, Result};

/// OUI HDMI Licensing (HDMI 1.4 VSDB), v datech little-endian.
pub const OUI_HDMI: u32 = 0x000C03;
/// OUI HDMI Forum (HF-VSDB, HDMI 2.x).
pub const OUI_HDMI_FORUM: u32 = 0xC45DD8;

/// Kratky deskriptor videa (SVD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Svd {
    pub vic: u8,
    /// Nativni rezim (bit 7 u VIC 1 az 64).
    pub native: bool,
}

/// HDMI 1.4 VSDB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hdmi {
    /// Fyzicka adresa CEC (napr. 0x1000 = 1.0.0.0).
    pub phys_addr: u16,
    /// Hluboka barva: 30, 36, 48 bitu na pixel.
    pub dc30: bool,
    pub dc36: bool,
    pub dc48: bool,
    pub dc_y444: bool,
    /// Nejvyssi TMDS clock v kHz (0 = neuvedeno, pak 165 MHz).
    pub max_tmds_khz: u32,
}

/// HDMI Forum VSDB (HDMI 2.0+).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdmiForum {
    pub version: u8,
    /// Nejvyssi rychlost znaku TMDS v kHz (0 = jen do 340 MHz).
    pub max_tmds_char_khz: u32,
    /// Prijemce ma SCDC (stav a scrambling pres DDC).
    pub scdc: bool,
    /// Scrambling i pod 340 MHz.
    pub lte_340_scramble: bool,
}

/// Kratky deskriptor audia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sad {
    /// Format (1 = LPCM).
    pub format: u8,
    pub channels: u8,
    /// Bitova mapa vzorkovacich frekvenci (bit 0 = 32 kHz ... bit 6 = 192 kHz).
    pub rates: u8,
    /// LPCM: bitova mapa sirek vzorku (bit 0 = 16, 1 = 20, 2 = 24 bitu).
    pub detail: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cta {
    pub revision: u8,
    pub underscan: bool,
    pub basic_audio: bool,
    pub ycbcr444: bool,
    pub ycbcr422: bool,
    pub native_dtds: u8,
    pub svds: Vec<Svd>,
    pub audio: Vec<Sad>,
    /// Bitova mapa alokace reproduktoru (0 = blok chybi).
    pub speakers: u32,
    pub hdmi: Option<Hdmi>,
    pub hdmi_forum: Option<HdmiForum>,
    /// VIC, ktere umi YCbCr 4:2:0 (blok 0x0E).
    pub y420_only: Vec<u8>,
    pub detailed: Vec<Detailed>,
}

impl Cta {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 128 || b[0] != 0x02 {
            return Err(Error::BadHeader);
        }
        let d = b[2] as usize;
        if d != 0 && !(4..=127).contains(&d) {
            return Err(Error::Invalid);
        }
        let mut c = Cta {
            revision: b[1],
            underscan: b[3] & 0x80 != 0,
            basic_audio: b[3] & 0x40 != 0,
            ycbcr444: b[3] & 0x20 != 0,
            ycbcr422: b[3] & 0x10 != 0,
            native_dtds: b[3] & 0x0F,
            svds: Vec::new(),
            audio: Vec::new(),
            speakers: 0,
            hdmi: None,
            hdmi_forum: None,
            y420_only: Vec::new(),
            detailed: Vec::new(),
        };
        if d == 0 {
            return Ok(c);
        }
        // Datove bloky jen u revize 3+.
        let mut i = 4;
        while c.revision >= 3 && i < d {
            let tag = b[i] >> 5;
            let len = (b[i] & 0x1F) as usize;
            let Some(p) = b.get(i + 1..i + 1 + len) else {
                return Err(Error::Truncated);
            };
            if i + 1 + len > d {
                return Err(Error::Truncated);
            }
            c.data_block(tag, p);
            i += 1 + len;
        }
        // DTD od `d` po 18 B, dokud nema nulovy pixel clock.
        let mut j = d;
        while j + 18 <= 127 {
            match parse_dtd(&b[j..j + 18]) {
                Some(t) => c.detailed.push(t),
                None => break,
            }
            j += 18;
        }
        Ok(c)
    }

    fn data_block(&mut self, tag: u8, p: &[u8]) {
        match tag {
            1 => {
                for s in p.chunks_exact(3) {
                    self.audio.push(Sad {
                        format: (s[0] >> 3) & 0x0F,
                        channels: (s[0] & 7) + 1,
                        rates: s[1] & 0x7F,
                        detail: s[2],
                    });
                }
            }
            2 => {
                for &v in p {
                    // VIC 129-192 s bitem 7 = nativni VIC 1-64; 193+ = 8bitove VIC.
                    let (vic, native) = if (129..=192).contains(&v) { (v & 0x7F, true) } else { (v, false) };
                    self.svds.push(Svd { vic, native });
                }
            }
            3 if p.len() >= 3 => {
                let oui = p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16;
                if oui == OUI_HDMI && p.len() >= 5 {
                    let get = |k: usize| p.get(k).copied().unwrap_or(0);
                    let dc = get(5);
                    self.hdmi = Some(Hdmi {
                        phys_addr: u16::from_be_bytes([p[3], p[4]]),
                        dc48: dc & 0x40 != 0,
                        dc36: dc & 0x20 != 0,
                        dc30: dc & 0x10 != 0,
                        dc_y444: dc & 0x08 != 0,
                        max_tmds_khz: get(6) as u32 * 5_000,
                    });
                } else if oui == OUI_HDMI_FORUM && p.len() >= 6 {
                    self.hdmi_forum = Some(HdmiForum {
                        version: p[3],
                        max_tmds_char_khz: p[4] as u32 * 5_000,
                        scdc: p[5] & 0x80 != 0,
                        lte_340_scramble: p[5] & 0x08 != 0,
                    });
                }
            }
            4 if p.len() >= 3 => self.speakers = p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16,
            // Rozsirena znacka v prvnim bajtu.
            7 if !p.is_empty() => match p[0] {
                // HF-SCDB (0x79): jako HF-VSDB, misto OUI znacka a 2 B
                // rezervy, takze offsety poli zustavaji (tak i `drm_edid.c`).
                0x79 if p.len() >= 6 => {
                    self.hdmi_forum = Some(HdmiForum {
                        version: p[3],
                        max_tmds_char_khz: p[4] as u32 * 5_000,
                        scdc: p[5] & 0x80 != 0,
                        lte_340_scramble: p[5] & 0x08 != 0,
                    });
                }
                // YCbCr 4:2:0 Video Data Block: VIC jen ve 4:2:0.
                0x0E => self.y420_only.extend_from_slice(&p[1..]),
                _ => {}
            },
            _ => {}
        }
    }

    /// Casovani vsech VIC, ktera umime (nezname se preskoci).
    pub fn vic_timings(&self) -> Vec<Timing> {
        self.svds
            .iter()
            .filter_map(|s| {
                let mut t = vic(s.vic)?;
                if s.native {
                    t.flags |= flags::PREFERRED;
                }
                Some(t)
            })
            .collect()
    }
}

const PP: u32 = flags::HSYNC_POS | flags::VSYNC_POS;
const NN: u32 = 0;
const I: u32 = flags::INTERLACED;

const fn t(clock_khz: u32, h: [u16; 4], v: [u16; 4], f: u32) -> Timing {
    Timing {
        clock_khz,
        hactive: h[0],
        hfront: h[1],
        hsync: h[2],
        hback: h[3],
        vactive: v[0],
        vfront: v[1],
        vsync: v[2],
        vback: v[3],
        flags: f,
    }
}

/// Casovani kodu VIC (CTA-861-G, tabulka 1-2; vyber beznych rezimu).
/// U prokladanych rezimu jsou svisle porchy na jedno pole.
pub fn vic(code: u8) -> Option<Timing> {
    Some(match code {
        1 => t(25_175, [640, 16, 96, 48], [480, 10, 2, 33], NN),
        2 | 3 => t(27_000, [720, 16, 62, 60], [480, 9, 6, 30], NN),
        4 => t(74_250, [1280, 110, 40, 220], [720, 5, 5, 20], PP),
        5 => t(74_250, [1920, 88, 44, 148], [1080, 2, 5, 15], PP | I),
        16 => t(148_500, [1920, 88, 44, 148], [1080, 4, 5, 36], PP),
        17 | 18 => t(27_000, [720, 12, 64, 68], [576, 5, 5, 39], NN),
        19 => t(74_250, [1280, 440, 40, 220], [720, 5, 5, 20], PP),
        20 => t(74_250, [1920, 528, 44, 148], [1080, 2, 5, 15], PP | I),
        31 => t(148_500, [1920, 528, 44, 148], [1080, 4, 5, 36], PP),
        32 => t(74_250, [1920, 638, 44, 148], [1080, 4, 5, 36], PP),
        33 => t(74_250, [1920, 528, 44, 148], [1080, 4, 5, 36], PP),
        34 => t(74_250, [1920, 88, 44, 148], [1080, 4, 5, 36], PP),
        93 => t(297_000, [3840, 1276, 88, 296], [2160, 8, 10, 72], PP),
        94 => t(297_000, [3840, 1056, 88, 296], [2160, 8, 10, 72], PP),
        95 => t(297_000, [3840, 176, 88, 296], [2160, 8, 10, 72], PP),
        96 => t(594_000, [3840, 1056, 88, 296], [2160, 8, 10, 72], PP),
        97 => t(594_000, [3840, 176, 88, 296], [2160, 8, 10, 72], PP),
        _ => return None,
    })
}

/// Kod VIC pro casovani (pro AVI infoframe); 0 = zadny.
pub fn vic_of(t: &Timing) -> u8 {
    const KNOWN: [u8; 17] = [1, 2, 4, 5, 16, 17, 19, 20, 31, 32, 33, 34, 93, 94, 95, 96, 97];
    for &k in &KNOWN {
        if let Some(v) = vic(k)
            && v.hactive == t.hactive
            && v.vactive == t.vactive
            && v.htotal() == t.htotal()
            && v.vtotal() == t.vtotal()
            && v.interlaced() == t.interlaced()
            && v.clock_khz.abs_diff(t.clock_khz) <= v.clock_khz / 500
        {
            return k;
        }
    }
    0
}
