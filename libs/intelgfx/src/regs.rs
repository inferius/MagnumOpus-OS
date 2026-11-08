//! Registry obrazove casti Intel (offsety v MMIO BAR0 = GTTMMADR).
//!
//! Puvod hodnot (jen fakta, kod neprevzaty), vse licence MIT, overeno
//! v hlavickach souboru Linuxu:
//! - `drivers/gpu/drm/i915/display/intel_display_regs.h` (transcodery,
//!   DDI, DP_TP_CTL, M/N, hotplug, scaler),
//! - `drivers/gpu/drm/i915/display/skl_universal_plane_regs.h` (roviny),
//! - `drivers/gpu/drm/i915/display/intel_dp_aux_regs.h` (AUX),
//! - `drivers/gpu/drm/i915/display/intel_gmbus_regs.h`, `intel_gmbus.h`,
//! - `drivers/gpu/drm/i915/display/intel_display_device.c` (offsety pipe
//!   a transcoderu),
//! - `drivers/gpu/drm/i915/gt/intel_gtt.h`, `intel_ggtt.c` (GGTT).
//!
//! Verejne Intel PRM (Graphics Programmer's Reference Manual, svazky
//! "Display Engine" a "Registers") popisuji tytez registry.

/// Pipe A..D a transcoder EDP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Trans {
    A,
    B,
    C,
    D,
    /// Jen SKL az ICL (vestaveny panel na portu A).
    Edp,
}

impl Trans {
    pub const ALL: [Trans; 5] = [Trans::A, Trans::B, Trans::C, Trans::D, Trans::Edp];

    /// Offset transcoderu (`TRANSCODER_*_OFFSET`).
    pub fn offset(self) -> u32 {
        match self {
            Trans::A => 0x60000,
            Trans::B => 0x61000,
            Trans::C => 0x62000,
            Trans::D => 0x63000,
            Trans::Edp => 0x6F000,
        }
    }

    /// Offset pipe (`PIPE_*_OFFSET`); EDP ma "pipe" jen pro TRANSCONF.
    pub fn pipe_offset(self) -> u32 {
        match self {
            Trans::A => 0x70000,
            Trans::B => 0x71000,
            Trans::C => 0x72000,
            Trans::D => 0x73000,
            Trans::Edp => 0x7F000,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Trans::A => "A",
            Trans::B => "B",
            Trans::C => "C",
            Trans::D => "D",
            Trans::Edp => "EDP",
        }
    }
}

/// Pipe (rovina, scaler, citac snimku): A..D jako cislo 0..3.
pub type Pipe = u32;

// Transcoder (relativne k `Trans::offset`).
pub const HTOTAL: u32 = 0x00;
pub const HBLANK: u32 = 0x04;
pub const HSYNC: u32 = 0x08;
pub const VTOTAL: u32 = 0x0C;
pub const VBLANK: u32 = 0x10;
pub const VSYNC: u32 = 0x14;
/// `PIPESRC`: sirka-1 << 16 | vyska-1 (v bloku transcoderu, pro pipe).
pub const PIPESRC: u32 = 0x1C;
pub const DATA_M1: u32 = 0x30;
pub const DATA_N1: u32 = 0x34;
pub const LINK_M1: u32 = 0x40;
pub const LINK_N1: u32 = 0x44;
pub const TRANS_DDI_FUNC_CTL: u32 = 0x400;

/// `TRANSCONF` (relativne k `Trans::pipe_offset`).
pub const TRANSCONF: u32 = 0x08;
pub const TRANSCONF_ENABLE: u32 = 1 << 31;
pub const TRANSCONF_STATE: u32 = 1 << 30;
/// Citac snimku pipe (`PIPE_FRMCOUNT_G4X`), relativne k pipe offsetu.
pub const FRMCOUNT: u32 = 0x40;

pub mod ddi_func {
    pub const ENABLE: u32 = 1 << 31;
    /// SKL..ICL: port A..E v bitech 30:28.
    pub const PORT_SHIFT: u32 = 28;
    pub const PORT_MASK: u32 = 7 << 28;
    /// TGL+: (port + 1) v bitech 30:27, 0 = zadny.
    pub const TGL_PORT_SHIFT: u32 = 27;
    pub const TGL_PORT_MASK: u32 = 0xF << 27;
    pub const MODE_SHIFT: u32 = 24;
    pub const MODE_MASK: u32 = 7 << 24;
    pub const MODE_HDMI: u32 = 0;
    pub const MODE_DVI: u32 = 1;
    pub const MODE_DP_SST: u32 = 2;
    pub const MODE_DP_MST: u32 = 3;
    pub const BPC_SHIFT: u32 = 20;
    pub const BPC_MASK: u32 = 7 << 20;
    /// Sirka portu DP (lane - 1) v bitech 3:1.
    pub const WIDTH_SHIFT: u32 = 1;
    pub const WIDTH_MASK: u32 = 7 << 1;
    pub const PHSYNC: u32 = 1 << 16;
    pub const PVSYNC: u32 = 1 << 17;
}

/// Bitu na barvu z pole BPC (8, 10, 6, 12).
pub fn ddi_bpc(v: u32) -> u8 {
    match (v & ddi_func::BPC_MASK) >> ddi_func::BPC_SHIFT {
        1 => 10,
        2 => 6,
        3 => 12,
        _ => 8,
    }
}

// Univerzalni roviny (SKL+): rovina 1 pipe A od 0x70180, pipe po
// 0x1000, rovina po 0x100.
pub fn plane(pipe: Pipe, plane: u32, reg: u32) -> u32 {
    0x70180 + pipe * 0x1000 + plane * 0x100 + reg
}
pub const PLANE_CTL: u32 = 0x00;
pub const PLANE_STRIDE: u32 = 0x08;
pub const PLANE_POS: u32 = 0x0C;
pub const PLANE_SIZE: u32 = 0x10;
pub const PLANE_SURF: u32 = 0x1C;
pub const PLANE_OFFSET: u32 = 0x24;
pub const PLANE_SURFLIVE: u32 = 0x2C;
pub const PLANE_CTL_ENABLE: u32 = 1 << 31;
/// Pole formatu SKL (27:24) - XRGB8888 = 4 plati i na ICL+ (bit 23 = 0).
pub const PLANE_CTL_FORMAT_MASK: u32 = 0xF << 24;
pub const PLANE_CTL_FORMAT_XRGB8888: u32 = 4 << 24;
/// Dlazdice (12:10), 0 = linearni.
pub const PLANE_CTL_TILED_MASK: u32 = 7 << 10;
/// Adresa povrchu (31:12).
pub const PLANE_SURF_ADDR_MASK: u32 = 0xFFFF_F000;

// Scaler pipe (SKL+): scaler 1 pipe A 0x68180, pipe po 0x800, scaler po 0x100.
pub fn scaler(pipe: Pipe, id: u32, reg: u32) -> u32 {
    0x68100 + pipe * 0x800 + id * 0x100 + reg
}
pub const PS_WIN_POS: u32 = 0x70;
pub const PS_WIN_SZ: u32 = 0x74;
pub const PS_CTRL: u32 = 0x80;
pub const PS_SCALER_EN: u32 = 1 << 31;
/// Filtr "medium" = 0 v bitech 24:23.
pub const PS_FILTER_MEDIUM: u32 = 0;

// DDI a DP (SKL..ICL: po portech od 0x64000, krok 0x100).
pub fn ddi_buf_ctl(port: u32) -> u32 {
    0x64000 + port * 0x100
}
pub const DDI_BUF_CTL_ENABLE: u32 = 1 << 31;
pub const DDI_BUF_IS_IDLE: u32 = 1 << 7;
/// Vyber polozky tabulky buzeni (SKL, bity 27:24).
pub const DDI_BUF_EMP_SHIFT: u32 = 24;
pub const DDI_BUF_EMP_MASK: u32 = 0xF << 24;
pub fn dp_tp_ctl(port: u32) -> u32 {
    0x64040 + port * 0x100
}
pub const DP_TP_CTL_ENABLE: u32 = 1 << 31;
pub const DP_TP_CTL_ENHANCED_FRAME: u32 = 1 << 18;
pub const DP_TP_CTL_TRAIN_SHIFT: u32 = 8;
pub const DP_TP_CTL_TRAIN_MASK: u32 = 7 << 8;
pub const DP_TP_PAT1: u32 = 0;
pub const DP_TP_PAT2: u32 = 1;
pub const DP_TP_IDLE: u32 = 2;
pub const DP_TP_NORMAL: u32 = 3;
pub const DP_TP_PAT3: u32 = 4;
pub const DP_TP_PAT4: u32 = 5;

// AUX (pred MTL): kanal A 0x64010, krok 0x100, 5 datovych registru.
pub fn aux_ctl(ch: u32) -> u32 {
    0x64010 + ch * 0x100
}
pub fn aux_data(ch: u32, i: u32) -> u32 {
    0x64014 + ch * 0x100 + i * 4
}
pub mod aux {
    pub const SEND_BUSY: u32 = 1 << 31;
    pub const DONE: u32 = 1 << 30;
    pub const INTERRUPT: u32 = 1 << 29;
    pub const TIME_OUT_ERROR: u32 = 1 << 28;
    pub const TIME_OUT_MAX: u32 = 3 << 26;
    pub const RECEIVE_ERROR: u32 = 1 << 25;
    pub const SIZE_SHIFT: u32 = 20;
    pub const SIZE_MASK: u32 = 0x1F << 20;
    /// `FW_SYNC_PULSE_SKL(32)` a `SYNC_PULSE_SKL(32)`.
    pub const SYNC_PULSES_SKL: u32 = (31 << 5) | 31;
}

// GMBUS (I2C pro DDC), blok PCH_DISPLAY_BASE 0xC0000 + 0x5100.
pub const GMBUS0: u32 = 0xC5100;
pub const GMBUS1: u32 = 0xC5104;
pub const GMBUS2: u32 = 0xC5108;
pub const GMBUS3: u32 = 0xC510C;
pub const GMBUS4: u32 = 0xC5110;
pub const GMBUS5: u32 = 0xC5120;
pub mod gmbus {
    pub const RATE_100KHZ: u32 = 0;
    pub const SW_CLR_INT: u32 = 1 << 31;
    pub const SW_RDY: u32 = 1 << 30;
    pub const CYCLE_WAIT: u32 = 1 << 25;
    pub const CYCLE_INDEX: u32 = 2 << 25;
    pub const CYCLE_STOP: u32 = 4 << 25;
    pub const BYTE_COUNT_SHIFT: u32 = 16;
    pub const INDEX_SHIFT: u32 = 8;
    pub const ADDR_SHIFT: u32 = 1;
    pub const READ: u32 = 1;
    pub const INUSE: u32 = 1 << 15;
    pub const HW_WAIT_PHASE: u32 = 1 << 14;
    pub const HW_RDY: u32 = 1 << 11;
    pub const SATOER: u32 = 1 << 10;
    pub const ACTIVE: u32 = 1 << 9;
}

// Hotplug.
pub const SDEISR: u32 = 0xC4000;
pub const SDE_PORTB_HOTPLUG_CPT: u32 = 1 << 21;
pub const SDE_PORTC_HOTPLUG_CPT: u32 = 1 << 22;
pub const SDE_PORTD_HOTPLUG_CPT: u32 = 1 << 23;
/// ICP+: DDI port A.. v bitech 16+.
pub fn sde_ddi_hotplug_icp(port: u32) -> u32 {
    1 << (16 + port)
}
pub const GEN8_DE_PORT_ISR: u32 = 0x44440;
/// Port A (CPU) na SKL: `GEN8_DE_PORT_HOTPLUG(HPD_PORT_A)` = bit 3.
pub const DE_PORT_HOTPLUG_A: u32 = 1 << 3;
pub const GEN11_DE_HPD_ISR: u32 = 0x44470;
/// Type-C port `tc` (0 = TC1): DP alt mode bit 16+tc, Thunderbolt bit tc.
pub fn gen11_tc_hotplug(tc: u32) -> u32 {
    (1 << (16 + tc)) | (1 << tc)
}

// GGTT: v BAR0 od 8 MiB (gen8+), polozky po 8 B.
pub const GGTT_OFFSET: u32 = 8 << 20;
pub const PTE_PRESENT: u64 = 1 << 0;
/// Polozka ukazuje do lokalni pameti karty (diskretni).
pub const PTE_LM: u64 = 1 << 1;
pub const PTE_ADDR_MASK: u64 = ((1u64 << 46) - 1) & !0xFFF;

/// Konfiguracni prostor PCI: zakladna stolen pameti (BDSM) a GGC.
pub const PCI_GGC: u16 = 0x50;
pub const PCI_BDSM: u16 = 0x5C;
