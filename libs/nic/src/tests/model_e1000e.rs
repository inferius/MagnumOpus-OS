//! Model registru e1000e (82574 a PCH) pro testy: reset, NVM, PHY za
//! MDIC a semaforem, LANPHYPC, kruhy s klasickymi deskriptory, ICR.

use std::vec::Vec;

use super::sim::{Control, Mem, Model, Regs32};
use crate::e1000e::*;
use crate::ids::E1000eKind;

pub struct E1000eModel {
    pub regs: Regs32,
    pub kind: E1000eKind,
    /// MAC v NVM (nahraje se do RAL0/RAH0 pri resetu), `None` = prazdna.
    pub nvm_mac: Option<[u8; 6]>,
    /// MAC v EEPROM (cteni pres EERD, jen 82574).
    pub eeprom_mac: Option<[u8; 6]>,
    pub link_up: bool,
    /// PHY odpovida pres MDIC (PCH po ULP ne, dokud ho LANPHYPC neprobudi).
    pub phy_awake: bool,
    /// Firmware ME blokuje reset PHY.
    pub phy_reset_blocked: bool,
    pub resets: u32,
    pub lanphypc_toggles: u32,
    lanphypc_forced: bool,
    icr: u32,
    sent: Vec<Vec<u8>>,
    interrupts: u32,
    gone: bool,
    /// Chyby, ktere model nahlasi v dalsim prijatem ramci (bajt +13).
    pub next_rx_errors: u8,
}

impl E1000eModel {
    pub fn new(kind: E1000eKind) -> Self {
        Self {
            regs: Regs32::default(),
            kind,
            nvm_mac: Some([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]),
            eeprom_mac: None,
            link_up: true,
            phy_awake: true,
            phy_reset_blocked: false,
            resets: 0,
            lanphypc_toggles: 0,
            lanphypc_forced: false,
            icr: 0,
            sent: Vec::new(),
            interrupts: 0,
            gone: false,
            next_rx_errors: 0,
        }
    }

    fn raise(&mut self, cause: u32) {
        self.icr |= cause;
        if self.icr & self.regs.get(IMS) != 0 {
            self.interrupts += 1;
        }
    }

    fn reset(&mut self) {
        let ext = self.regs.get(EXTCNF_CTRL) & !EXTCNF_SWFLAG;
        let fwsm = self.regs.get(FWSM);
        self.regs = Regs32::default();
        self.regs.set(EXTCNF_CTRL, ext);
        self.regs.set(FWSM, fwsm);
        self.regs.set(EECD, EECD_AUTO_RD);
        self.regs.set(
            STATUS,
            if self.kind == E1000eKind::Pch {
                STATUS_LAN_INIT_DONE
            } else {
                0
            },
        );
        if let Some(m) = self.nvm_mac {
            self.regs
                .set(RAL0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
            self.regs
                .set(RAH0, u32::from(m[4]) | u32::from(m[5]) << 8 | 1 << 31);
        }
        self.icr = 0;
        self.resets += 1;
    }

    fn ring_len(&self, reg: u32) -> u32 {
        self.regs.get(reg) / 16
    }

    fn transmit(&mut self, mem: &Mem) {
        if self.regs.get(TCTL) & TCTL_EN == 0 {
            return;
        }
        let n = self.ring_len(TDLEN);
        let base = u64::from(self.regs.get(TDBAH)) << 32 | u64::from(self.regs.get(TDBAL));
        let mut head = self.regs.get(TDH);
        let tail = self.regs.get(TDT);
        assert!(tail < n, "TDT mimo kruh");
        while head != tail {
            let d = base + u64::from(head) * 16;
            let addr = mem.read64(d);
            let w = mem.read32(d + 8);
            let len = (w & 0xFFFF) as usize;
            assert!(
                w & TXD_CMD_EOP != 0 && w & TXD_CMD_IFCS != 0,
                "deskriptor bez EOP/IFCS"
            );
            assert!(len >= 60, "kratky ramec nedoplneny ({len} B)");
            self.sent.push(mem.bytes(addr, len));
            if w & TXD_CMD_RS != 0 {
                mem.write32(d + 12, TXD_DD);
            }
            head = (head + 1) % n;
        }
        self.regs.set(TDH, head);
        self.raise(INT_TXDW);
    }
}

impl Model for E1000eModel {
    fn read(&mut self, _mem: &Mem, reg: u32, width: u8) -> u32 {
        assert_eq!(width, 4, "e1000e: jen 32bitove pristupy ({reg:#x})");
        if self.gone {
            return u32::MAX;
        }
        match reg {
            ICR => core::mem::take(&mut self.icr),
            STATUS => {
                let ctrl = self.regs.get(CTRL);
                let mut st = self.regs.get(STATUS);
                if ctrl & CTRL_GIO_MASTER_DISABLE == 0 {
                    st |= STATUS_GIO_MASTER_EN;
                }
                if self.link_up && ctrl & CTRL_SLU != 0 {
                    st |= STATUS_LU | STATUS_FD | 2 << 6;
                }
                st
            }
            EERD => {
                let v = self.regs.get(EERD);
                if v & EERD_START == 0 {
                    return v;
                }
                let word = (v >> 2) as usize;
                let data = self.eeprom_mac.map_or(0xFFFF, |m| {
                    u16::from_le_bytes([m[word * 2], m[word * 2 + 1]])
                });
                (v & !EERD_START) | EERD_DONE | u32::from(data) << 16
            }
            MDIC => {
                if self.phy_awake && self.regs.get(EXTCNF_CTRL) & EXTCNF_SWFLAG != 0 {
                    MDIC_READY | 0x0154
                } else {
                    MDIC_READY | MDIC_ERROR
                }
            }
            FWSM => {
                if self.phy_reset_blocked {
                    0
                } else {
                    1 << 6
                }
            }
            _ => self.regs.get(reg),
        }
    }

    fn write(&mut self, mem: &Mem, reg: u32, width: u8, v: u32) {
        assert_eq!(width, 4, "e1000e: jen 32bitove pristupy ({reg:#x})");
        if self.gone {
            return;
        }
        match reg {
            CTRL => {
                if v & CTRL_RST != 0 {
                    self.reset();
                    return;
                }
                if v & CTRL_LANPHYPC_OVERRIDE != 0 && v & CTRL_LANPHYPC_VALUE == 0 {
                    self.lanphypc_forced = true;
                } else if self.lanphypc_forced && v & CTRL_LANPHYPC_OVERRIDE == 0 {
                    self.lanphypc_forced = false;
                    self.lanphypc_toggles += 1;
                    self.phy_awake = true;
                    let ext = self.regs.get(CTRL_EXT);
                    self.regs.set(CTRL_EXT, ext | CTRL_EXT_LPCD);
                }
                let was_up = self.regs.get(CTRL) & CTRL_SLU != 0;
                self.regs.set(CTRL, v);
                if !was_up && v & CTRL_SLU != 0 && self.link_up {
                    self.raise(INT_LSC);
                }
            }
            IMS => {
                let m = self.regs.get(IMS) | v;
                self.regs.set(IMS, m);
                // Cekajici pricina po odmaskovani take vyvola preruseni.
                if self.icr & v != 0 {
                    self.interrupts += 1;
                }
            }
            IMC => {
                let m = self.regs.get(IMS) & !v;
                self.regs.set(IMS, m);
            }
            ICR => self.icr &= !v,
            TDT => {
                self.regs.set(TDT, v);
                self.transmit(mem);
            }
            RDT => {
                assert!(v < self.ring_len(RDLEN).max(1), "RDT mimo kruh");
                self.regs.set(RDT, v);
            }
            _ => self.regs.set(reg, v),
        }
    }
}

impl Control for E1000eModel {
    fn deliver(&mut self, mem: &Mem, frame: &[u8]) -> bool {
        if self.gone || self.regs.get(RCTL) & RCTL_EN == 0 || !self.link_up {
            return false;
        }
        let n = self.ring_len(RDLEN);
        let head = self.regs.get(RDH);
        if head == self.regs.get(RDT) {
            self.raise(INT_RXO);
            return false;
        }
        let base = u64::from(self.regs.get(RDBAH)) << 32 | u64::from(self.regs.get(RDBAL));
        let d = base + u64::from(head) * 16;
        let addr = mem.read64(d);
        mem.write(addr, frame);
        mem.write32(d + 8, frame.len() as u32);
        let errors = core::mem::take(&mut self.next_rx_errors);
        mem.write32(d + 12, RXD_DD | RXD_EOP | u32::from(errors) << 8);
        self.regs.set(RDH, (head + 1) % n);
        self.raise(INT_RXT0);
        true
    }

    fn take_sent(&mut self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.sent)
    }

    fn set_link(&mut self, up: bool) {
        self.link_up = up;
        self.raise(INT_LSC);
    }

    fn interrupts(&self) -> u32 {
        self.interrupts
    }

    fn unplug(&mut self) {
        self.gone = true;
    }
}
