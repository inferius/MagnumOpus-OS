//! HDMI: infoframy (CTA-861-G kapitola 6), TMDS clock a scrambling
//! (HDMI 2.0, SCDC). Infoframe ma hlavicku 3 B (typ, verze, delka),
//! kontrolni soucet a data:
//!
//! ```text
//! [typ] [verze] [delka N] [soucet] [PB1 .. PBN]   soucet vsech = 0 (mod 256)
//! ```
//!
//! Hodnoty poli podle CTA-861 a `drm_hdmi_helper.c` / `hdmi.c` (MIT).
//! Ovladac zdroje zapise zakodovany ramec do svych registru (Intel:
//! `VIDEO_DIP_DATA`).

use crate::cta;
use crate::timing::Timing;

pub mod kind {
    pub const VENDOR: u8 = 0x81;
    pub const AVI: u8 = 0x82;
    pub const SPD: u8 = 0x83;
    pub const AUDIO: u8 = 0x84;
}

/// Barevny prostor (AVI Y).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colorspace {
    Rgb = 0,
    Ycbcr422 = 1,
    Ycbcr444 = 2,
    Ycbcr420 = 3,
}

/// Rozsah kvantizace RGB (AVI Q).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Range {
    Default = 0,
    Limited = 1,
    Full = 2,
}

/// Infoframe: hlavicka a az 27 B dat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Infoframe {
    pub kind: u8,
    pub version: u8,
    pub len: u8,
    pub payload: [u8; 27],
}

impl Infoframe {
    fn new(kind: u8, version: u8, len: u8) -> Self {
        Self {
            kind,
            version,
            len,
            payload: [0; 27],
        }
    }

    pub fn checksum(&self) -> u8 {
        let sum = self.payload[..self.len as usize]
            .iter()
            .fold(self.kind.wrapping_add(self.version).wrapping_add(self.len), |a, &b| a.wrapping_add(b));
        0u8.wrapping_sub(sum)
    }

    /// Bajty ramce: hlavicka, soucet, data. Vraci delku.
    pub fn encode(&self, out: &mut [u8; 31]) -> usize {
        out[0] = self.kind;
        out[1] = self.version;
        out[2] = self.len;
        out[3] = self.checksum();
        let n = self.len as usize;
        out[4..4 + n].copy_from_slice(&self.payload[..n]);
        4 + n
    }
}

/// AVI infoframe (verze 2, 13 B) pro rezim `t`.
pub fn avi(t: &Timing, cs: Colorspace, range: Range) -> Infoframe {
    let mut f = Infoframe::new(kind::AVI, 2, 13);
    let vic = cta::vic_of(t);
    // PB1: Y, A0 = 1 (platny aktivni format), bez pruhu, bez skenu.
    f.payload[0] = (cs as u8) << 5 | 1 << 4;
    // PB2: kolorimetrie podle VIC (0 = bez udaje), pomer stran obrazu,
    // aktivni format = jako obraz (8).
    let aspect = if t.hactive as u32 * 3 == t.vactive as u32 * 4 { 1 } else { 2 };
    f.payload[1] = aspect << 4 | 8;
    // PB3: IT obsah ne, rozsah kvantizace RGB.
    f.payload[2] = (range as u8) << 2;
    // PB4: VIC (rezimy 4K s HDMI VIC jdou ve vendor infoframe a tady 0,
    // HDMI 1.4b 8.2.3.2).
    f.payload[3] = if hdmi_vic(vic) != 0 { 0 } else { vic };
    // PB5: YCC rozsah, typ obsahu, opakovani pixelu 0.
    f.payload[4] = 0;
    f
}

/// HDMI VIC pro 4K rezimy, ktere maji i kod VIC (HDMI 1.4b tabulka 8-13).
pub fn hdmi_vic(vic: u8) -> u8 {
    match vic {
        95 => 1,
        94 => 2,
        93 => 3,
        _ => 0,
    }
}

/// Vendor infoframe HDMI (OUI 00-0C-03); `None`, kdyz neni potreba.
pub fn vendor_hdmi(t: &Timing) -> Option<Infoframe> {
    let hv = hdmi_vic(cta::vic_of(t));
    if hv == 0 {
        return None;
    }
    let mut f = Infoframe::new(kind::VENDOR, 1, 5);
    f.payload[..3].copy_from_slice(&[0x03, 0x0C, 0x00]);
    // HDMI_Video_Format = 001 (rozsirene rozliseni).
    f.payload[3] = 1 << 5;
    f.payload[4] = hv;
    Some(f)
}

/// SPD infoframe (jmeno zdroje): vyrobce 8 B, produkt 16 B, typ 0x09 (PC).
pub fn spd(vendor: &str, product: &str) -> Infoframe {
    let mut f = Infoframe::new(kind::SPD, 1, 25);
    for (i, b) in vendor.bytes().take(8).enumerate() {
        f.payload[i] = b;
    }
    for (i, b) in product.bytes().take(16).enumerate() {
        f.payload[8 + i] = b;
    }
    f.payload[24] = 0x09;
    f
}

/// Audio infoframe: LPCM, `channels` kanalu, ostatni z proudu.
pub fn audio(channels: u8) -> Infoframe {
    let mut f = Infoframe::new(kind::AUDIO, 1, 10);
    f.payload[0] = channels.clamp(1, 8) - 1;
    f
}

/// TMDS clock v kHz pro pixel clock a hloubku barvy (bitu na slozku).
/// 4:2:2 ma vzdy 24 bitu na pixel (= pixel clock), 4:2:0 polovinu.
pub fn tmds_clock_khz(t: &Timing, bpc: u8, cs: Colorspace) -> u32 {
    let base = match cs {
        Colorspace::Ycbcr420 => t.clock_khz / 2,
        _ => t.clock_khz,
    };
    match cs {
        Colorspace::Ycbcr422 => base,
        _ => (base as u64 * bpc as u64 / 8) as u32,
    }
}

/// Nejvyssi TMDS clock bez scramblingu (HDMI 1.4).
pub const TMDS_MAX_HDMI14_KHZ: u32 = 340_000;
/// Nejvyssi TMDS rychlost znaku HDMI 2.0.
pub const TMDS_MAX_HDMI20_KHZ: u32 = 600_000;

/// Nastaveni linky TMDS pro rezim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TmdsConfig {
    pub tmds_khz: u32,
    /// Scrambling a pomer bitovych hodin 1/40 (nad 340 MHz).
    pub scrambling: bool,
    pub ratio_1_40: bool,
}

/// Overi, ze monitor rezim unese, a urci scrambling. `None` = nad limity
/// monitoru (VSDB / HF-VSDB) nebo zdroje.
pub fn tmds_config(t: &Timing, bpc: u8, cs: Colorspace, monitor: Option<&cta::Cta>, src_max_khz: u32) -> Option<TmdsConfig> {
    let tmds = tmds_clock_khz(t, bpc, cs);
    if tmds > src_max_khz {
        return None;
    }
    let hdmi = monitor.and_then(|c| c.hdmi);
    let forum = monitor.and_then(|c| c.hdmi_forum);
    // Limit monitoru: HF-VSDB, jinak VSDB, jinak 165 MHz (DVI single link).
    let limit = match (forum, hdmi) {
        (Some(f), _) if f.max_tmds_char_khz > 0 => f.max_tmds_char_khz,
        (Some(_), _) => TMDS_MAX_HDMI14_KHZ,
        (None, Some(h)) if h.max_tmds_khz > 0 => h.max_tmds_khz,
        _ => 165_000,
    };
    if tmds > limit {
        return None;
    }
    let high = tmds > TMDS_MAX_HDMI14_KHZ;
    if high && !forum.is_some_and(|f| f.scdc) {
        return None;
    }
    let low_scramble = !high && forum.is_some_and(|f| f.scdc && f.lte_340_scramble);
    Some(TmdsConfig {
        tmds_khz: tmds,
        scrambling: high || low_scramble,
        ratio_1_40: high,
    })
}

/// SCDC (Status and Control Data Channel) pres DDC, adresa I2C 0x54.
pub mod scdc {
    pub const I2C_ADDR: u8 = 0x54;
    pub const SINK_VERSION: u8 = 0x01;
    pub const SOURCE_VERSION: u8 = 0x02;
    pub const TMDS_CONFIG: u8 = 0x20;
    pub const SCRAMBLER_STATUS: u8 = 0x21;
    pub const STATUS_FLAGS0: u8 = 0x40;
    pub const SCRAMBLING_ENABLE: u8 = 1 << 0;
    pub const BIT_CLOCK_RATIO_40: u8 = 1 << 1;

    /// Hodnota `TMDS_CONFIG` pro konfiguraci.
    pub fn tmds_config(c: &super::TmdsConfig) -> u8 {
        (if c.scrambling { SCRAMBLING_ENABLE } else { 0 }) | if c.ratio_1_40 { BIT_CLOCK_RATIO_40 } else { 0 }
    }
}
