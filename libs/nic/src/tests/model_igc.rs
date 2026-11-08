//! Model registru igc (I225/I226) pro testy: reset DEV_RST, zapnuti
//! front (RXDCTL/TXDCTL), SRRCTL, pokrocile deskriptory, ICR.

use std::vec::Vec;

use super::sim::{Control, Mem, Model, Regs32};
use crate::igc::*;

pub struct IgcModel {
    pub regs: Regs32,
    pub nvm_mac: Option<[u8; 6]>,
    pub link_up: bool,
    /// Rychlost po autonegociaci: 10, 100, 1000 nebo 2500.
    pub speed: u32,
    pub resets: u32,
    icr: u32,
    sent: Vec<Vec<u8>>,
    interrupts: u32,
    gone: bool,
    pub next_rx_error: bool,
}

impl IgcModel {
    pub fn new() -> Self {
        Self {
            regs: Regs32::default(),
            nvm_mac: Some([0x00, 0x1B, 0x21, 0xAA, 0xBB, 0xCC]),
            link_up: true,
            speed: 2500,
            resets: 0,
            icr: 0,
            sent: Vec::new(),
            interrupts: 0,
            gone: false,
            next_rx_error: false,
        }
    }

    fn raise(&mut self, cause: u32) {
        self.icr |= cause;
        if self.icr & self.regs.get(IMS) != 0 {
            self.interrupts += 1;
        }
    }

    fn reset(&mut self) {
        self.regs = Regs32::default();
        self.regs.set(EECD, EECD_AUTO_RD);
        if let Some(m) = self.nvm_mac {
            self.regs.set(RAL0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
            self.regs.set(RAH0, u32::from(m[4]) | u32::from(m[5]) << 8 | 1 << 31);
        }
        self.icr = 0;
        self.resets += 1;
    }

    fn transmit(&mut self, mem: &Mem) {
        if self.regs.get(TCTL) & TCTL_EN == 0 || self.regs.get(TXDCTL) & QUEUE_ENABLE == 0 {
            return;
        }
        let n = self.regs.get(TDLEN) / 16;
        let base = u64::from(self.regs.get(TDBAH)) << 32 | u64::from(self.regs.get(TDBAL));
        let mut head = self.regs.get(TDH);
        let tail = self.regs.get(TDT);
        assert!(tail < n, "TDT mimo kruh");
        while head != tail {
            let d = base + u64::from(head) * 16;
            let addr = mem.read64(d);
            let cmd = mem.read32(d + 8);
            let olinfo = mem.read32(d + 12);
            let len = (cmd & 0xFFFF) as usize;
            assert_eq!(cmd & (0xF << 20), TXD_DTYP_DATA, "typ deskriptoru neni datovy");
            assert!(cmd & TXD_CMD_DEXT != 0 && cmd & TXD_CMD_EOP != 0 && cmd & TXD_CMD_IFCS != 0);
            assert_eq!((olinfo >> TXD_PAYLEN_SHIFT) as usize, len, "PAYLEN neodpovida delce");
            self.sent.push(mem.bytes(addr, len));
            if cmd & TXD_CMD_RS != 0 {
                mem.write32(d + 12, olinfo | TXD_DD);
            }
            head = (head + 1) % n;
        }
        self.regs.set(TDH, head);
        self.raise(INT_TXDW);
    }
}

impl Model for IgcModel {
    fn read(&mut self, _mem: &Mem, reg: u32, width: u8) -> u32 {
        assert_eq!(width, 4, "igc: jen 32bitove pristupy ({reg:#x})");
        if self.gone {
            return u32::MAX;
        }
        match reg {
            ICR => core::mem::take(&mut self.icr),
            STATUS => {
                let ctrl = self.regs.get(CTRL);
                let mut st = 0;
                if ctrl & CTRL_GIO_MASTER_DISABLE == 0 {
                    st |= STATUS_GIO_MASTER_EN;
                }
                if self.link_up && ctrl & CTRL_SLU != 0 {
                    st |= STATUS_LU | STATUS_FD;
                    st |= match self.speed {
                        2500 => STATUS_SPEED_1000 | STATUS_SPEED_2500,
                        1000 => STATUS_SPEED_1000,
                        100 => STATUS_SPEED_100,
                        _ => 0,
                    };
                }
                st
            }
            _ => self.regs.get(reg),
        }
    }

    fn write(&mut self, mem: &Mem, reg: u32, width: u8, v: u32) {
        assert_eq!(width, 4, "igc: jen 32bitove pristupy ({reg:#x})");
        if self.gone {
            return;
        }
        match reg {
            CTRL if v & CTRL_DEV_RST != 0 => self.reset(),
            IMS => {
                let m = self.regs.get(IMS) | v;
                self.regs.set(IMS, m);
                if self.icr & v != 0 {
                    self.interrupts += 1;
                }
            }
            IMC => {
                let m = self.regs.get(IMS) & !v;
                self.regs.set(IMS, m);
            }
            RXDCTL => {
                if v & QUEUE_ENABLE != 0 {
                    let s = self.regs.get(SRRCTL);
                    assert_eq!(s & 0x7F, SRRCTL_BSIZE_2K, "SRRCTL: buffer neni 2 KiB");
                    assert_eq!(s & (7 << 25), SRRCTL_ADV_ONEBUF, "SRRCTL: typ deskriptoru");
                    assert_ne!(self.regs.get(RDLEN), 0, "fronta zapnuta bez kruhu");
                }
                self.regs.set(RXDCTL, v);
            }
            TDT => {
                self.regs.set(TDT, v);
                self.transmit(mem);
            }
            _ => self.regs.set(reg, v),
        }
    }
}

impl Control for IgcModel {
    fn deliver(&mut self, mem: &Mem, frame: &[u8]) -> bool {
        if self.gone
            || self.regs.get(RCTL) & RCTL_EN == 0
            || self.regs.get(RXDCTL) & QUEUE_ENABLE == 0
            || !self.link_up
        {
            return false;
        }
        let n = self.regs.get(RDLEN) / 16;
        let head = self.regs.get(RDH);
        if head == self.regs.get(RDT) {
            return false;
        }
        let base = u64::from(self.regs.get(RDBAH)) << 32 | u64::from(self.regs.get(RDBAL));
        let d = base + u64::from(head) * 16;
        let addr = mem.read64(d);
        assert_eq!(mem.read64(d + 8), 0, "adresa hlavicky (rozdeleni) neni 0");
        mem.write(addr, frame);
        // Zpetny zapis prepise oba qwordy deskriptoru.
        mem.write32(d, 0x1234_5678);
        mem.write32(d + 4, 0);
        let err = if core::mem::take(&mut self.next_rx_error) { RXD_ERR_RXE } else { 0 };
        mem.write32(d + 8, RXD_DD | RXD_EOP | err);
        mem.write32(d + 12, frame.len() as u32);
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
