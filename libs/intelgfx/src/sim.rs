//! Simulovane registry pro testy na hostiteli: pamet registru, zaznam
//! vsech zapisu (porovnava se se zaznamenanou ocekavanou posloupnosti)
//! a jednoduche modely hardwaru (AUX s prijemcem DP, GMBUS s EDID,
//! latch PLANE_SURF ve vsync).
//!
//! Modely jsou jen tak verne, jak je popisuje dokumentace (PRM, i915);
//! skutecny hardware se muze chovat jinak - proto `docs/cs/displej.md`
//! vycita, co na hardwaru overeno neni.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::regs::{self, aux, gmbus};
use crate::{Clock, Mmio};

/// Prijemce na kanalu AUX (model DP monitoru): prijme zakodovanou
/// zpravu, vrati odpoved (prvni bajt = kod).
pub type AuxSink = alloc::boxed::Box<dyn FnMut(&[u8]) -> Vec<u8>>;

#[derive(Default)]
pub struct Sim {
    pub regs: BTreeMap<u32, u32>,
    /// Zapisy (registr, hodnota) v poradi.
    pub writes: Vec<(u32, u32)>,
    /// Prijemce AUX podle kanalu.
    pub aux: BTreeMap<u32, AuxSink>,
    /// EDID na pinech GMBUS.
    pub ddc: BTreeMap<u32, Vec<u8>>,
    gmbus_index: usize,
    gmbus_left: u32,
    gmbus_pin: u32,
    /// Zapsane PLANE_SURF cekajici na vsync (registr SURFLIVE, hodnota).
    pending_surf: Vec<(u32, u32)>,
    /// Simulovany cas (ns).
    pub time: u64,
}

impl Sim {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, reg: u32, v: u32) -> &mut Self {
        self.regs.insert(reg, v);
        self
    }

    pub fn get(&self, reg: u32) -> u32 {
        self.regs.get(&reg).copied().unwrap_or(0)
    }

    /// Vsync: zapsane adresy povrchu se projevi (SURFLIVE), citac snimku +1.
    pub fn vblank(&mut self) {
        for (live, v) in core::mem::take(&mut self.pending_surf) {
            self.regs.insert(live, v);
        }
        for p in 0..4 {
            let r = 0x70000 + p * 0x1000 + regs::FRMCOUNT;
            let v = self.get(r).wrapping_add(1);
            self.regs.insert(r, v);
        }
    }

    /// Zapisy jen do registru v rozsahu (pro porovnani posloupnosti).
    pub fn writes_in(&self, lo: u32, hi: u32) -> Vec<(u32, u32)> {
        self.writes.iter().copied().filter(|&(r, _)| (lo..hi).contains(&r)).collect()
    }

    fn aux_send(&mut self, ctl_reg: u32, v: u32) {
        let ch = (ctl_reg - 0x64010) / 0x100;
        let n = ((v & aux::SIZE_MASK) >> aux::SIZE_SHIFT) as usize;
        let mut tx = Vec::new();
        for i in 0..5 {
            tx.extend_from_slice(&self.get(regs::aux_data(ch, i)).to_be_bytes());
        }
        tx.truncate(n);
        let reply = match self.aux.get_mut(&ch) {
            Some(sink) => sink(&tx),
            None => {
                // Nic nepripojeno: timeout.
                self.regs.insert(ctl_reg, aux::DONE | aux::TIME_OUT_ERROR);
                return;
            }
        };
        let mut words = [0u8; 20];
        let k = reply.len().min(20);
        words[..k].copy_from_slice(&reply[..k]);
        for i in 0..5 {
            let w = u32::from_be_bytes([words[i * 4], words[i * 4 + 1], words[i * 4 + 2], words[i * 4 + 3]]);
            self.regs.insert(regs::aux_data(ch, i as u32), w);
        }
        self.regs.insert(ctl_reg, aux::DONE | ((k as u32) << aux::SIZE_SHIFT));
    }

    fn gmbus_cmd(&mut self, v: u32) {
        if v & gmbus::SW_CLR_INT != 0 {
            self.regs.insert(regs::GMBUS2, gmbus::HW_RDY);
            return;
        }
        if v & gmbus::SW_RDY == 0 {
            return;
        }
        let addr = (v >> gmbus::ADDR_SHIFT) & 0x7F;
        let count = (v >> gmbus::BYTE_COUNT_SHIFT) & 0x1FF;
        if v & gmbus::CYCLE_STOP != 0 && v & gmbus::READ == 0 && count == 0 {
            // STOP: sbernice volna.
            self.regs.insert(regs::GMBUS2, gmbus::HW_RDY);
            return;
        }
        if !(self.ddc.contains_key(&self.gmbus_pin) && addr == 0x50) {
            // Nikdo neodpovedel (NAK).
            self.regs.insert(regs::GMBUS2, gmbus::SATOER | gmbus::HW_RDY);
            return;
        }
        if v & gmbus::CYCLE_INDEX != 0 {
            self.gmbus_index = ((v >> gmbus::INDEX_SHIFT) & 0xFF) as usize;
        }
        if v & gmbus::READ != 0 {
            self.gmbus_left = count;
            self.fill_gmbus3();
        }
        self.regs.insert(regs::GMBUS2, gmbus::HW_RDY | gmbus::ACTIVE);
    }

    fn fill_gmbus3(&mut self) {
        let data = self.ddc.get(&self.gmbus_pin).cloned().unwrap_or_default();
        let mut w = 0u32;
        for i in 0..4 {
            if self.gmbus_left == 0 {
                break;
            }
            let b = data.get(self.gmbus_index).copied().unwrap_or(0xFF);
            w |= (b as u32) << (8 * i);
            self.gmbus_index += 1;
            self.gmbus_left -= 1;
        }
        self.regs.insert(regs::GMBUS3, w);
    }
}

impl Mmio for Sim {
    fn read32(&mut self, reg: u32) -> u32 {
        let v = self.get(reg);
        if reg == regs::GMBUS3 {
            // Dalsi 4 bajty pro pristi cteni.
            self.fill_gmbus3();
        }
        v
    }

    fn write32(&mut self, reg: u32, v: u32) {
        self.writes.push((reg, v));
        self.regs.insert(reg, v);
        if (0x64010..0x64A10).contains(&reg) && (reg - 0x64010) % 0x100 == 0 && v & aux::SEND_BUSY != 0 {
            self.aux_send(reg, v);
        }
        if reg == regs::GMBUS0 {
            self.gmbus_pin = v & 0x1F;
        }
        if reg == regs::GMBUS1 {
            self.gmbus_cmd(v);
        }
        // PLANE_SURF: projevi se az ve vsync.
        if (0x70180..0x74000).contains(&reg) && (reg - 0x70180) % 0x100 == regs::PLANE_SURF {
            self.pending_surf.push((reg - regs::PLANE_SURF + regs::PLANE_SURFLIVE, v));
        }
    }
}

/// Samostatne hodiny bez vazby na registry.
#[derive(Default)]
pub struct FakeClock {
    pub t: u64,
}

impl Clock for FakeClock {
    fn now(&self) -> u64 {
        self.t
    }

    fn delay_us(&mut self, us: u32) {
        self.t += us as u64 * 1000;
    }
}
