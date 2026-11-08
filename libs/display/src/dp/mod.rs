//! DisplayPort: registry DPCD prijemce, schopnosti linky, vypocet sirky
//! pasma a vyber konfigurace linky.
//!
//! - [`auxch`]: transakce AUX (nativni a I2C pres AUX), cteni EDID.
//! - [`train`]: navazani linky (obnova hodin, vyrovnani kanalu).
//! - [`mst`]: Multi-Stream Transport - jen priprava (sideband zpravy, CRC).
//!
//! Adresy a vyznam bitu podle VESA DisplayPort 1.4a (stejne hodnoty
//! definuje linuxovy `include/drm/display/drm_dp.h`, licence MIT-like
//! "permission ... without fee").
//!
//! Kodovani 8b/10b (RBR az HBR3): kazdy symbol nese 1 bajt, takze
//! "link clock" v kHz (162000 pro 1.62 Gb/s) je zaroven datovy tok jedne
//! lane v kB/s. UHBR (128b/132b, DP 2.x) nepodporujeme.

pub mod auxch;
pub mod mst;
pub mod train;

use crate::timing::Timing;
use crate::{Error, Result};

/// Adresy DPCD.
pub mod dpcd {
    pub const REV: u32 = 0x000;
    pub const MAX_LINK_RATE: u32 = 0x001;
    pub const MAX_LANE_COUNT: u32 = 0x002;
    pub const MAX_DOWNSPREAD: u32 = 0x003;
    pub const DOWNSTREAMPORT_PRESENT: u32 = 0x005;
    pub const TRAINING_AUX_RD_INTERVAL: u32 = 0x00E;
    pub const SUPPORTED_LINK_RATES: u32 = 0x010;
    pub const MSTM_CAP: u32 = 0x021;
    pub const LINK_BW_SET: u32 = 0x100;
    pub const LANE_COUNT_SET: u32 = 0x101;
    pub const TRAINING_PATTERN_SET: u32 = 0x102;
    pub const TRAINING_LANE0_SET: u32 = 0x103;
    pub const DOWNSPREAD_CTRL: u32 = 0x107;
    pub const MAIN_LINK_CHANNEL_CODING_SET: u32 = 0x108;
    pub const MSTM_CTRL: u32 = 0x111;
    pub const LINK_RATE_SET: u32 = 0x115;
    pub const SINK_COUNT: u32 = 0x200;
    pub const DEVICE_SERVICE_IRQ_VECTOR: u32 = 0x201;
    pub const LANE0_1_STATUS: u32 = 0x202;
    pub const LANE_ALIGN_STATUS_UPDATED: u32 = 0x204;
    pub const SINK_STATUS: u32 = 0x205;
    pub const ADJUST_REQUEST_LANE0_1: u32 = 0x206;
    pub const SET_POWER: u32 = 0x600;
    pub const SIDEBAND_MSG_DOWN_REQ_BASE: u32 = 0x1000;
    pub const SIDEBAND_MSG_DOWN_REP_BASE: u32 = 0x1400;
    pub const EXTENDED_CAPS: u32 = 0x2200;
    pub const EDP_DPCD_REV: u32 = 0x700;
}

/// Bity v DPCD.
pub mod bits {
    pub const ENHANCED_FRAME_CAP: u8 = 1 << 7;
    pub const TPS3_SUPPORTED: u8 = 1 << 6;
    pub const TPS4_SUPPORTED: u8 = 1 << 7;
    pub const EXTENDED_RECEIVER_CAP: u8 = 1 << 7;
    pub const MST_CAP: u8 = 1 << 0;
    pub const ENHANCED_FRAME_EN: u8 = 1 << 7;
    pub const SCRAMBLING_DISABLE: u8 = 1 << 5;
    pub const LANE_CR_DONE: u8 = 1 << 0;
    pub const LANE_CHANNEL_EQ_DONE: u8 = 1 << 1;
    pub const LANE_SYMBOL_LOCKED: u8 = 1 << 2;
    pub const INTERLANE_ALIGN_DONE: u8 = 1 << 0;
    pub const SET_POWER_D0: u8 = 1;
    pub const SET_POWER_D3: u8 = 2;
    pub const CODING_8B10B: u8 = 1;
    pub const MST_EN: u8 = 1 << 0;
    pub const UP_REQ_EN: u8 = 1 << 1;
}

/// Rychlost linky (link clock v kHz, = kB/s na lane u 8b/10b).
pub const RBR: u32 = 162_000;
pub const HBR: u32 = 270_000;
pub const HBR2: u32 = 540_000;
pub const HBR3: u32 = 810_000;

/// Kod `LINK_BW_SET` pro rychlost (`None` = jen pres tabulku eDP).
pub fn bw_code(rate: u32) -> Option<u8> {
    match rate {
        RBR => Some(0x06),
        HBR => Some(0x0A),
        HBR2 => Some(0x14),
        HBR3 => Some(0x1E),
        _ => None,
    }
}

pub fn rate_of_bw_code(code: u8) -> Option<u32> {
    match code {
        0x06 => Some(RBR),
        0x0A => Some(HBR),
        0x14 => Some(HBR2),
        0x1E => Some(HBR3),
        _ => None,
    }
}

/// Schopnosti prijemce (prvnich 16 B DPCD, u eDP 1.4 i tabulka rychlosti).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    /// Verze DPCD (0x12 = 1.2, 0x14 = 1.4).
    pub rev: u8,
    /// Podporovane rychlosti, vzestupne.
    pub rates: alloc::vec::Vec<u32>,
    /// Rychlosti jsou z tabulky eDP (nastavuji se `LINK_RATE_SET` indexem).
    pub edp_table: bool,
    pub max_lanes: u8,
    pub enhanced_frame: bool,
    pub tps3: bool,
    pub tps4: bool,
    pub mst: bool,
    /// Interval cteni stavu pri vyrovnani kanalu (us).
    pub eq_interval_us: u32,
    /// Za prijemcem je prevodnik (DP -> HDMI/VGA).
    pub downstream_port: bool,
}

impl Caps {
    /// Rozebere 16 B od `DPCD_REV` (`base`), volitelne tabulku rychlosti eDP
    /// (16 B od 0x010) a `MSTM_CAP`.
    pub fn parse(base: &[u8], edp_rates: Option<&[u8]>, mstm_cap: u8) -> Result<Self> {
        if base.len() < 16 {
            return Err(Error::Truncated);
        }
        if base[0] == 0 {
            // Nulova verze = nic neni pripojeno (nebo AUX nefunguje).
            return Err(Error::Invalid);
        }
        let mut rates = alloc::vec::Vec::new();
        let mut edp_table = false;
        if let Some(t) = edp_rates {
            for c in t.as_chunks::<2>().0 {
                let v = u16::from_le_bytes([c[0], c[1]]) as u32;
                if v == 0 {
                    break;
                }
                // Jednotky 200 kHz bitove rychlosti -> link clock (deleno 10 bity).
                rates.push(v * 20);
            }
            edp_table = !rates.is_empty();
            rates.sort_unstable();
        }
        if !edp_table {
            let max = rate_of_bw_code(base[1]).unwrap_or(RBR);
            rates = [RBR, HBR, HBR2, HBR3].into_iter().filter(|&r| r <= max).collect();
        }
        let lanes = base[2] & 0x1F;
        let eq_interval_us = match base[0x0E] & 0x7F {
            0 => 400,
            n => n as u32 * 4000,
        };
        Ok(Caps {
            rev: base[0],
            rates,
            edp_table,
            max_lanes: if matches!(lanes, 1 | 2 | 4) { lanes } else { 1 },
            enhanced_frame: base[2] & bits::ENHANCED_FRAME_CAP != 0,
            tps3: base[2] & bits::TPS3_SUPPORTED != 0,
            tps4: base[3] & bits::TPS4_SUPPORTED != 0,
            mst: base[0] >= 0x12 && mstm_cap & bits::MST_CAP != 0,
            eq_interval_us,
            downstream_port: base[5] & 1 != 0,
        })
    }

    pub fn max_rate(&self) -> u32 {
        self.rates.last().copied().unwrap_or(RBR)
    }
}

/// Konfigurace linky.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConfig {
    /// Link clock v kHz.
    pub rate: u32,
    pub lanes: u8,
    pub enhanced_frame: bool,
    /// Index v tabulce rychlosti eDP (`LINK_RATE_SET`), jinak `LINK_BW_SET`.
    pub edp_rate_index: Option<u8>,
}

impl LinkConfig {
    /// Datovy tok linky v kB/s (8b/10b).
    pub fn bandwidth_kbps(&self) -> u64 {
        self.rate as u64 * self.lanes as u64
    }
}

/// Potrebny datovy tok rezimu v kB/s pri `bpp` bitech na pixel.
pub fn required_kbps(t: &Timing, bpp: u32) -> u64 {
    (t.clock_khz as u64 * bpp as u64).div_ceil(8)
}

/// Nejmensi konfigurace linky, ktera unese rezim (nejdriv mene lane pri
/// nizsi rychlosti - setri energii; jako `intel_dp_compute_link_config`
/// v rezimu "nejmensi rychlost"). `max` = omezeni zdroje.
pub fn choose_link(caps: &Caps, t: &Timing, bpp: u32, src_max_rate: u32, src_max_lanes: u8) -> Result<LinkConfig> {
    let need = required_kbps(t, bpp);
    let lanes_max = caps.max_lanes.min(src_max_lanes);
    for (idx, &rate) in caps.rates.iter().enumerate() {
        if rate > src_max_rate {
            break;
        }
        for lanes in [1u8, 2, 4] {
            if lanes > lanes_max {
                break;
            }
            let c = LinkConfig {
                rate,
                lanes,
                enhanced_frame: caps.enhanced_frame,
                edp_rate_index: if caps.edp_table { Some(idx as u8) } else { None },
            };
            if c.bandwidth_kbps() >= need {
                return Ok(c);
            }
        }
    }
    Err(Error::Bandwidth)
}

/// Mensi konfigurace po neuspesnem navazani (nejdriv nizsi rychlost,
/// pak mene lane; jako `intel_dp_get_link_train_fallback_values`).
pub fn fallback(caps: &Caps, cur: &LinkConfig) -> Option<LinkConfig> {
    let pos = caps.rates.iter().position(|&r| r == cur.rate)?;
    if pos > 0 {
        return Some(LinkConfig {
            rate: caps.rates[pos - 1],
            edp_rate_index: cur.edp_rate_index.map(|i| i - 1),
            ..*cur
        });
    }
    if cur.lanes > 1 {
        let top = caps.rates.len() - 1;
        return Some(LinkConfig {
            rate: caps.rates[top],
            lanes: cur.lanes / 2,
            edp_rate_index: cur.edp_rate_index.map(|_| top as u8),
            ..*cur
        });
    }
    None
}

/// Data a M/N hodnoty pro vysilani obrazu (DP 1.4 cast 2.2.5.2):
/// `data_m / data_n = tok dat / tok linky`, `link_m / link_n = pixel clock
/// / link clock`. N je mocnina dvou do 2^23 (jako `intel_link_compute_m_n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MN {
    pub tu: u32,
    pub data_m: u32,
    pub data_n: u32,
    pub link_m: u32,
    pub link_n: u32,
}

pub fn compute_m_n(bpp: u32, lanes: u8, pixel_khz: u32, link_khz: u32) -> MN {
    let reduce = |m: u64, n: u64| -> (u32, u32) {
        // N = 0x800000 (pevne, kvuli kompatibilite s nekterymi prijemci),
        // M prepocitane.
        let n2 = 0x80_0000u64;
        let m2 = (m * n2 / n).min(0xFF_FFFF);
        (m2 as u32, n2 as u32)
    };
    let (data_m, data_n) = reduce(pixel_khz as u64 * bpp as u64, link_khz as u64 * lanes as u64 * 8);
    let (link_m, link_n) = reduce(pixel_khz as u64, link_khz as u64);
    MN {
        tu: 64,
        data_m,
        data_n,
        link_m,
        link_n,
    }
}
