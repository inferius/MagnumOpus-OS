//! Konektory, monitory a rezimy v protokolu displeje ([`crate::display`]):
//! odpoved `CONNECTORS`, pozadavek `SET_MODE` a udalost `HOTPLUG`.
//!
//! Spolecne pro vsechny ovladace (fbdisplay, virtio-gpu, bochs-display,
//! intel-display). Ovladac bez nastavovani rezimu (framebuffer z
//! firmwaru) hlasi jediny konektor s jedinym rezimem.
//!
//! Kodovani konektoru: `output u32 | kind u8 | connected bool | monitor str
//! | sirka mm u32 | vyska mm u32 | pocet rezimu u32 | rezimy`;
//! rezim: `w u32 | h u32 | obnova mHz u32 | pixel clock kHz u32 | priznaky u32`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{ReadExt, Reader, Result, WriteExt, Writer, read_count};

/// Typ konektoru.
pub mod kind {
    /// Virtualni vystup (virtio-gpu, bochs-display).
    pub const VIRTUAL: u8 = 0;
    /// Framebuffer firmwaru (GOP) - typ skutecneho vystupu neznamy.
    pub const FIRMWARE: u8 = 1;
    pub const DISPLAYPORT: u8 = 2;
    /// Vestaveny panel (eDP).
    pub const EDP: u8 = 3;
    pub const HDMI: u8 = 4;
    pub const DVI: u8 = 5;
    pub const VGA: u8 = 6;

    pub fn name(k: u8) -> &'static str {
        match k {
            VIRTUAL => "virtual",
            FIRMWARE => "firmware",
            DISPLAYPORT => "DP",
            EDP => "eDP",
            HDMI => "HDMI",
            DVI => "DVI",
            VGA => "VGA",
            _ => "?",
        }
    }
}

/// Priznaky rezimu.
pub mod mode_flags {
    /// Preferovany (nativni) rezim monitoru.
    pub const PREFERRED: u32 = 1 << 0;
    /// Prave nastaveny rezim.
    pub const CURRENT: u32 = 1 << 1;
}

/// Nejvic konektoru a rezimu v odpovedi (zbytek ovladac vynecha).
pub const MAX_CONNECTORS: usize = 16;
pub const MAX_MODES: usize = 128;

/// Rezim vystupu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mode {
    pub w: u32,
    pub h: u32,
    /// Obnova v tisicinach Hz (59940 = 59.94 Hz).
    pub refresh_mhz: u32,
    /// Pixel clock v kHz (0 = neznamy, napr. framebuffer firmwaru).
    pub clock_khz: u32,
    pub flags: u32,
}

impl Mode {
    pub fn is_current(&self) -> bool {
        self.flags & mode_flags::CURRENT != 0
    }

    pub fn is_preferred(&self) -> bool {
        self.flags & mode_flags::PREFERRED != 0
    }
}

/// Konektor (vystup) a pripojeny monitor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Connector {
    /// Cislo vystupu (stejne jako v `ATTACH` / `PRESENT`).
    pub output: u32,
    pub kind: u8,
    pub connected: bool,
    /// Jmeno monitoru z EDID (prazdne = nezname).
    pub monitor: String,
    /// Fyzicka velikost obrazu v mm (0 = neznama).
    pub size_mm: (u32, u32),
    pub modes: Vec<Mode>,
}

impl Connector {
    /// Prave nastaveny rezim.
    pub fn current(&self) -> Option<&Mode> {
        self.modes.iter().find(|m| m.is_current())
    }

    pub fn preferred(&self) -> Option<&Mode> {
        self.modes.iter().find(|m| m.is_preferred())
    }

    /// Jediny konektor s jedinym rezimem (framebuffer, ovladac bez
    /// nastavovani rezimu).
    pub fn fixed(output: u32, kind: u8, w: u32, h: u32) -> Self {
        Connector {
            output,
            kind,
            connected: true,
            monitor: String::new(),
            size_mm: (0, 0),
            modes: vec![Mode {
                w,
                h,
                refresh_mhz: 60_000,
                clock_khz: 0,
                flags: mode_flags::CURRENT | mode_flags::PREFERRED,
            }],
        }
    }
}

pub(crate) fn encode_connectors(w: &mut Writer, cs: &[Connector]) {
    let cs = &cs[..cs.len().min(MAX_CONNECTORS)];
    w.u32(cs.len() as u32);
    for c in cs {
        w.u32(c.output).u8(c.kind).bool(c.connected).str(&c.monitor).u32(c.size_mm.0).u32(c.size_mm.1);
        let ms = &c.modes[..c.modes.len().min(MAX_MODES)];
        w.u32(ms.len() as u32);
        for m in ms {
            w.u32(m.w).u32(m.h).u32(m.refresh_mhz).u32(m.clock_khz).u32(m.flags);
        }
    }
}

pub(crate) fn decode_connectors(r: &mut Reader<'_>, msg_len: usize) -> Result<Vec<Connector>> {
    // Nejmensi konektor: 4 + 1 + 1 + 2 + 4 + 4 + 4 bajtu.
    let n = read_count(r, 20, msg_len)?;
    if n > MAX_CONNECTORS {
        return Err(crate::Error::InvalidArgs);
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let output = r.u32()?;
        let kind = r.u8()?;
        let connected = r.bool()?;
        let monitor = r.str()?;
        let size_mm = (r.u32()?, r.u32()?);
        let k = read_count(r, 20, msg_len)?;
        if k > MAX_MODES {
            return Err(crate::Error::InvalidArgs);
        }
        let mut modes = Vec::with_capacity(k);
        for _ in 0..k {
            modes.push(Mode {
                w: r.u32()?,
                h: r.u32()?,
                refresh_mhz: r.u32()?,
                clock_khz: r.u32()?,
                flags: r.u32()?,
            });
        }
        out.push(Connector {
            output,
            kind,
            connected,
            monitor,
            size_mm,
            modes,
        });
    }
    Ok(out)
}

/// Vyber vystupu pro kompozitor: prvni pripojeny konektor s rezimem
/// (prednost ma vestaveny panel), jinak `None`.
pub fn primary(cs: &[Connector]) -> Option<&Connector> {
    let usable = |c: &&Connector| c.connected && !c.modes.is_empty();
    cs.iter().filter(usable).find(|c| c.kind == kind::EDP).or_else(|| cs.iter().find(usable))
}

/// Rezim pro pozadavek `w` x `h` s obnovou `refresh_mhz` (0 = nejvyssi).
pub fn find_mode(c: &Connector, w: u32, h: u32, refresh_mhz: u32) -> Option<&Mode> {
    c.modes
        .iter()
        .filter(|m| m.w == w && m.h == h)
        .min_by_key(|m| if refresh_mhz == 0 { u32::MAX - m.refresh_mhz } else { m.refresh_mhz.abs_diff(refresh_mhz) })
}

/// Odpoved na `CONNECTORS` a `SET_MODE` ovladace s pevnymi rezimy
/// (`sizes[i]` = rozmery vystupu `i`): `SET_MODE` uspeje jen pro
/// stavajici rozmery. Ostatni pozadavky -> `None`.
pub fn fixed_reply(req: &crate::display::Request, sizes: &[(u32, u32)], kind: u8) -> Option<crate::display::Reply> {
    use crate::display::{Reply, Request, op};
    Some(match *req {
        Request::Connectors => Reply::Connectors {
            connectors: sizes.iter().enumerate().map(|(i, &(w, h))| Connector::fixed(i as u32, kind, w, h)).collect(),
        },
        Request::SetMode { output, w, h, .. } => match sizes.get(output as usize) {
            Some(&(cw, ch)) if (cw, ch) == (w, h) => Reply::ModeSet { output, w, h, refresh_mhz: 60_000 },
            Some(_) => Reply::Error { op: op::SET_MODE, code: crate::Error::NotSupported },
            None => Reply::Error { op: op::SET_MODE, code: crate::Error::NotFound },
        },
        _ => return None,
    })
}
