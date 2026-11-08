//! Intel igc: I225 a I226 (2,5 Gb/s), I220-V, I221-V. Pokrocile
//! ("advanced") deskriptory 16 B, fronta 0 prijmu a odesilani, preruseni
//! v rezimu MSI pres `ICR` (bez MSI-X vektoru pro fronty).
//!
//! Inicializace (podle ovladace Linuxu igc):
//!
//! 1. zakazat preruseni, zastavit prijem a odesilani, odpojit DMA,
//! 2. reset zarizeni (`CTRL.DEV_RST`), pockat na nacteni NVM (`EECD.AUTO_RD`),
//! 3. MAC z `RAL0/RAH0`, jinak nahodna mistni,
//! 4. linka: `CTRL.SLU`, rychlost z autonegociace PHY (vychozi stav PHY
//!    po resetu nabizi vsechny rychlosti do 2,5 Gb/s),
//! 5. fronta prijmu: `SRRCTL` (buffer 2 KiB, jeden buffer na ramec),
//!    `RXDCTL.ENABLE`; fronta odesilani: `TXDCTL.ENABLE`; `RCTL`, `TCTL`.
//!
//! **Overeni:** jen testy na hostiteli nad simulovanymi registry. QEMU
//! (ani 11.1) model I225 nema (`-device igb` je 82576, jiny cip); na
//! skutecnem hardwaru ovladac overeny neni. Proti Linuxu chybi nastaveni
//! PHY (vychozi hodnoty po resetu), EEE a opravy jednotlivych revizi.
//!
//! **Puvod:** vlastni kod. Z ovladacu Linuxu (GPL-2) a datasheetu vyrobce
//! jsou prevzata jen fakta - offsety a bity registru, poradi inicializace
//! a chyby cipu; zadny kod z nich prelozeny neni. Kod z BSD neprevzat.

use alloc::vec::Vec;

use crate::{
    BUF, Events, Hw, HwError, Layout, LinkState, MAX_FRAME, Nic, RING, Stats, TxRing, copy_tx, local_mac,
    valid_mac, wait_for, write_addr,
};

pub const CTRL: u32 = 0x0000;
pub const STATUS: u32 = 0x0008;
pub const EECD: u32 = 0x0010;
pub const ICR: u32 = 0x1500;
pub const IMS: u32 = 0x1508;
pub const IMC: u32 = 0x150C;
pub const EIMC: u32 = 0x1528;
pub const RCTL: u32 = 0x0100;
pub const TCTL: u32 = 0x0400;
pub const RDBAL: u32 = 0xC000;
pub const RDBAH: u32 = 0xC004;
pub const RDLEN: u32 = 0xC008;
pub const SRRCTL: u32 = 0xC00C;
pub const RDH: u32 = 0xC010;
pub const RDT: u32 = 0xC018;
pub const RXDCTL: u32 = 0xC028;
pub const TDBAL: u32 = 0xE000;
pub const TDBAH: u32 = 0xE004;
pub const TDLEN: u32 = 0xE008;
pub const TDH: u32 = 0xE010;
pub const TDT: u32 = 0xE018;
pub const TXDCTL: u32 = 0xE028;
pub const MTA: u32 = 0x5200;
pub const RAL0: u32 = 0x5400;
pub const RAH0: u32 = 0x5404;

pub const CTRL_GIO_MASTER_DISABLE: u32 = 1 << 2;
pub const CTRL_SLU: u32 = 1 << 6;
pub const CTRL_FRCSPD: u32 = 1 << 11;
pub const CTRL_FRCDPX: u32 = 1 << 12;
pub const CTRL_DEV_RST: u32 = 1 << 29;
pub const STATUS_FD: u32 = 1 << 0;
pub const STATUS_LU: u32 = 1 << 1;
pub const STATUS_SPEED_100: u32 = 1 << 6;
pub const STATUS_SPEED_1000: u32 = 1 << 7;
pub const STATUS_GIO_MASTER_EN: u32 = 1 << 19;
pub const STATUS_SPEED_2500: u32 = 1 << 22;
pub const EECD_AUTO_RD: u32 = 1 << 9;

pub const INT_TXDW: u32 = 1 << 0;
pub const INT_LSC: u32 = 1 << 2;
pub const INT_RXSEQ: u32 = 1 << 3;
pub const INT_RXDMT0: u32 = 1 << 4;
pub const INT_RXT0: u32 = 1 << 7;
const INT_MASK: u32 = INT_TXDW | INT_LSC | INT_RXSEQ | INT_RXDMT0 | INT_RXT0;

pub const RCTL_EN: u32 = 1 << 1;
pub const RCTL_MPE: u32 = 1 << 4;
pub const RCTL_BAM: u32 = 1 << 15;
pub const RCTL_SECRC: u32 = 1 << 26;
pub const TCTL_EN: u32 = 1 << 1;
pub const TCTL_PSP: u32 = 1 << 3;
const TCTL_CT: u32 = 0x0F << 4;
const TCTL_RTLC: u32 = 1 << 24;

/// SRRCTL: buffer paketu v KiB (bity 6:0), typ deskriptoru "advanced, one
/// buffer" (bity 27:25 = 001), zahodit pri plne fronte.
pub const SRRCTL_BSIZE_2K: u32 = 2;
pub const SRRCTL_ADV_ONEBUF: u32 = 1 << 25;
pub const SRRCTL_DROP_EN: u32 = 1 << 31;
pub const QUEUE_ENABLE: u32 = 1 << 25;
/// Prahy fronty prijmu (PTHRESH 8, HTHRESH 8, WTHRESH 4) jako Linux.
const RXDCTL_THRESH: u32 = 8 | 8 << 8 | 4 << 16;
/// Fronta odesilani: zpetny zapis po kazdem deskriptoru (WTHRESH 1).
const TXDCTL_THRESH: u32 = 8 | 1 << 8 | 1 << 16;

// Deskriptor prijmu: cteni = adresa paketu (+0), adresa hlavicky (+8);
// zpetny zapis = stav a chyby u32 (+8), delka u16 (+12).
pub const RXD_DD: u32 = 1 << 0;
pub const RXD_EOP: u32 = 1 << 1;
/// Chyba ramce (RXE) v poli chyb zpetneho zapisu.
pub const RXD_ERR_RXE: u32 = 1 << 31;
// Deskriptor odesilani (datovy): adresa (+0), cmd_type_len (+8),
// olinfo_status (+12, delka dat od bitu 14; DD = bit 0 po odeslani).
pub const TXD_DTYP_DATA: u32 = 3 << 20;
pub const TXD_CMD_EOP: u32 = 1 << 24;
pub const TXD_CMD_IFCS: u32 = 1 << 25;
pub const TXD_CMD_RS: u32 = 1 << 27;
pub const TXD_CMD_DEXT: u32 = 1 << 29;
pub const TXD_PAYLEN_SHIFT: u32 = 14;
pub const TXD_DD: u32 = 1 << 0;

pub struct Igc<H: Hw> {
    hw: H,
    model: &'static str,
    mac: [u8; 6],
    rx_next: usize,
    tx: TxRing,
    stats: Stats,
    notes: Vec<&'static str>,
}

pub fn model(device: u16) -> &'static str {
    match device {
        0x125B..=0x125D | 0x125F | 0x3102 | 0x5503 => "I226",
        0x15F7 => "I220-V",
        0x125E => "I221-V",
        _ => "I225",
    }
}

impl<H: Hw> Igc<H> {
    pub fn open(hw: H, device: u16, seed: u64) -> Result<Self, HwError> {
        let mut nic = Self {
            hw,
            model: model(device),
            mac: [0; 6],
            rx_next: 0,
            tx: TxRing::default(),
            stats: Stats::default(),
            notes: Vec::new(),
        };
        if nic.hw.read32(STATUS) == u32::MAX {
            return Err(HwError::Gone);
        }
        nic.reset()?;
        nic.mac = nic.read_mac(seed);
        nic.setup()?;
        Ok(nic)
    }

    fn quiesce(&self) {
        let hw = &self.hw;
        hw.write32(IMC, u32::MAX);
        hw.write32(EIMC, u32::MAX);
        hw.write32(RCTL, 0);
        hw.write32(TCTL, TCTL_PSP);
        hw.read32(STATUS);
    }

    fn reset(&mut self) -> Result<(), HwError> {
        let hw = &self.hw;
        let ctrl = hw.read32(CTRL);
        hw.write32(CTRL, ctrl | CTRL_GIO_MASTER_DISABLE);
        if !wait_for(hw, 80_000, 100, || hw.read32(STATUS) & STATUS_GIO_MASTER_EN == 0) {
            self.notes.push("DMA se pred resetem nezastavilo");
        }
        self.quiesce();
        let hw = &self.hw;
        hw.sleep_us(10_000);
        let ctrl = hw.read32(CTRL);
        hw.write32(CTRL, ctrl | CTRL_DEV_RST);
        hw.sleep_us(1_000);
        if !wait_for(hw, 100_000, 1_000, || hw.read32(CTRL) & CTRL_DEV_RST == 0) {
            return Err(HwError::Timeout("reset zarizeni"));
        }
        if !wait_for(hw, 10_000, 1_000, || hw.read32(EECD) & EECD_AUTO_RD != 0) {
            self.notes.push("NVM se po resetu nenacetla");
        }
        hw.write32(IMC, u32::MAX);
        hw.read32(ICR);
        Ok(())
    }

    fn read_mac(&mut self, seed: u64) -> [u8; 6] {
        let hw = &self.hw;
        let lo = hw.read32(RAL0).to_le_bytes();
        let hi = hw.read32(RAH0);
        let mut m = [lo[0], lo[1], lo[2], lo[3], hi as u8, (hi >> 8) as u8];
        if hi & (1 << 31) == 0 || !valid_mac(&m) {
            // Karta bez NVM (ID ..._BLANK_NVM) MAC nema.
            self.notes.push("karta nema MAC, pouzita nahodna mistni");
            m = local_mac(seed);
            let hw = &self.hw;
            hw.write32(RAL0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
            hw.write32(RAH0, u32::from(m[4]) | u32::from(m[5]) << 8 | 1 << 31);
        }
        m
    }

    fn setup(&mut self) -> Result<(), HwError> {
        let hw = &self.hw;
        for i in 0..128 {
            hw.write32(MTA + i * 4, 0);
        }
        let ctrl = hw.read32(CTRL) & !(CTRL_FRCSPD | CTRL_FRCDPX);
        hw.write32(CTRL, ctrl | CTRL_SLU);

        let base = hw.dma_addr();
        for i in 0..RING {
            self.post_rx(i);
            hw.dma_write(Layout::tx_desc(i), &[0u8; 16]);
        }
        hw.dma_wmb();
        // Prijem: fronta vypnuta behem nastaveni.
        hw.write32(RXDCTL, 0);
        write_addr(hw, RDBAL, RDBAH, base + Layout::RX_RING as u64);
        hw.write32(RDLEN, (RING * crate::DESC) as u32);
        hw.write32(SRRCTL, SRRCTL_BSIZE_2K | SRRCTL_ADV_ONEBUF | SRRCTL_DROP_EN);
        hw.write32(RDH, 0);
        hw.write32(RDT, 0);
        hw.write32(RXDCTL, QUEUE_ENABLE | RXDCTL_THRESH);
        if !wait_for(hw, 10_000, 100, || hw.read32(RXDCTL) & QUEUE_ENABLE != 0) {
            return Err(HwError::Timeout("zapnuti fronty prijmu"));
        }
        hw.write32(RDT, (RING - 1) as u32);
        // Odesilani.
        hw.write32(TXDCTL, 0);
        write_addr(hw, TDBAL, TDBAH, base + Layout::TX_RING as u64);
        hw.write32(TDLEN, (RING * crate::DESC) as u32);
        hw.write32(TDH, 0);
        hw.write32(TDT, 0);
        hw.write32(TXDCTL, QUEUE_ENABLE | TXDCTL_THRESH);
        if !wait_for(hw, 10_000, 100, || hw.read32(TXDCTL) & QUEUE_ENABLE != 0) {
            return Err(HwError::Timeout("zapnuti fronty odesilani"));
        }
        hw.write32(TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_RTLC);
        hw.write32(RCTL, RCTL_EN | RCTL_MPE | RCTL_BAM | RCTL_SECRC);
        hw.read32(ICR);
        hw.write32(IMS, INT_MASK);
        Ok(())
    }

    /// Deskriptor prijmu `i` ve stavu "cteni" (zpetny zapis ho prepsal).
    fn post_rx(&self, i: usize) {
        let hw = &self.hw;
        let d = Layout::rx_desc(i);
        hw.dma_write(d, &(hw.dma_addr() + Layout::rx_buf(i) as u64).to_le_bytes());
        // Adresa hlavicky 0 = bez rozdeleni; zaroven nuluje stav (DD).
        hw.dma_write(d + 8, &[0u8; 8]);
    }

    pub fn hw(&self) -> &H {
        &self.hw
    }
}

impl<H: Hw> Nic for Igc<H> {
    fn model(&self) -> &'static str {
        self.model
    }

    fn mac(&self) -> [u8; 6] {
        self.mac
    }

    fn link(&self) -> Result<LinkState, HwError> {
        let st = self.hw.read32(STATUS);
        if st == u32::MAX {
            return Err(HwError::Gone);
        }
        let up = st & STATUS_LU != 0;
        let speed = if !up {
            0
        } else if st & STATUS_SPEED_1000 != 0 {
            if st & STATUS_SPEED_2500 != 0 { 2500 } else { 1000 }
        } else if st & STATUS_SPEED_100 != 0 {
            100
        } else {
            10
        };
        Ok(LinkState {
            up,
            speed_mbps: speed,
            full_duplex: up && st & STATUS_FD != 0,
        })
    }

    fn interrupt(&mut self) -> Result<Events, HwError> {
        let icr = self.hw.read32(ICR);
        if icr == u32::MAX && self.hw.read32(STATUS) == u32::MAX {
            return Err(HwError::Gone);
        }
        Ok(Events {
            link: icr & (INT_LSC | INT_RXSEQ) != 0,
            fatal: false,
        })
    }

    fn receive(&mut self, deliver: &mut dyn FnMut(&[u8])) -> Result<(), HwError> {
        let mut frame = [0u8; BUF];
        let mut done = 0;
        while done < RING {
            let i = self.rx_next;
            let d = Layout::rx_desc(i);
            let st = self.hw.dma_read32(d + 8);
            if st & RXD_DD == 0 {
                break;
            }
            self.hw.dma_rmb();
            let len = (self.hw.dma_read32(d + 12) & 0xFFFF) as usize;
            if st & RXD_EOP != 0 && st & RXD_ERR_RXE == 0 && (14..=MAX_FRAME).contains(&len) {
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
        if done > 0 {
            self.hw.dma_wmb();
            self.hw.mmio_wmb();
            self.hw.write32(RDT, ((self.rx_next + RING - 1) % RING) as u32);
        }
        Ok(())
    }

    fn reclaim(&mut self) -> Result<(), HwError> {
        while self.tx.in_flight() > 0 {
            if self.hw.dma_read32(Layout::tx_desc(self.tx.head) + 12) & TXD_DD == 0 {
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
        hw.dma_write(d, &(hw.dma_addr() + Layout::tx_buf(i) as u64).to_le_bytes());
        hw.dma_write32(d + 8, len | TXD_DTYP_DATA | TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS | TXD_CMD_DEXT);
        hw.dma_write32(d + 12, len << TXD_PAYLEN_SHIFT);
        self.tx.tail = (i + 1) % RING;
        self.stats.tx_frames += 1;
    }

    fn kick(&mut self) {
        self.hw.dma_wmb();
        self.hw.mmio_wmb();
        self.hw.write32(TDT, self.tx.tail as u32);
    }

    fn stop(&mut self) {
        self.quiesce();
        self.hw.write32(RXDCTL, 0);
        self.hw.write32(TXDCTL, 0);
        let ctrl = self.hw.read32(CTRL);
        self.hw.write32(CTRL, ctrl | CTRL_DEV_RST);
        self.hw.sleep_us(10_000);
    }

    fn stats(&self) -> Stats {
        self.stats
    }

    fn notes(&self) -> &[&'static str] {
        &self.notes
    }
}
