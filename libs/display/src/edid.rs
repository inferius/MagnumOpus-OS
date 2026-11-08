//! Zakladni blok EDID 1.3 a 1.4 (VESA E-EDID, 128 B) a rozdeleni
//! rozsireni (CTA-861, DisplayID).
//!
//! ```text
//!  0..8    hlavicka 00 FF FF FF FF FF FF 00
//!  8..18   vyrobce (PNP id), produkt, seriove cislo, tyden a rok
//! 18..20   verze, revize
//! 20..25   vstup (digitalni: bitu na barvu, rozhrani), rozmer cm, gamma, vlastnosti
//! 25..35   barevne souradnice (nectou se)
//! 35..38   ustalena casovani (bitova mapa)
//! 38..54   8 standardnich casovani po 2 B
//! 54..126  4 deskriptory po 18 B (detailni casovani nebo textove/limity)
//! 126      pocet bloku rozsireni, 127 kontrolni soucet
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use crate::cta::Cta;
use crate::displayid::DisplayId;
use crate::timing::{Timing, flags};
use crate::{Error, Result};

pub const BLOCK: usize = 128;
pub const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
/// Nejvic bloku, ktere cteme (E-DDC umi 256 segmentu; realne monitory
/// maji 1 az 4 bloky).
pub const MAX_BLOCKS: usize = 8;

/// Rozhrani digitalniho vstupu (EDID 1.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    Undefined,
    Dvi,
    HdmiA,
    HdmiB,
    Mddi,
    DisplayPort,
    /// Analogovy vstup (VGA).
    Analog,
    Other(u8),
}

/// Vstup monitoru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Input {
    pub digital: bool,
    /// Bitu na barevnou slozku (EDID 1.4 digitalni), `None` = neuvedeno.
    pub bpc: Option<u8>,
    pub interface: Interface,
}

/// Standardni casovani (2 B): rozmery a obnova.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdTiming {
    pub w: u16,
    pub h: u16,
    pub refresh_hz: u8,
}

/// Detailni casovani z deskriptoru (DTD) i s fyzickou velikosti obrazu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detailed {
    pub timing: Timing,
    /// Velikost obrazu v mm (0 = neuvedeno).
    pub size_mm: (u16, u16),
}

/// Rozsah frekvenci monitoru (deskriptor 0xFD).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RangeLimits {
    pub vmin_hz: u16,
    pub vmax_hz: u16,
    pub hmin_khz: u16,
    pub hmax_khz: u16,
    /// Nejvyssi pixel clock v kHz (0 = neuvedeno).
    pub max_clock_khz: u32,
}

/// Blok rozsireni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extension {
    Cta(Cta),
    DisplayId(DisplayId),
    /// Neznamy typ (znacka bloku).
    Other(u8),
    /// Blok se spatnym kontrolnim souctem nebo obsahem (preskocen).
    Broken(u8),
}

/// Rozebrany EDID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edid {
    /// Tri pismena PNP id vyrobce (napr. `AOC`, `LGD`).
    pub vendor: [u8; 3],
    pub product: u16,
    pub serial: u32,
    pub week: u8,
    pub year: u16,
    pub version: (u8, u8),
    pub input: Input,
    /// Rozmer obrazovky v cm (0 = neuvedeno nebo pomer stran).
    pub size_cm: (u8, u8),
    /// Gamma x 100 (`None` = definovana jinde).
    pub gamma: Option<u16>,
    /// Bajt vlastnosti (24).
    pub features: u8,
    /// Ustalena casovani (rozmery, obnova Hz, prokladane).
    pub established: Vec<(u16, u16, u8, bool)>,
    pub standard: Vec<StdTiming>,
    pub detailed: Vec<Detailed>,
    pub name: Option<String>,
    pub serial_text: Option<String>,
    pub texts: Vec<String>,
    pub range: Option<RangeLimits>,
    pub extensions: Vec<Extension>,
}

/// Kontrolni soucet bloku: soucet vsech 128 bajtu je 0 (mod 256).
pub fn checksum_ok(block: &[u8]) -> bool {
    block.len() == BLOCK && block.iter().fold(0u8, |a, &b| a.wrapping_add(b)) == 0
}

/// Ustalena casovani: bit -> (sirka, vyska, obnova, prokladane).
const ESTABLISHED: [(u16, u16, u8, bool); 17] = [
    // Bajt 35, bit 7 az 0.
    (720, 400, 70, false),
    (720, 400, 88, false),
    (640, 480, 60, false),
    (640, 480, 67, false),
    (640, 480, 72, false),
    (640, 480, 75, false),
    (800, 600, 56, false),
    (800, 600, 60, false),
    // Bajt 36, bit 7 az 0.
    (800, 600, 72, false),
    (800, 600, 75, false),
    (832, 624, 75, false),
    (1024, 768, 87, true),
    (1024, 768, 60, false),
    (1024, 768, 70, false),
    (1024, 768, 75, false),
    (1280, 1024, 75, false),
    // Bajt 37, bit 7.
    (1152, 870, 75, false),
];

impl Edid {
    /// Rozebere EDID (zakladni blok + rozsireni, `data.len()` = nasobek 128).
    /// Zakladni blok musi mit spravnou hlavicku i soucet; vadne rozsireni
    /// se jen oznaci ([`Extension::Broken`]).
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < BLOCK {
            return Err(Error::Truncated);
        }
        let b = &data[..BLOCK];
        if b[..8] != HEADER {
            return Err(Error::BadHeader);
        }
        if !checksum_ok(b) {
            return Err(Error::BadChecksum);
        }
        let id = u16::from_be_bytes([b[8], b[9]]);
        let letter = |shift: u16| b'A' - 1 + ((id >> shift) & 0x1F) as u8;
        let version = (b[18], b[19]);
        let v14 = version >= (1, 4);
        let input = parse_input(b[20], v14);
        let gamma = if b[23] == 0xFF { None } else { Some(b[23] as u16 + 100) };
        let mut e = Edid {
            vendor: [letter(10), letter(5), letter(0)],
            product: u16::from_le_bytes([b[10], b[11]]),
            serial: u32::from_le_bytes([b[12], b[13], b[14], b[15]]),
            week: b[16],
            year: if b[17] == 0 { 0 } else { 1990 + b[17] as u16 },
            version,
            input,
            size_cm: (b[21], b[22]),
            gamma,
            features: b[24],
            established: Vec::new(),
            standard: Vec::new(),
            detailed: Vec::new(),
            name: None,
            serial_text: None,
            texts: Vec::new(),
            range: None,
            extensions: Vec::new(),
        };
        let bits = (b[35] as u32) << 16 | (b[36] as u32) << 8 | b[37] as u32;
        for (i, t) in ESTABLISHED.iter().enumerate() {
            if bits & (1 << (23 - i)) != 0 {
                e.established.push(*t);
            }
        }
        for i in 0..8 {
            if let Some(t) = parse_std(b[38 + 2 * i], b[39 + 2 * i], version) {
                e.standard.push(t);
            }
        }
        for i in 0..4 {
            let d = &b[54 + 18 * i..72 + 18 * i];
            e.descriptor(d, i == 0);
        }
        let count = (b[126] as usize).min(MAX_BLOCKS - 1);
        for i in 1..=count {
            let Some(block) = data.get(i * BLOCK..(i + 1) * BLOCK) else {
                break;
            };
            e.extensions.push(parse_extension(block));
        }
        Ok(e)
    }

    fn descriptor(&mut self, d: &[u8], first: bool) {
        if d[0] != 0 || d[1] != 0 {
            if let Some(mut t) = parse_dtd(d) {
                // Prvni DTD je preferovany (EDID 1.4 vzdy, 1.3 s bitem 1).
                if first && (self.version >= (1, 4) || self.features & 0x02 != 0) {
                    t.timing.flags |= flags::PREFERRED;
                }
                self.detailed.push(t);
            }
            return;
        }
        match d[3] {
            0xFF => self.serial_text = Some(text(&d[5..18])),
            0xFE => self.texts.push(text(&d[5..18])),
            0xFC => self.name = Some(text(&d[5..18])),
            0xFD => self.range = Some(parse_range(d)),
            0xFA => {
                for i in 0..6 {
                    if let Some(t) = parse_std(d[5 + 2 * i], d[6 + 2 * i], self.version) {
                        self.standard.push(t);
                    }
                }
            }
            _ => {}
        }
    }

    /// Jmeno pro uzivatele: jmeno monitoru z EDID, jinak vyrobce a produkt.
    pub fn display_name(&self) -> String {
        match &self.name {
            Some(n) if !n.is_empty() => n.clone(),
            _ => alloc::format!("{} {:04X}", self.vendor_str(), self.product),
        }
    }

    pub fn vendor_str(&self) -> &str {
        core::str::from_utf8(&self.vendor).unwrap_or("???")
    }

    /// Preferovane detailni casovani.
    pub fn preferred(&self) -> Option<&Timing> {
        self.detailed.iter().map(|d| &d.timing).find(|t| t.preferred()).or_else(|| self.detailed.first().map(|d| &d.timing))
    }

    /// Velikost obrazu v mm: z DTD, jinak z rozmeru v cm.
    pub fn size_mm(&self) -> (u16, u16) {
        if let Some(d) = self.detailed.iter().find(|d| d.size_mm.0 > 0 && d.size_mm.1 > 0) {
            return d.size_mm;
        }
        (self.size_cm.0 as u16 * 10, self.size_cm.1 as u16 * 10)
    }

    /// Rozsireni CTA-861 (prvni).
    pub fn cta(&self) -> Option<&Cta> {
        self.extensions.iter().find_map(|x| match x {
            Extension::Cta(c) => Some(c),
            _ => None,
        })
    }

    /// Rozsireni DisplayID.
    pub fn displayid(&self) -> impl Iterator<Item = &DisplayId> {
        self.extensions.iter().filter_map(|x| match x {
            Extension::DisplayId(d) => Some(d),
            _ => None,
        })
    }

    /// Je to monitor HDMI (CTA rozsireni s HDMI VSDB)?
    pub fn is_hdmi(&self) -> bool {
        self.cta().is_some_and(|c| c.hdmi.is_some())
    }
}

fn parse_input(v: u8, v14: bool) -> Input {
    if v & 0x80 == 0 {
        return Input {
            digital: false,
            bpc: None,
            interface: Interface::Analog,
        };
    }
    if !v14 {
        return Input {
            digital: true,
            bpc: None,
            interface: Interface::Undefined,
        };
    }
    let bpc = match (v >> 4) & 7 {
        1 => Some(6),
        2 => Some(8),
        3 => Some(10),
        4 => Some(12),
        5 => Some(14),
        6 => Some(16),
        _ => None,
    };
    let interface = match v & 0x0F {
        0 => Interface::Undefined,
        1 => Interface::Dvi,
        2 => Interface::HdmiA,
        3 => Interface::HdmiB,
        4 => Interface::Mddi,
        5 => Interface::DisplayPort,
        x => Interface::Other(x),
    };
    Input {
        digital: true,
        bpc,
        interface,
    }
}

/// Standardni casovani (2 B); `01 01` a `00 00` = nepouzito.
pub fn parse_std(a: u8, b: u8, version: (u8, u8)) -> Option<StdTiming> {
    if (a == 0x01 && b == 0x01) || a == 0 {
        return None;
    }
    let w = (a as u16 + 31) * 8;
    let h = match b >> 6 {
        // Pred EDID 1.3 znamenalo 00 pomer 1:1.
        0 if version < (1, 3) => w,
        0 => w * 10 / 16,
        1 => w * 3 / 4,
        2 => w * 4 / 5,
        _ => w * 9 / 16,
    };
    Some(StdTiming {
        w,
        h,
        refresh_hz: (b & 0x3F) + 60,
    })
}

/// Detailni casovani (18 B); `None` = pixel clock 0 (jiny deskriptor).
pub fn parse_dtd(d: &[u8]) -> Option<Detailed> {
    if d.len() < 18 {
        return None;
    }
    let clock = u16::from_le_bytes([d[0], d[1]]) as u32 * 10;
    if clock == 0 {
        return None;
    }
    let hactive = d[2] as u16 | ((d[4] as u16 >> 4) << 8);
    let hblank = d[3] as u16 | ((d[4] as u16 & 0x0F) << 8);
    let vactive = d[5] as u16 | ((d[7] as u16 >> 4) << 8);
    let vblank = d[6] as u16 | ((d[7] as u16 & 0x0F) << 8);
    let hfront = d[8] as u16 | (((d[11] as u16 >> 6) & 3) << 8);
    let hsync = d[9] as u16 | (((d[11] as u16 >> 4) & 3) << 8);
    let vfront = (d[10] as u16 >> 4) | (((d[11] as u16 >> 2) & 3) << 4);
    let vsync = (d[10] as u16 & 0x0F) | ((d[11] as u16 & 3) << 4);
    let hmm = d[12] as u16 | ((d[14] as u16 >> 4) << 8);
    let vmm = d[13] as u16 | ((d[14] as u16 & 0x0F) << 8);
    let f = d[17];
    let mut fl = 0;
    if f & 0x80 != 0 {
        fl |= flags::INTERLACED;
    }
    // Digitalni oddelena synchronizace (bity 4-3 = 11): polarity v bitech 2 a 1.
    if (f >> 3) & 3 == 3 {
        if f & 0x04 != 0 {
            fl |= flags::VSYNC_POS;
        }
        if f & 0x02 != 0 {
            fl |= flags::HSYNC_POS;
        }
    } else if (f >> 3) & 3 == 2 && f & 0x02 != 0 {
        // Digitalni slozena: bit 1 = kladna polarita hsync.
        fl |= flags::HSYNC_POS;
    }
    if hblank < hfront + hsync || vblank < vfront + vsync {
        return None;
    }
    let mut t = Timing {
        clock_khz: clock,
        hactive,
        hfront,
        hsync,
        hback: hblank - hfront - hsync,
        vactive,
        vfront,
        vsync,
        vback: vblank - vfront - vsync,
        flags: fl,
    };
    if t.interlaced() {
        // DTD udava aktivni radky jednoho pole; Timing drzi cely snimek.
        t.vactive *= 2;
    }
    Some(Detailed {
        timing: t,
        size_mm: (hmm, vmm),
    })
}

fn parse_range(d: &[u8]) -> RangeLimits {
    let off = d[4];
    let add = |bit: u8, v: u8| v as u16 + if off & bit != 0 { 255 } else { 0 };
    RangeLimits {
        vmin_hz: add(0x02, d[5]),
        vmax_hz: add(0x01, d[6]),
        hmin_khz: add(0x08, d[7]),
        hmax_khz: add(0x04, d[8]),
        max_clock_khz: d[9] as u32 * 10_000,
    }
}

/// Text deskriptoru: ASCII do znaku 0x0A, bez koncovych mezer.
fn text(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0x0A).unwrap_or(b.len());
    // Nektere panely maji pred textem nulove bajty: vynechat.
    let s: String = b[..end]
        .iter()
        .filter(|&&c| c != 0)
        .map(|&c| if (0x20..0x7F).contains(&c) { c as char } else { '?' })
        .collect();
    String::from(s.trim())
}

fn parse_extension(block: &[u8]) -> Extension {
    let tag = block[0];
    if !checksum_ok(block) {
        return Extension::Broken(tag);
    }
    match tag {
        0x02 => Cta::parse(block).map(Extension::Cta).unwrap_or(Extension::Broken(tag)),
        // DisplayID jako rozsireni EDID: sekce zacina za znackou.
        0x70 => DisplayId::parse(&block[1..BLOCK - 1]).map(Extension::DisplayId).unwrap_or(Extension::Broken(tag)),
        t => Extension::Other(t),
    }
}

/// Precte EDID po blocich pres `block(index, buf)` (DDC/GMBUS, I2C pres
/// AUX): zakladni blok, pak tolik rozsireni, kolik rika bajt 126 (nejvic
/// [`MAX_BLOCKS`]). Blok se spatnym souctem se jednou cte znovu (sum na
/// DDC); zakladni blok musi projit, rozsireni se prip. ponecha vadne
/// (parser ho oznaci jako [`Extension::Broken`]).
pub fn read(mut block: impl FnMut(u8, &mut [u8]) -> Result<()>) -> Result<Vec<u8>> {
    let mut out = alloc::vec![0u8; BLOCK];
    let mut ok = false;
    for _ in 0..2 {
        block(0, &mut out)?;
        if out[..8] == HEADER && checksum_ok(&out) {
            ok = true;
            break;
        }
    }
    if !ok {
        return Err(Error::BadChecksum);
    }
    let count = (out[126] as usize).min(MAX_BLOCKS - 1);
    for i in 1..=count {
        let mut b = [0u8; BLOCK];
        for _ in 0..2 {
            block(i as u8, &mut b)?;
            if checksum_ok(&b) {
                break;
            }
        }
        out.extend_from_slice(&b);
    }
    Ok(out)
}
