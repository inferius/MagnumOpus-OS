//! Realtek RTL8111/8168 (ovladac r8169 v Linuxu). Deskriptory 16 B:
//! `opts1` (vlastnik, konec kruhu, prvni/posledni cast, delka), `opts2`,
//! adresa. Registry jsou 8-, 16- i 32bitove na ruznych offsetech.
//!
//! Inicializace (podle Linuxu `rtl_hw_start` zjednodusene pro jednu
//! frontu):
//!
//! 1. zakazat preruseni, reset (`ChipCmd.CmdReset`, cip ho sam shodi),
//! 2. MAC z `MAC0..MAC5` (cip ji tam nahraje z eFuse/EEPROM),
//! 3. odemknout konfiguracni registry (`Cfg9346`), adresy kruhu,
//!    `RxMaxSize`, `CPlusCmd` (bez offloadu a VLAN), zapnout prijem
//!    a odesilani, `RxConfig`, `TxConfig`, zamknout,
//! 4. preruseni (`IntrMask`).
//!
//! Delka prijateho ramce v `opts1` je vcetne FCS (4 B), ovladac ji
//! odecte. Kratke ramce doplni na 60 B sam (nektere revize cipu je
//! nedoplnuji spravne).
//!
//! **Overeni:** jen testy na hostiteli nad simulovanymi registry - QEMU
//! cip RTL8168 nema (`rtl8139` je jiny, starsi cip). Na skutecnem hardwaru
//! ovladac overeny neni. Proti Linuxu chybi nastaveni PHY a firmware
//! (`rtl_fw`) pro jednotlive revize cipu (XID v `TxConfig`), ASPM, EEE
//! a opravy chyb revizi; linka spoleha na vychozi stav PHY po zapnuti.
//!
//! **Puvod:** vlastni kod. Z ovladacu Linuxu (GPL-2) a datasheetu vyrobce
//! jsou prevzata jen fakta - offsety a bity registru, poradi inicializace
//! a chyby cipu; zadny kod z nich prelozeny neni. Kod z BSD neprevzat.

use alloc::vec::Vec;

use crate::{
    BUF, Events, Hw, HwError, Layout, LinkState, MAX_FRAME, Nic, RING, Stats, TxRing, copy_tx, local_mac,
    valid_mac, wait_for, write_addr,
};

pub const MAC0: u32 = 0x00;
pub const MAC4: u32 = 0x04;
pub const MAR0: u32 = 0x08;
pub const TX_DESC_LO: u32 = 0x20;
pub const TX_DESC_HI: u32 = 0x24;
pub const CHIP_CMD: u32 = 0x37;
pub const TX_POLL: u32 = 0x38;
pub const INTR_MASK: u32 = 0x3C;
pub const INTR_STATUS: u32 = 0x3E;
pub const TX_CONFIG: u32 = 0x40;
pub const RX_CONFIG: u32 = 0x44;
pub const CFG9346: u32 = 0x50;
pub const PHY_STATUS: u32 = 0x6C;
pub const RX_MAX_SIZE: u32 = 0xDA;
pub const CPLUS_CMD: u32 = 0xE0;
pub const INTR_MITIGATE: u32 = 0xE2;
pub const RX_DESC_LO: u32 = 0xE4;
pub const RX_DESC_HI: u32 = 0xE8;
pub const MAX_TX_PACKET_SIZE: u32 = 0xEC;

pub const CMD_RESET: u8 = 0x10;
pub const CMD_RX_ENB: u8 = 0x08;
pub const CMD_TX_ENB: u8 = 0x04;
pub const TX_POLL_NPQ: u8 = 0x40;

pub const INT_RX_OK: u16 = 0x0001;
pub const INT_RX_ERR: u16 = 0x0002;
pub const INT_TX_OK: u16 = 0x0004;
pub const INT_TX_ERR: u16 = 0x0008;
pub const INT_RX_OVERFLOW: u16 = 0x0010;
pub const INT_LINK_CHG: u16 = 0x0020;
pub const INT_RX_FIFO_OVER: u16 = 0x0040;
pub const INT_TX_DESC_UNAVAIL: u16 = 0x0080;
pub const INT_SYS_ERR: u16 = 0x8000;
const INT_MASK: u16 = INT_RX_OK
    | INT_RX_ERR
    | INT_TX_OK
    | INT_TX_ERR
    | INT_RX_OVERFLOW
    | INT_LINK_CHG
    | INT_RX_FIFO_OVER
    | INT_TX_DESC_UNAVAIL
    | INT_SYS_ERR;

pub const RX_ACCEPT_BROADCAST: u32 = 0x08;
pub const RX_ACCEPT_MULTICAST: u32 = 0x04;
pub const RX_ACCEPT_MY_PHYS: u32 = 0x02;
/// DMA bez omezeni delky davky (7 << 8), preruseni po 128 B prijmu
/// a vic davek najednou (RTL8168C a novejsi).
const RX_DMA_BURST: u32 = 7 << 8;
const RX_MULTI_EN: u32 = 1 << 14;
const RX128_INT_EN: u32 = 1 << 15;
/// DMA odesilani bez omezeni davky, standardni mezera mezi ramci.
const TX_CONFIG_VALUE: u32 = 7 << 8 | 3 << 24;

pub const CFG9346_LOCK: u8 = 0x00;
pub const CFG9346_UNLOCK: u8 = 0xC0;

pub const PHY_FULL_DUP: u8 = 0x01;
pub const PHY_LINK: u8 = 0x02;
pub const PHY_10: u8 = 0x04;
pub const PHY_100: u8 = 0x08;
pub const PHY_1000F: u8 = 0x10;

pub const CPLUS_RX_VLAN: u16 = 1 << 6;
pub const CPLUS_RX_CHKSUM: u16 = 1 << 5;
pub const CPLUS_PCI_DAC: u16 = 1 << 4;

pub const DESC_OWN: u32 = 1 << 31;
pub const DESC_RING_END: u32 = 1 << 30;
pub const DESC_FIRST: u32 = 1 << 29;
pub const DESC_LAST: u32 = 1 << 28;
/// Chyba prijmu (souhrnny bit; CRC, runt, watchdog).
pub const RX_RES: u32 = 1 << 21;
/// Delka v `opts1` (prijem i odesilani), bity 13:0.
pub const LEN_MASK: u32 = 0x3FFF;
const FCS: usize = 4;

pub struct R8169<H: Hw> {
    hw: H,
    mac: [u8; 6],
    rx_next: usize,
    tx: TxRing,
    stats: Stats,
    notes: Vec<&'static str>,
}

/// Bit "konec kruhu" pro deskriptor `i`.
fn ring_end(i: usize) -> u32 {
    if i == RING - 1 { DESC_RING_END } else { 0 }
}

impl<H: Hw> R8169<H> {
    pub fn open(hw: H, _device: u16, seed: u64) -> Result<Self, HwError> {
        let mut nic = Self {
            hw,
            mac: [0; 6],
            rx_next: 0,
            tx: TxRing::default(),
            stats: Stats::default(),
            notes: Vec::new(),
        };
        if nic.hw.read32(TX_CONFIG) == u32::MAX {
            return Err(HwError::Gone);
        }
        nic.reset()?;
        nic.mac = nic.read_mac(seed);
        nic.setup();
        Ok(nic)
    }

    fn reset(&mut self) -> Result<(), HwError> {
        let hw = &self.hw;
        hw.write16(INTR_MASK, 0);
        hw.write16(INTR_STATUS, 0xFFFF);
        hw.write8(CHIP_CMD, CMD_RESET);
        if !wait_for(hw, 100_000, 100, || hw.read8(CHIP_CMD) & CMD_RESET == 0) {
            return Err(HwError::Timeout("reset cipu"));
        }
        Ok(())
    }

    fn read_mac(&mut self, seed: u64) -> [u8; 6] {
        let hw = &self.hw;
        let mut m = [0u8; 6];
        for (i, b) in m.iter_mut().enumerate() {
            *b = hw.read8(MAC0 + i as u32);
        }
        if !valid_mac(&m) || m == [0xFF; 6] {
            self.notes.push("karta nema MAC, pouzita nahodna mistni");
            m = local_mac(seed);
            let hw = &self.hw;
            hw.write8(CFG9346, CFG9346_UNLOCK);
            // Horni cast driv, zapis dolni ji potvrdi (jako Linux).
            hw.write32(MAC4, u32::from(m[4]) | u32::from(m[5]) << 8);
            hw.read32(MAC4);
            hw.write32(MAC0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
            hw.read32(MAC0);
            hw.write8(CFG9346, CFG9346_LOCK);
        }
        m
    }

    fn setup(&mut self) {
        let hw = &self.hw;
        let base = hw.dma_addr();
        for i in 0..RING {
            self.post_rx(i);
            let t = Layout::tx_desc(i);
            hw.dma_write(t, &[0u8; 16]);
            hw.dma_write32(t, ring_end(i));
        }
        hw.dma_wmb();
        hw.write8(CFG9346, CFG9346_UNLOCK);
        // Prijate ramce do jednoho bufferu: delsi zahodit.
        hw.write16(RX_MAX_SIZE, BUF as u16);
        // Bez offloadu kontrolnich souctu a VLAN; adresy nad 4 GiB.
        let mut cp = hw.read16(CPLUS_CMD) & !(CPLUS_RX_VLAN | CPLUS_RX_CHKSUM);
        if (base + Layout::SIZE as u64) >> 32 != 0 {
            cp |= CPLUS_PCI_DAC;
        }
        hw.write16(CPLUS_CMD, cp);
        hw.write16(INTR_MITIGATE, 0);
        write_addr(hw, TX_DESC_LO, TX_DESC_HI, base + Layout::TX_RING as u64);
        write_addr(hw, RX_DESC_LO, RX_DESC_HI, base + Layout::RX_RING as u64);
        // Nejdelsi odesilany ramec v jednotkach 128 B (RTL8168).
        hw.write8(MAX_TX_PACKET_SIZE, (BUF / 128) as u8);
        hw.write8(CHIP_CMD, CMD_TX_ENB | CMD_RX_ENB);
        hw.write32(
            RX_CONFIG,
            RX128_INT_EN
                | RX_MULTI_EN
                | RX_DMA_BURST
                | RX_ACCEPT_BROADCAST
                | RX_ACCEPT_MULTICAST
                | RX_ACCEPT_MY_PHYS,
        );
        hw.write32(TX_CONFIG, TX_CONFIG_VALUE);
        // Vsechny skupinove adresy.
        hw.write32(MAR0, u32::MAX);
        hw.write32(MAR0 + 4, u32::MAX);
        hw.write8(CFG9346, CFG9346_LOCK);
        hw.write16(INTR_STATUS, 0xFFFF);
        hw.write16(INTR_MASK, INT_MASK);
    }

    /// Deskriptor prijmu `i` predat karte.
    fn post_rx(&self, i: usize) {
        let hw = &self.hw;
        let d = Layout::rx_desc(i);
        hw.dma_write32(d + 4, 0);
        hw.dma_write(d + 8, &(hw.dma_addr() + Layout::rx_buf(i) as u64).to_le_bytes());
        // Vlastnik az nakonec: karta nesmi videt deskriptor bez adresy.
        hw.dma_wmb();
        hw.dma_write32(d, DESC_OWN | ring_end(i) | BUF as u32);
    }

    pub fn hw(&self) -> &H {
        &self.hw
    }
}

impl<H: Hw> Nic for R8169<H> {
    fn model(&self) -> &'static str {
        "RTL8111/8168"
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link(&self) -> Result<LinkState, HwError> {
        let st = self.hw.read8(PHY_STATUS);
        if st == u8::MAX && self.hw.read32(TX_CONFIG) == u32::MAX {
            return Err(HwError::Gone);
        }
        let up = st & PHY_LINK != 0;
        let speed = if !up {
            0
        } else if st & PHY_1000F != 0 {
            1000
        } else if st & PHY_100 != 0 {
            100
        } else if st & PHY_10 != 0 {
            10
        } else {
            0
        };
        Ok(LinkState {
            up,
            speed_mbps: speed,
            full_duplex: up && (st & PHY_FULL_DUP != 0 || st & PHY_1000F != 0),
        })
    }

    fn interrupt(&mut self) -> Result<Events, HwError> {
        let st = self.hw.read16(INTR_STATUS);
        if st == u16::MAX && self.hw.read32(TX_CONFIG) == u32::MAX {
            return Err(HwError::Gone);
        }
        // Potvrzeni zapisem jednicek.
        if st != 0 {
            self.hw.write16(INTR_STATUS, st);
        }
        Ok(Events {
            link: st & INT_LINK_CHG != 0,
            fatal: st & INT_SYS_ERR != 0,
        })
    }

    fn receive(&mut self, deliver: &mut dyn FnMut(&[u8])) -> Result<(), HwError> {
        let mut frame = [0u8; BUF];
        let mut done = 0;
        while done < RING {
            let i = self.rx_next;
            let d = Layout::rx_desc(i);
            let opts1 = self.hw.dma_read32(d);
            if opts1 & DESC_OWN != 0 {
                break;
            }
            self.hw.dma_rmb();
            let whole = opts1 & (DESC_FIRST | DESC_LAST) == DESC_FIRST | DESC_LAST;
            let len = ((opts1 & LEN_MASK) as usize).saturating_sub(FCS);
            if whole && opts1 & RX_RES == 0 && (14..=MAX_FRAME).contains(&len) {
                self.hw.dma_read(Layout::rx_buf(i), &mut frame[..len]);
                self.stats.rx_frames += 1;
                deliver(&frame[..len]);
            } else {
                self.stats.rx_errors += 1;
            }
            self.post_rx(i);
            self.rx_next = (i + 1) % RING;
            done += 1;
        }
        Ok(())
    }

    fn reclaim(&mut self) -> Result<(), HwError> {
        while self.tx.in_flight() > 0 {
            if self.hw.dma_read32(Layout::tx_desc(self.tx.head)) & DESC_OWN != 0 {
                break;
            }
            self.tx.head = (self.tx.head + 1) % RING;
        }
        Ok(())
    }

    fn tx_space(&self) -> usize {
        self.tx.space()
    }

    fn send(&mut self, frame: &[u8]) {
        let i = self.tx.tail;
        let hw = &self.hw;
        let len = copy_tx(hw, Layout::tx_buf(i), frame) as u32;
        let d = Layout::tx_desc(i);
        hw.dma_write32(d + 4, 0);
        hw.dma_write(d + 8, &(hw.dma_addr() + Layout::tx_buf(i) as u64).to_le_bytes());
        hw.dma_wmb();
        hw.dma_write32(d, DESC_OWN | ring_end(i) | DESC_FIRST | DESC_LAST | len);
        self.tx.tail = (i + 1) % RING;
        self.stats.tx_frames += 1;
    }

    fn kick(&mut self) {
        self.hw.dma_wmb();
        self.hw.mmio_wmb();
        self.hw.write8(TX_POLL, TX_POLL_NPQ);
    }

    fn stop(&mut self) {
        let hw = &self.hw;
        hw.write16(INTR_MASK, 0);
        hw.write8(CHIP_CMD, 0);
        hw.write8(CHIP_CMD, CMD_RESET);
        wait_for(hw, 100_000, 100, || hw.read8(CHIP_CMD) & CMD_RESET == 0);
    }

    fn stats(&self) -> Stats {
        self.stats
    }

    fn notes(&self) -> &[&'static str] {
        &self.notes
    }
}
