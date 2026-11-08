//! Stav obrazu, ktery nastavil firmware (GOP): zapnute transcodery,
//! jejich casovani, port DDI a rovina 1 jejich pipe.
//!
//! Format registru casovani (`intel_display_regs.h`): `HTOTAL` =
//! (celkem-1) << 16 | (aktivni-1), `HBLANK`/`HSYNC` = (konec-1) << 16 |
//! (zacatek-1), `PIPESRC` = (sirka-1) << 16 | (vyska-1).
//!
//! Predpoklad (neovereno na vsech deskach): transcoder EDP (SKL..ICL)
//! krmi pipe A - tak ho nastavuje GOP u vestavenych panelu.

use alloc::vec::Vec;

use display::timing::{Timing, flags};

use crate::Mmio;
use crate::platform::Platform;
use crate::regs::{self, Trans, ddi_func};

/// Typ vystupu na portu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Hdmi,
    Dvi,
    Dp,
    /// Vestaveny panel (transcoder EDP, nebo DP na portu A).
    Edp,
}

/// Nastaveni DDI transcoderu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ddi {
    /// Port 0 = A, 1 = B, ... (u TGL+ Type-C porty od 3).
    pub port: u32,
    pub output: Output,
    pub bpc: u8,
    /// Pocet lane DP.
    pub lanes: u8,
    pub mst: bool,
    pub raw: u32,
}

/// Rovina 1 pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plane {
    pub ctl: u32,
    pub stride: u32,
    pub pos: u32,
    pub size: u32,
    pub surf: u32,
    pub offset: u32,
}

impl Plane {
    pub fn enabled(&self) -> bool {
        self.ctl & regs::PLANE_CTL_ENABLE != 0
    }

    /// Linearni XRGB8888 - jediny format, ktery umime prevzit.
    pub fn is_linear_xrgb(&self) -> bool {
        self.ctl & regs::PLANE_CTL_TILED_MASK == 0 && self.ctl & regs::PLANE_CTL_FORMAT_MASK == regs::PLANE_CTL_FORMAT_XRGB8888
    }

    /// Krok radku v bajtech (linearni: jednotky 64 B).
    pub fn stride_bytes(&self) -> u32 {
        (self.stride & 0x7FF) * 64
    }

    pub fn ggtt(&self) -> u32 {
        self.surf & regs::PLANE_SURF_ADDR_MASK
    }
}

/// Jeden zapnuty vystup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Active {
    pub trans: Trans,
    pub pipe: regs::Pipe,
    /// Casovani (pixel clock 0 = neznamy z registru; doplni ovladac).
    pub timing: Timing,
    /// Velikost zdroje pipe (pred scalerem).
    pub src: (u32, u32),
    pub ddi: Option<Ddi>,
    pub plane: Plane,
}

fn pair(v: u32) -> (u32, u32) {
    ((v >> 16) + 1, (v & 0xFFFF) + 1)
}

/// Rozebere `TRANS_DDI_FUNC_CTL`.
pub fn decode_ddi(p: &Platform, trans: Trans, v: u32) -> Option<Ddi> {
    if v & ddi_func::ENABLE == 0 {
        return None;
    }
    let port = if p.tgl_ddi_select() {
        let s = (v & ddi_func::TGL_PORT_MASK) >> ddi_func::TGL_PORT_SHIFT;
        s.checked_sub(1)?
    } else {
        (v & ddi_func::PORT_MASK) >> ddi_func::PORT_SHIFT
    };
    let mode = (v & ddi_func::MODE_MASK) >> ddi_func::MODE_SHIFT;
    let output = match mode {
        ddi_func::MODE_HDMI => Output::Hdmi,
        ddi_func::MODE_DVI => Output::Dvi,
        ddi_func::MODE_DP_SST | ddi_func::MODE_DP_MST if trans == Trans::Edp || port == 0 => Output::Edp,
        ddi_func::MODE_DP_SST | ddi_func::MODE_DP_MST => Output::Dp,
        _ => return None,
    };
    Some(Ddi {
        port,
        output,
        bpc: regs::ddi_bpc(v),
        lanes: (((v & ddi_func::WIDTH_MASK) >> ddi_func::WIDTH_SHIFT) + 1) as u8,
        mst: mode == ddi_func::MODE_DP_MST,
        raw: v,
    })
}

/// Casovani transcoderu (bez pixel clock).
pub fn read_timing<M: Mmio + ?Sized>(m: &mut M, t: Trans, ddi_raw: u32) -> Timing {
    let base = t.offset();
    let (htotal, hactive) = pair(m.read32(base + regs::HTOTAL));
    let (hsync_end, hsync_start) = pair(m.read32(base + regs::HSYNC));
    let (vtotal, vactive) = pair(m.read32(base + regs::VTOTAL));
    let (vsync_end, vsync_start) = pair(m.read32(base + regs::VSYNC));
    let mut f = 0;
    if ddi_raw & ddi_func::PHSYNC != 0 {
        f |= flags::HSYNC_POS;
    }
    if ddi_raw & ddi_func::PVSYNC != 0 {
        f |= flags::VSYNC_POS;
    }
    let sat = |a: u32, b: u32| a.saturating_sub(b).min(u16::MAX as u32) as u16;
    Timing {
        clock_khz: 0,
        hactive: hactive as u16,
        hfront: sat(hsync_start, hactive),
        hsync: sat(hsync_end, hsync_start),
        hback: sat(htotal, hsync_end),
        vactive: vactive as u16,
        vfront: sat(vsync_start, vactive),
        vsync: sat(vsync_end, vsync_start),
        vback: sat(vtotal, vsync_end),
        flags: f,
    }
}

pub fn read_plane<M: Mmio + ?Sized>(m: &mut M, pipe: regs::Pipe) -> Plane {
    let r = |m: &mut M, reg| m.read32(regs::plane(pipe, 0, reg));
    Plane {
        ctl: r(m, regs::PLANE_CTL),
        stride: r(m, regs::PLANE_STRIDE),
        pos: r(m, regs::PLANE_POS),
        size: r(m, regs::PLANE_SIZE),
        surf: r(m, regs::PLANE_SURF),
        offset: r(m, regs::PLANE_OFFSET),
    }
}

/// Zapnute vystupy (poradi: EDP, A, B, C, D).
pub fn read<M: Mmio + ?Sized>(m: &mut M, p: &Platform) -> Vec<Active> {
    let mut out = Vec::new();
    let mut list: Vec<(Trans, regs::Pipe)> = Vec::new();
    if p.has_edp_transcoder() {
        list.push((Trans::Edp, 0));
    }
    for (i, t) in [Trans::A, Trans::B, Trans::C, Trans::D].into_iter().enumerate().take(p.pipes()) {
        list.push((t, i as u32));
    }
    for (t, pipe) in list {
        let conf = m.read32(t.pipe_offset() + regs::TRANSCONF);
        if conf & regs::TRANSCONF_ENABLE == 0 {
            continue;
        }
        // EDP krmi pipe A: pipe A pak nema vlastni transcoder A zapnuty.
        if out.iter().any(|a: &Active| a.pipe == pipe) {
            continue;
        }
        let raw = m.read32(t.offset() + regs::TRANS_DDI_FUNC_CTL);
        let ddi = decode_ddi(p, t, raw);
        let timing = read_timing(m, t, raw);
        // PIPESRC lezi v bloku transcoderu pipe (A..D), ne EDP.
        let src_base = Trans::ALL[pipe as usize].offset();
        let src = pair(m.read32(src_base + regs::PIPESRC));
        let plane = read_plane(m, pipe);
        out.push(Active {
            trans: t,
            pipe,
            timing,
            src,
            ddi,
            plane,
        });
    }
    out
}
