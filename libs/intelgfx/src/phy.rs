//! Zdrojova strana navazani linky DP na Gen9 (SKL..CML): vzor v
//! `DP_TP_CTL` portu, buzeni vyberem polozky tabulky `DDI_BUF_TRANS`
//! (tabulku naprogramoval firmware) v `DDI_BUF_CTL` bity 27:24.
//! Poradi polozek (napeti, predzdurazneni) jako `intel_ddi.c` (MIT):
//! (0,0) (0,1) (0,2) (0,3) (1,0) (1,1) (1,2) (2,0) (2,1) (3,0).
//!
//! Pouziti: znovunavazani linky po jeji ztrate (hotplug na DP se stejnou
//! rychlosti). Od ICL je buzeni v PHY (combo/Type-C) jinde - tam ovladac
//! linku znovu nenavazuje a vraci se na framebuffer firmwaru.

use display::Result;
use display::dp::auxch::Aux;
use display::dp::train::{Drive, Pattern, Phy};

use crate::dpaux::IntelAux;
use crate::regs;
use crate::{Clock, Mmio};

/// Index polozky tabulky buzeni pro (napeti, predzdurazneni).
pub fn level(d: Drive) -> u32 {
    const T: [(u8, u8); 10] = [(0, 0), (0, 1), (0, 2), (0, 3), (1, 0), (1, 1), (1, 2), (2, 0), (2, 1), (3, 0)];
    T.iter().position(|&(v, p)| v == d.vswing && p == d.preemph).unwrap_or(0) as u32
}

pub struct Gen9Link<'a, M: Mmio + ?Sized, C: Clock + ?Sized> {
    pub mmio: &'a mut M,
    pub clock: &'a mut C,
    pub port: u32,
    pub aux_ch: u32,
}

impl<M: Mmio + ?Sized, C: Clock + ?Sized> Aux for Gen9Link<'_, M, C> {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8; 20]) -> Result<usize> {
        IntelAux { mmio: &mut *self.mmio, clock: &mut *self.clock, ch: self.aux_ch }.transfer(tx, rx)
    }

    fn delay_us(&mut self, us: u32) {
        self.clock.delay_us(us);
    }
}

impl<M: Mmio + ?Sized, C: Clock + ?Sized> Phy for Gen9Link<'_, M, C> {
    fn set_pattern(&mut self, p: Pattern) -> Result<()> {
        let v = match p {
            Pattern::None => regs::DP_TP_NORMAL,
            Pattern::Tps1 => regs::DP_TP_PAT1,
            Pattern::Tps2 => regs::DP_TP_PAT2,
            Pattern::Tps3 => regs::DP_TP_PAT3,
            Pattern::Tps4 => regs::DP_TP_PAT4,
        };
        self.mmio.rmw(regs::dp_tp_ctl(self.port), regs::DP_TP_CTL_TRAIN_MASK, v << regs::DP_TP_CTL_TRAIN_SHIFT);
        Ok(())
    }

    fn set_drive(&mut self, d: Drive, _lanes: u8) -> Result<()> {
        self.mmio.rmw(regs::ddi_buf_ctl(self.port), regs::DDI_BUF_EMP_MASK, level(d) << regs::DDI_BUF_EMP_SHIFT);
        Ok(())
    }

    fn supports_tps3(&self) -> bool {
        true
    }
}
