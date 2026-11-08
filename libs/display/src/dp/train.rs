//! Navazani linky DisplayPortu (link training, DP 1.4a kapitola 3.5.1):
//!
//! 1. **Obnova hodin (CR)**: zdroj vysila TPS1, prijemce zamyka hodiny
//!    a zada o upravu napeti (vswing) a predzdurazneni (pre-emphasis).
//!    Nejvic 10 kol, nejvic 5 kol se stejnym napetim; konci i pri maximu
//!    napeti na vsech lane.
//! 2. **Vyrovnani kanalu (EQ)**: TPS2/3/4 (podle schopnosti), prijemce
//!    hlasi EQ, zamek symbolu a zarovnani lane. Nejvic 5 kol; ztrata CR
//!    = selhani.
//! 3. Vypnuti vzoru (`TRAINING_PATTERN_SET` = 0), zdroj vysila obraz.
//!
//! Pri selhani volajici zkusi mensi konfiguraci ([`super::fallback`]).
//! Logika (spolecne buzeni pro vsechny lane = maximum pozadavku,
//! pocitadla kol) odpovida `intel_dp_link_training.c` (MIT) a
//! `drm_dp_helper.c`.

use super::auxch::{Aux, dpcd_read, dpcd_write};
use super::{Caps, LinkConfig, bits, bw_code, dpcd};
use crate::{Error, Result};

/// Treninkovy vzor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Bez vzoru: zdroj vysila obraz.
    None,
    Tps1,
    Tps2,
    Tps3,
    Tps4,
}

impl Pattern {
    /// Hodnota `TRAINING_PATTERN_SET` (bez bitu scramblingu).
    pub fn dpcd(self) -> u8 {
        match self {
            Pattern::None => 0,
            Pattern::Tps1 => 1,
            Pattern::Tps2 => 2,
            Pattern::Tps3 => 3,
            Pattern::Tps4 => 7,
        }
    }
}

/// Buzeni jedne lane: napeti 0-3, predzdurazneni 0-3 (soucet <= 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Drive {
    pub vswing: u8,
    pub preemph: u8,
}

/// Fyzicka vrstva zdroje (port DDI, PHY).
pub trait Phy {
    fn set_pattern(&mut self, p: Pattern) -> Result<()>;
    /// Stejne buzeni pro vsechny pouzite lane.
    fn set_drive(&mut self, d: Drive, lanes: u8) -> Result<()>;
    /// Nejvyssi napeti zdroje (0-3).
    fn max_vswing(&self) -> u8 {
        3
    }
    /// Nejvyssi predzdurazneni pri danem napeti.
    fn max_preemph(&self, vswing: u8) -> u8 {
        3 - vswing.min(3)
    }
    fn supports_tps3(&self) -> bool {
        true
    }
    fn supports_tps4(&self) -> bool {
        false
    }
}

/// Stav lane z registru 0x202 az 0x207.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status(pub [u8; 6]);

impl Status {
    fn lane(&self, l: usize) -> u8 {
        (self.0[l / 2] >> ((l % 2) * 4)) & 0x0F
    }

    pub fn cr_ok(&self, lanes: u8) -> bool {
        (0..lanes as usize).all(|l| self.lane(l) & bits::LANE_CR_DONE != 0)
    }

    pub fn eq_ok(&self, lanes: u8) -> bool {
        let need = bits::LANE_CR_DONE | bits::LANE_CHANNEL_EQ_DONE | bits::LANE_SYMBOL_LOCKED;
        (0..lanes as usize).all(|l| self.lane(l) & need == need) && self.0[2] & bits::INTERLANE_ALIGN_DONE != 0
    }

    /// Pozadavek prijemce: maximum pres pouzite lane.
    pub fn requested(&self, lanes: u8) -> Drive {
        let mut d = Drive::default();
        for l in 0..lanes as usize {
            let v = self.0[4 + l / 2] >> ((l % 2) * 4);
            d.vswing = d.vswing.max(v & 3);
            d.preemph = d.preemph.max((v >> 2) & 3);
        }
        d
    }
}

/// Hodnota `TRAINING_LANEx_SET` vcetne priznaku maxima.
fn lane_set<P: Phy + ?Sized>(phy: &P, d: Drive) -> u8 {
    let mut v = d.vswing | (d.preemph << 3);
    if d.vswing >= phy.max_vswing() {
        v |= 1 << 2;
    }
    if d.preemph >= phy.max_preemph(d.vswing) {
        v |= 1 << 5;
    }
    v
}

/// Pozadavek omezeny schopnostmi zdroje.
fn clamp<P: Phy + ?Sized>(phy: &P, mut d: Drive) -> Drive {
    d.vswing = d.vswing.min(phy.max_vswing());
    d.preemph = d.preemph.min(phy.max_preemph(d.vswing));
    d
}

fn write_pattern<L: Aux + Phy + ?Sized>(link: &mut L, p: Pattern, d: Drive, lanes: u8) -> Result<()> {
    link.set_pattern(p)?;
    link.set_drive(d, lanes)?;
    let mut buf = [0u8; 5];
    buf[0] = p.dpcd() | if p == Pattern::Tps4 { 0 } else { bits::SCRAMBLING_DISABLE };
    let ls = lane_set(link, d);
    for b in buf.iter_mut().skip(1).take(lanes as usize) {
        *b = ls;
    }
    dpcd_write(link, dpcd::TRAINING_PATTERN_SET, &buf[..1 + lanes as usize])
}

fn update_drive<L: Aux + Phy + ?Sized>(link: &mut L, d: Drive, lanes: u8) -> Result<()> {
    link.set_drive(d, lanes)?;
    let ls = lane_set(link, d);
    let buf = [ls; 4];
    dpcd_write(link, dpcd::TRAINING_LANE0_SET, &buf[..lanes as usize])
}

fn read_status<L: Aux + ?Sized>(link: &mut L) -> Result<Status> {
    let mut s = [0u8; 6];
    dpcd_read(link, dpcd::LANE0_1_STATUS, &mut s)?;
    Ok(Status(s))
}

/// Vysledek navazani (pro log a testy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trained {
    pub drive: Drive,
    pub pattern: Pattern,
    pub cr_rounds: u32,
    pub eq_rounds: u32,
}

/// Nastavi konfiguraci linky v prijemci a navaze linku.
pub fn train<L: Aux + Phy + ?Sized>(link: &mut L, caps: &Caps, cfg: &LinkConfig) -> Result<Trained> {
    let r = train_inner(link, caps, cfg);
    // Vzor vypnout vzdy (i po chybe), at prijemce neceka na trenink.
    let stop = dpcd_write(link, dpcd::TRAINING_PATTERN_SET, &[0]);
    let _ = link.set_pattern(Pattern::None);
    let t = r?;
    stop?;
    Ok(t)
}

fn train_inner<L: Aux + Phy + ?Sized>(link: &mut L, caps: &Caps, cfg: &LinkConfig) -> Result<Trained> {
    let lanes = cfg.lanes;
    if !matches!(lanes, 1 | 2 | 4) || lanes > caps.max_lanes {
        return Err(Error::Invalid);
    }
    // Konfigurace linky.
    let lane_count = lanes | if cfg.enhanced_frame { bits::ENHANCED_FRAME_EN } else { 0 };
    match cfg.edp_rate_index {
        Some(i) => {
            dpcd_write(link, dpcd::LANE_COUNT_SET, &[lane_count])?;
            dpcd_write(link, dpcd::LINK_RATE_SET, &[i])?;
        }
        None => {
            let bw = bw_code(cfg.rate).ok_or(Error::Invalid)?;
            dpcd_write(link, dpcd::LINK_BW_SET, &[bw, lane_count])?;
        }
    }
    dpcd_write(link, dpcd::DOWNSPREAD_CTRL, &[0])?;
    if caps.rev >= 0x12 {
        dpcd_write(link, dpcd::MAIN_LINK_CHANNEL_CODING_SET, &[bits::CODING_8B10B])?;
    }
    // 1. Obnova hodin.
    let mut d = Drive::default();
    write_pattern(link, Pattern::Tps1, d, lanes)?;
    let mut rounds = 0;
    let mut same = 0;
    loop {
        link.delay_us(100);
        let s = read_status(link)?;
        rounds += 1;
        if s.cr_ok(lanes) {
            break;
        }
        if rounds >= 10 || same >= 4 || d.vswing >= link.max_vswing() {
            return Err(Error::ClockRecovery);
        }
        let n = clamp(link, s.requested(lanes));
        if n.vswing == d.vswing {
            same += 1;
        } else {
            same = 0;
        }
        d = n;
        update_drive(link, d, lanes)?;
    }
    let cr_rounds = rounds;
    // 2. Vyrovnani kanalu.
    let pattern = if caps.tps4 && link.supports_tps4() {
        Pattern::Tps4
    } else if caps.tps3 && link.supports_tps3() {
        Pattern::Tps3
    } else {
        Pattern::Tps2
    };
    write_pattern(link, pattern, d, lanes)?;
    for eq in 1..=5 {
        link.delay_us(caps.eq_interval_us);
        let s = read_status(link)?;
        if !s.cr_ok(lanes) {
            return Err(Error::ChannelEq);
        }
        if s.eq_ok(lanes) {
            return Ok(Trained {
                drive: d,
                pattern,
                cr_rounds,
                eq_rounds: eq,
            });
        }
        d = clamp(link, s.requested(lanes));
        update_drive(link, d, lanes)?;
    }
    Err(Error::ChannelEq)
}

/// Kontrola linky za behu (po preruseni HPD): `false` = linka ztratila
/// synchronizaci a je potreba ji navazat znovu.
pub fn link_ok<L: Aux + ?Sized>(link: &mut L, lanes: u8) -> Result<bool> {
    Ok(read_status(link)?.eq_ok(lanes))
}
