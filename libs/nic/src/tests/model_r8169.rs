//! Model registru RTL8168 pro testy: bajtove registry s pristupy 8/16/32
//! bitu, reset ChipCmd, zamek Cfg9346, vlastni indexy kruhu cipu (cip je
//! po resetu nastavi na zacatek a pak jde podle bitu OWN a RingEnd).

use std::vec::Vec;

use super::sim::{Control, Mem, Model};
use crate::r8169::*;

pub struct R8169Model {
    pub regs: [u8; 256],
    pub mac: [u8; 6],
    pub link_up: bool,
    pub resets: u32,
    rx_idx: u64,
    tx_idx: u64,
    sent: Vec<Vec<u8>>,
    interrupts: u32,
    gone: bool,
    /// Dalsi prijaty ramec rozdelit do dvou deskriptoru (delsi nez buffer).
    pub next_split: bool,
    pub next_rx_error: bool,
}

impl R8169Model {
    pub fn new() -> Self {
        let mut m = Self {
            regs: [0; 256],
            mac: [0x00, 0xE0, 0x4C, 0x68, 0x00, 0x01],
            link_up: true,
            resets: 0,
            rx_idx: 0,
            tx_idx: 0,
            sent: Vec::new(),
            interrupts: 0,
            gone: false,
            next_split: false,
            next_rx_error: false,
        };
        m.power_on();
        m
    }

    fn power_on(&mut self) {
        self.regs[..6].copy_from_slice(&self.mac);
        // XID RTL8168h v TxConfig (jen pro realisticky obsah).
        self.set32(TX_CONFIG, 0x5400_0000);
    }

    fn get16(&self, r: u32) -> u16 {
        u16::from_le_bytes([self.regs[r as usize], self.regs[r as usize + 1]])
    }

    fn get32(&self, r: u32) -> u32 {
        let r = r as usize;
        u32::from_le_bytes(self.regs[r..r + 4].try_into().unwrap())
    }

    fn set16(&mut self, r: u32, v: u16) {
        self.regs[r as usize..r as usize + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn set32(&mut self, r: u32, v: u32) {
        self.regs[r as usize..r as usize + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn addr(&self, lo: u32) -> u64 {
        u64::from(self.get32(lo + 4)) << 32 | u64::from(self.get32(lo))
    }

    fn raise(&mut self, cause: u16) {
        let st = self.get16(INTR_STATUS) | cause;
        self.set16(INTR_STATUS, st);
        if st & self.get16(INTR_MASK) != 0 {
            self.interrupts += 1;
        }
    }

    fn high_dma_ok(&self, addr: u64) {
        if addr >> 32 != 0 {
            assert!(
                self.get16(CPLUS_CMD) & CPLUS_PCI_DAC != 0,
                "adresa nad 4 GiB bez PCIDAC"
            );
        }
    }

    fn transmit(&mut self, mem: &Mem) {
        if self.regs[CHIP_CMD as usize] & CMD_TX_ENB == 0 {
            return;
        }
        let base = self.addr(TX_DESC_LO);
        let mut sent_any = false;
        loop {
            let d = base + self.tx_idx * 16;
            let opts1 = mem.read32(d);
            if opts1 & DESC_OWN == 0 {
                break;
            }
            assert!(
                opts1 & DESC_FIRST != 0 && opts1 & DESC_LAST != 0,
                "ramec ve vic deskriptorech"
            );
            let len = (opts1 & LEN_MASK) as usize;
            assert!(len >= 60, "kratky ramec nedoplneny ({len} B)");
            let addr = mem.read64(d + 8);
            self.high_dma_ok(addr);
            self.sent.push(mem.bytes(addr, len));
            mem.write32(d, opts1 & !DESC_OWN);
            self.tx_idx = if opts1 & DESC_RING_END != 0 {
                0
            } else {
                self.tx_idx + 1
            };
            assert!(self.tx_idx < 1024, "kruh odesilani bez RingEnd");
            sent_any = true;
        }
        if sent_any {
            self.raise(INT_TX_OK);
        }
    }

    fn reset(&mut self) {
        let mac = self.regs[..6].to_vec();
        let txcfg = self.get32(TX_CONFIG) & 0x7FF0_0000;
        self.regs = [0; 256];
        self.regs[..6].copy_from_slice(&mac);
        self.set32(TX_CONFIG, txcfg);
        self.rx_idx = 0;
        self.tx_idx = 0;
        self.resets += 1;
    }

    fn put_rx(&mut self, mem: &Mem, data: &[u8], flags: u32) -> bool {
        let base = self.addr(RX_DESC_LO);
        let d = base + self.rx_idx * 16;
        let opts1 = mem.read32(d);
        if opts1 & DESC_OWN == 0 {
            return false;
        }
        let size = (opts1 & LEN_MASK) as usize;
        assert!(data.len() <= size, "ramec delsi nez buffer deskriptoru");
        let addr = mem.read64(d + 8);
        self.high_dma_ok(addr);
        mem.write(addr, data);
        mem.write32(d, (opts1 & DESC_RING_END) | flags | data.len() as u32);
        self.rx_idx = if opts1 & DESC_RING_END != 0 {
            0
        } else {
            self.rx_idx + 1
        };
        assert!(self.rx_idx < 1024, "kruh prijmu bez RingEnd");
        true
    }
}

impl Model for R8169Model {
    fn read(&mut self, _mem: &Mem, reg: u32, width: u8) -> u32 {
        if self.gone {
            return u32::MAX >> (32 - 8 * u32::from(width));
        }
        if reg == PHY_STATUS {
            assert_eq!(width, 1);
            return if self.link_up {
                u32::from(PHY_LINK | PHY_1000F | PHY_FULL_DUP)
            } else {
                0
            };
        }
        match width {
            1 => self.regs[reg as usize].into(),
            2 => self.get16(reg).into(),
            _ => self.get32(reg),
        }
    }

    fn write(&mut self, mem: &Mem, reg: u32, width: u8, v: u32) {
        if self.gone {
            return;
        }
        let unlocked = self.regs[CFG9346 as usize] == CFG9346_UNLOCK;
        match (reg, width) {
            (CHIP_CMD, 1) if v as u8 & CMD_RESET != 0 => self.reset(),
            (TX_POLL, 1) => {
                if v as u8 & TX_POLL_NPQ != 0 {
                    self.transmit(mem);
                }
            }
            (INTR_STATUS, 2) => {
                let st = self.get16(INTR_STATUS) & !(v as u16);
                self.set16(INTR_STATUS, st);
            }
            (MAC0 | MAC4, 4) => {
                assert!(unlocked, "zapis MAC bez odemceni Cfg9346");
                self.set32(reg, v);
            }
            (RX_MAX_SIZE | CPLUS_CMD, 2) => {
                assert!(unlocked, "zapis konfigurace bez odemceni Cfg9346");
                self.set16(reg, v as u16);
            }
            (_, 1) => self.regs[reg as usize] = v as u8,
            (_, 2) => self.set16(reg, v as u16),
            _ => self.set32(reg, v),
        }
    }
}

impl Control for R8169Model {
    fn deliver(&mut self, mem: &Mem, frame: &[u8]) -> bool {
        if self.gone || self.regs[CHIP_CMD as usize] & CMD_RX_ENB == 0 || !self.link_up {
            return false;
        }
        // Na drate je ramec i s FCS (4 B); cip ho do bufferu zapise taky.
        let mut wire = frame.to_vec();
        wire.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let err = if core::mem::take(&mut self.next_rx_error) {
            RX_RES
        } else {
            0
        };
        let ok = if core::mem::take(&mut self.next_split) {
            let (a, b) = wire.split_at(wire.len() / 2);
            self.put_rx(mem, a, DESC_FIRST) && self.put_rx(mem, b, DESC_LAST)
        } else {
            self.put_rx(mem, &wire, DESC_FIRST | DESC_LAST | err)
        };
        if ok {
            self.raise(INT_RX_OK);
        } else {
            self.raise(INT_RX_OVERFLOW);
        }
        ok
    }

    fn take_sent(&mut self) -> Vec<Vec<u8>> {
        core::mem::take(&mut self.sent)
    }

    fn set_link(&mut self, up: bool) {
        self.link_up = up;
        self.raise(INT_LINK_CHG);
    }

    fn interrupts(&self) -> u32 {
        self.interrupts
    }

    fn unplug(&mut self) {
        self.gone = true;
    }
}
