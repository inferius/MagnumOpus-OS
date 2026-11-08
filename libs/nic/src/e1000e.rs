//! Intel e1000e: 82574L/LA, 82583V (samostatne radice) a I217/I218/I219
//! (radic v cipove sade PCH). Klasicke ("legacy") deskriptory 16 B,
//! jedna fronta prijmu a odesilani.
//!
//! Inicializace (poradi podle datasheetu 82574 a ovladace Linuxu):
//!
//! 1. zakazat preruseni, zastavit prijem a odesilani, odpojit DMA
//!    (`GIO_MASTER_DISABLE`),
//! 2. reset MAC (`CTRL.RST`) - PHY se neresetuje (u PCH by po resetu PHY
//!    bylo nutne znovu nahrat konfiguraci OEM z NVM),
//! 3. pockat na nacteni NVM (82574: `EECD.AUTO_RD`, PCH: `STATUS.LAN_INIT_DONE`),
//! 4. PCH: overit, ze PHY odpovida; po "ultra low power" rezimu (Windows
//!    ho nastavi pri vypnuti bez kabelu) ho probudit prepnutim LANPHYPC,
//! 5. MAC z `RAL0/RAH0` (karta ji tam nahraje z NVM pri resetu), jinak
//!    z EEPROM (`EERD`, jen 82574) nebo nahodna mistni,
//! 6. kruhy, `RCTL`, `TCTL`, preruseni.
//!
//! **Overeni:** 82574L v QEMU (`-device e1000e`, x86_64 i aarch64). Na
//! skutecnem hardwaru ovladac overeny neni; u I217-I219 chybi oproti
//! Linuxu zejmena opravy PHY pro jednotlive revize (K1, jumbo, ULP pres
//! ME) a vyprazdneni kruhu po zaseknuti (I219, `FLUSH_DESC_REQUIRED`).
//!
//! **Puvod:** vlastni kod. Z ovladacu Linuxu (GPL-2) a datasheetu vyrobce
//! jsou prevzata jen fakta - offsety a bity registru, poradi inicializace
//! a chyby cipu; zadny kod z nich prelozeny neni. Kod z BSD neprevzat.

use alloc::vec::Vec;

use crate::ids::E1000eKind;
use crate::{
    BUF, Events, Hw, HwError, Layout, LinkState, MAX_FRAME, Nic, RING, Stats, TxRing, copy_tx, local_mac,
    valid_mac, wait_for, write_addr,
};

// Registry (offsety v BAR0).
pub const CTRL: u32 = 0x0000;
pub const STATUS: u32 = 0x0008;
pub const EECD: u32 = 0x0010;
pub const EERD: u32 = 0x0014;
pub const CTRL_EXT: u32 = 0x0018;
pub const MDIC: u32 = 0x0020;
pub const FEXTNVM3: u32 = 0x003C;
pub const ICR: u32 = 0x00C0;
pub const ITR: u32 = 0x00C4;
pub const IMS: u32 = 0x00D0;
pub const IMC: u32 = 0x00D8;
pub const RCTL: u32 = 0x0100;
pub const TCTL: u32 = 0x0400;
pub const TIPG: u32 = 0x0410;
pub const EXTCNF_CTRL: u32 = 0x0F00;
pub const RDBAL: u32 = 0x2800;
pub const RDBAH: u32 = 0x2804;
pub const RDLEN: u32 = 0x2808;
pub const RDH: u32 = 0x2810;
pub const RDT: u32 = 0x2818;
pub const RDTR: u32 = 0x2820;
pub const RADV: u32 = 0x282C;
pub const TDBAL: u32 = 0x3800;
pub const TDBAH: u32 = 0x3804;
pub const TDLEN: u32 = 0x3808;
pub const TDH: u32 = 0x3810;
pub const TDT: u32 = 0x3818;
pub const TXDCTL: u32 = 0x3828;
pub const MTA: u32 = 0x5200;
pub const RAL0: u32 = 0x5400;
pub const RAH0: u32 = 0x5404;
pub const FWSM: u32 = 0x5B54;

// CTRL
pub const CTRL_GIO_MASTER_DISABLE: u32 = 1 << 2;
pub const CTRL_SLU: u32 = 1 << 6;
pub const CTRL_FRCSPD: u32 = 1 << 11;
pub const CTRL_FRCDPX: u32 = 1 << 12;
pub const CTRL_LANPHYPC_OVERRIDE: u32 = 1 << 16;
pub const CTRL_LANPHYPC_VALUE: u32 = 1 << 17;
pub const CTRL_RST: u32 = 1 << 26;
pub const CTRL_PHY_RST: u32 = 1 << 31;
// STATUS
pub const STATUS_FD: u32 = 1 << 0;
pub const STATUS_LU: u32 = 1 << 1;
pub const STATUS_LAN_INIT_DONE: u32 = 1 << 9;
pub const STATUS_GIO_MASTER_EN: u32 = 1 << 19;
// EECD / EERD
pub const EECD_AUTO_RD: u32 = 1 << 9;
pub const EERD_START: u32 = 1 << 0;
pub const EERD_DONE: u32 = 1 << 1;
// CTRL_EXT
pub const CTRL_EXT_LPCD: u32 = 1 << 2;
pub const CTRL_EXT_FORCE_SMBUS: u32 = 1 << 11;
// FEXTNVM3: doba konfigurace PHY po probuzeni.
const FEXTNVM3_PHY_CFG_COUNTER_MASK: u32 = 3 << 26;
const FEXTNVM3_PHY_CFG_COUNTER_50MS: u32 = 1 << 26;
// MDIC (pristup k registrum PHY)
pub const MDIC_OP_READ: u32 = 2 << 26;
pub const MDIC_READY: u32 = 1 << 28;
pub const MDIC_ERROR: u32 = 1 << 30;
// EXTCNF_CTRL: semafor PHY mezi ovladacem a firmwarem (ME).
pub const EXTCNF_SWFLAG: u32 = 1 << 5;
// FWSM: firmware ME bezi / reset PHY povolen.
const FWSM_RSPCIPHY: u32 = 1 << 6;
// Priciny preruseni (ICR/IMS)
pub const INT_TXDW: u32 = 1 << 0;
pub const INT_LSC: u32 = 1 << 2;
pub const INT_RXSEQ: u32 = 1 << 3;
pub const INT_RXDMT0: u32 = 1 << 4;
pub const INT_RXO: u32 = 1 << 6;
pub const INT_RXT0: u32 = 1 << 7;
const INT_MASK: u32 = INT_TXDW | INT_LSC | INT_RXSEQ | INT_RXDMT0 | INT_RXO | INT_RXT0;
// RCTL: buffer 2048 B (BSIZE 00, BSEX 0), vsechny skupinove (MPE),
// vsesmerove (BAM), bez FCS (SECRC).
pub const RCTL_EN: u32 = 1 << 1;
pub const RCTL_MPE: u32 = 1 << 4;
pub const RCTL_BAM: u32 = 1 << 15;
pub const RCTL_SECRC: u32 = 1 << 26;
// TCTL: doplnovat kratke ramce (PSP), kolize podle 802.3 pro plny duplex.
pub const TCTL_EN: u32 = 1 << 1;
pub const TCTL_PSP: u32 = 1 << 3;
const TCTL_CT: u32 = 0x0F << 4;
const TCTL_COLD: u32 = 0x3F << 12;
const TCTL_RTLC: u32 = 1 << 24;
/// Mezery mezi ramci pro metaliku (IPGT 8, IPGR1 8, IPGR2 6).
const TIPG_COPPER: u32 = 8 | 8 << 10 | 6 << 20;
// TXDCTL: zpetny zapis po kazdem deskriptoru (WTHRESH 1, GRAN), jak ho
// Linux nastavuje pro 82571+ a PCH.
const TXDCTL_WTHRESH_MASK: u32 = 0x3F << 16;
const TXDCTL_FULL_TX_DESC_WB: u32 = 0x0101_0000;
const TXDCTL_COUNT_DESC: u32 = 1 << 22;
/// Omezeni preruseni v jednotkach 256 ns: 195 = nejvys ~20 000 za sekundu.
const ITR_DEFAULT: u32 = 195;

// Deskriptory: prijem = adresa, delka u16 (+8), soucet u16, stav u8
// (+12), chyby u8 (+13); odesilani = adresa, delka u16 (+8), CSO u8,
// prikaz u8 (+11), stav u8 (+12).
pub const RXD_DD: u32 = 1 << 0;
pub const RXD_EOP: u32 = 1 << 1;
/// Chyby prijmu v bajtu +13: CE, SE, SEQ, RXE.
pub const RXD_ERRORS: u32 = 0x87 << 8;
pub const TXD_CMD_EOP: u32 = 1 << 24;
pub const TXD_CMD_IFCS: u32 = 1 << 25;
pub const TXD_CMD_RS: u32 = 1 << 27;
pub const TXD_DD: u32 = 1 << 0;

/// Adresy PHY, na kterych ho hledat (PCH: 1 nebo 2 podle revize).
const PHY_ADDRS: [u32; 2] = [1, 2];
/// Registr PHY s prvni casti identifikatoru.
const PHY_ID1: u32 = 2;

pub struct E1000e<H: Hw> {
    hw: H,
    kind: E1000eKind,
    model: &'static str,
    mac: [u8; 6],
    rx_next: usize,
    tx: TxRing,
    stats: Stats,
    notes: Vec<&'static str>,
}

/// Jmeno cipu pro hlaseni.
pub fn model(device: u16) -> &'static str {
    match device {
        0x10D3 => "82574L",
        0x10F6 => "82574LA",
        0x150C => "82583V",
        0x153A | 0x153B => "I217",
        0x155A | 0x1559 | 0x15A0..=0x15A3 => "I218",
        _ => "I219",
    }
}

impl<H: Hw> E1000e<H> {
    /// Reset a nastaveni karty `device` (ID PCI). `seed` = nahoda pro
    /// mistni MAC, kdyz karta zadnou nema.
    pub fn open(hw: H, device: u16, seed: u64) -> Result<Self, HwError> {
        let kind = crate::ids::e1000e_kind(device).ok_or(HwError::Corrupted("nezname ID e1000e"))?;
        let mut nic = Self {
            hw,
            kind,
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
        if kind == E1000eKind::Pch {
            nic.wake_phy();
        }
        nic.mac = nic.read_mac(seed);
        nic.setup();
        Ok(nic)
    }

    fn quiesce(&self) {
        let hw = &self.hw;
        hw.write32(IMC, u32::MAX);
        hw.write32(RCTL, 0);
        hw.write32(TCTL, TCTL_PSP);
        hw.read32(STATUS);
    }

    fn reset(&mut self) -> Result<(), HwError> {
        self.quiesce();
        self.hw.sleep_us(10_000);
        // Nejdriv dokoncit rozpracovane DMA, pak reset.
        let ctrl = self.hw.read32(CTRL);
        self.hw.write32(CTRL, ctrl | CTRL_GIO_MASTER_DISABLE);
        if !wait_for(&self.hw, 80_000, 100, || self.hw.read32(STATUS) & STATUS_GIO_MASTER_EN == 0) {
            // Linux pokracuje taky: reset DMA stejne zastavi.
            self.notes.push("DMA se pred resetem nezastavilo");
        }
        // PCH: reset za semaforem (firmware ME nesmi zrovna pracovat s PHY);
        // semafor reset sam uvolni.
        if self.kind == E1000eKind::Pch && !self.swflag_acquire() {
            self.notes.push("firmware drzi semafor PHY, reset bez nej");
        }
        let ctrl = self.hw.read32(CTRL) & !CTRL_PHY_RST;
        self.hw.write32(CTRL, ctrl | CTRL_RST);
        self.hw.sleep_us(20_000);
        if !wait_for(&self.hw, 100_000, 1_000, || self.hw.read32(CTRL) & CTRL_RST == 0) {
            return Err(HwError::Timeout("reset MAC"));
        }
        match self.kind {
            E1000eKind::Discrete => {
                if !wait_for(&self.hw, 10_000, 1_000, || self.hw.read32(EECD) & EECD_AUTO_RD != 0) {
                    self.notes.push("NVM se po resetu nenacetla");
                }
            }
            E1000eKind::Pch => {
                if wait_for(&self.hw, 15_000, 100, || self.hw.read32(STATUS) & STATUS_LAN_INIT_DONE != 0) {
                    let st = self.hw.read32(STATUS);
                    self.hw.write32(STATUS, st & !STATUS_LAN_INIT_DONE);
                } else {
                    self.notes.push("LAN_INIT_DONE po resetu neprisel");
                }
            }
        }
        self.hw.write32(IMC, u32::MAX);
        self.hw.read32(ICR);
        Ok(())
    }

    /// Semafor PHY (PCH): pockat, az ho nikdo nedrzi, a vzit si ho.
    fn swflag_acquire(&self) -> bool {
        let hw = &self.hw;
        if !wait_for(hw, 100_000, 1_000, || hw.read32(EXTCNF_CTRL) & EXTCNF_SWFLAG == 0) {
            return false;
        }
        let v = hw.read32(EXTCNF_CTRL);
        hw.write32(EXTCNF_CTRL, v | EXTCNF_SWFLAG);
        wait_for(hw, 1_000, 100, || hw.read32(EXTCNF_CTRL) & EXTCNF_SWFLAG != 0)
    }

    fn swflag_release(&self) {
        let v = self.hw.read32(EXTCNF_CTRL);
        self.hw.write32(EXTCNF_CTRL, v & !EXTCNF_SWFLAG);
    }

    /// Precte registr PHY pres MDIC. `None` = PHY neodpovedel.
    fn phy_read(&self, addr: u32, reg: u32) -> Option<u16> {
        let hw = &self.hw;
        hw.write32(MDIC, reg << 16 | addr << 21 | MDIC_OP_READ);
        let mut v = 0;
        if !wait_for(hw, 3_200, 50, || {
            v = hw.read32(MDIC);
            v & MDIC_READY != 0
        }) || v & MDIC_ERROR != 0
        {
            return None;
        }
        Some(v as u16)
    }

    /// Odpovida PHY (cte se rozumny identifikator)?
    fn phy_accessible(&self) -> bool {
        if !self.swflag_acquire() {
            return false;
        }
        let ok = PHY_ADDRS
            .iter()
            .any(|&a| self.phy_read(a, PHY_ID1).is_some_and(|id| id != 0 && id != 0xFFFF));
        self.swflag_release();
        ok
    }

    /// PCH: PHY v rezimu nizke spotreby (ULP) neodpovida a linka nenabehne.
    /// Probuzeni jako v Linuxu: zrusit vynuceny SMBus a prepnout LANPHYPC
    /// (napajeni PHY). Kdyz reset PHY blokuje firmware ME, nesahat.
    fn wake_phy(&mut self) {
        if self.phy_accessible() {
            return;
        }
        if self.hw.read32(FWSM) & FWSM_RSPCIPHY == 0 {
            self.notes.push("PHY neodpovida a firmware ME blokuje jeho reset");
            return;
        }
        let hw = &self.hw;
        let ext = hw.read32(CTRL_EXT);
        hw.write32(CTRL_EXT, ext & !CTRL_EXT_FORCE_SMBUS);
        let f = hw.read32(FEXTNVM3);
        hw.write32(FEXTNVM3, (f & !FEXTNVM3_PHY_CFG_COUNTER_MASK) | FEXTNVM3_PHY_CFG_COUNTER_50MS);
        let ctrl = (hw.read32(CTRL) | CTRL_LANPHYPC_OVERRIDE) & !CTRL_LANPHYPC_VALUE;
        hw.write32(CTRL, ctrl);
        hw.read32(STATUS);
        hw.sleep_us(10);
        hw.write32(CTRL, ctrl & !CTRL_LANPHYPC_OVERRIDE);
        hw.read32(STATUS);
        wait_for(hw, 100_000, 5_000, || hw.read32(CTRL_EXT) & CTRL_EXT_LPCD != 0);
        hw.sleep_us(30_000);
        if self.phy_accessible() {
            self.notes.push("PHY probuzen z rezimu nizke spotreby (LANPHYPC)");
        } else {
            self.notes.push("PHY neodpovida ani po probuzeni (LANPHYPC)");
        }
    }

    fn read_mac(&mut self, seed: u64) -> [u8; 6] {
        let hw = &self.hw;
        let lo = hw.read32(RAL0).to_le_bytes();
        let hi = hw.read32(RAH0);
        let mut m = [lo[0], lo[1], lo[2], lo[3], hi as u8, (hi >> 8) as u8];
        if hi & (1 << 31) == 0 || !valid_mac(&m) {
            m = match self.kind {
                E1000eKind::Discrete => self.eeprom_mac().unwrap_or([0; 6]),
                // NVM PCH je ve flash BIOSu (BAR1): bez nej jen mistni.
                E1000eKind::Pch => [0; 6],
            };
            if !valid_mac(&m) {
                self.notes.push("karta nema MAC, pouzita nahodna mistni");
                m = local_mac(seed);
            }
            let hw = &self.hw;
            hw.write32(RAL0, u32::from_le_bytes([m[0], m[1], m[2], m[3]]));
            hw.write32(RAH0, u32::from(m[4]) | u32::from(m[5]) << 8 | 1 << 31);
        }
        m
    }

    /// MAC ze slov 0-2 EEPROM (82574).
    fn eeprom_mac(&self) -> Option<[u8; 6]> {
        let hw = &self.hw;
        let mut m = [0u8; 6];
        for w in 0..3u32 {
            hw.write32(EERD, w << 2 | EERD_START);
            let mut v = 0;
            if !wait_for(hw, 10_000, 5, || {
                v = hw.read32(EERD);
                v & EERD_DONE != 0
            }) {
                return None;
            }
            let word = (v >> 16) as u16;
            m[w as usize * 2..w as usize * 2 + 2].copy_from_slice(&word.to_le_bytes());
        }
        Some(m)
    }

    fn setup(&mut self) {
        let hw = &self.hw;
        for i in 0..128 {
            hw.write32(MTA + i * 4, 0);
        }
        // Linka: rychlost a duplex z autonegociace PHY.
        let ctrl = hw.read32(CTRL) & !(CTRL_FRCSPD | CTRL_FRCDPX | CTRL_PHY_RST);
        hw.write32(CTRL, ctrl | CTRL_SLU);

        let base = hw.dma_addr();
        for i in 0..RING {
            let d = Layout::rx_desc(i);
            hw.dma_write(d, &(base + Layout::rx_buf(i) as u64).to_le_bytes());
            hw.dma_write32(d + 8, 0);
            hw.dma_write32(d + 12, 0);
            let t = Layout::tx_desc(i);
            hw.dma_write(t, &[0u8; 16]);
        }
        hw.dma_wmb();
        write_addr(hw, RDBAL, RDBAH, base + Layout::RX_RING as u64);
        hw.write32(RDLEN, (RING * crate::DESC) as u32);
        hw.write32(RDH, 0);
        hw.write32(RDT, (RING - 1) as u32);
        hw.write32(RDTR, 0);
        hw.write32(RADV, 0);
        write_addr(hw, TDBAL, TDBAH, base + Layout::TX_RING as u64);
        hw.write32(TDLEN, (RING * crate::DESC) as u32);
        hw.write32(TDH, 0);
        hw.write32(TDT, 0);
        let txdctl = hw.read32(TXDCTL) & !TXDCTL_WTHRESH_MASK;
        hw.write32(TXDCTL, txdctl | TXDCTL_FULL_TX_DESC_WB | TXDCTL_COUNT_DESC);
        hw.write32(TIPG, TIPG_COPPER);
        hw.write32(TCTL, TCTL_EN | TCTL_PSP | TCTL_CT | TCTL_COLD | TCTL_RTLC);
        hw.write32(RCTL, RCTL_EN | RCTL_MPE | RCTL_BAM | RCTL_SECRC);
        hw.write32(ITR, ITR_DEFAULT);
        hw.read32(ICR);
        hw.write32(IMS, INT_MASK);
    }

    pub fn hw(&self) -> &H {
        &self.hw
    }
}

impl<H: Hw> Nic for E1000e<H> {
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
        Ok(LinkState {
            up,
            speed_mbps: if !up {
                0
            } else {
                match (st >> 6) & 3 {
                    0 => 10,
                    1 => 100,
                    _ => 1000,
                }
            },
            full_duplex: up && st & STATUS_FD != 0,
        })
    }

    fn interrupt(&mut self) -> Result<Events, HwError> {
        // Cteni ICR priciny zaroven potvrdi.
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
        let hw = &self.hw;
        let mut frame = [0u8; BUF];
        let mut done = 0;
        while done < RING {
            let d = Layout::rx_desc(self.rx_next);
            let st = hw.dma_read32(d + 12);
            if st & RXD_DD == 0 {
                break;
            }
            hw.dma_rmb();
            let len = (hw.dma_read32(d + 8) & 0xFFFF) as usize;
            if st & RXD_EOP != 0 && st & RXD_ERRORS == 0 && (14..=MAX_FRAME).contains(&len) {
                hw.dma_read(Layout::rx_buf(self.rx_next), &mut frame[..len]);
                self.stats.rx_frames += 1;
                deliver(&frame[..len]);
            } else {
                self.stats.rx_errors += 1;
            }
            hw.dma_write32(d + 12, 0);
            self.rx_next = (self.rx_next + 1) % RING;
            done += 1;
        }
        if done > 0 {
            // Posledni volny deskriptor = ten pred dalsim ke cteni.
            hw.dma_wmb();
            hw.mmio_wmb();
            hw.write32(RDT, ((self.rx_next + RING - 1) % RING) as u32);
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
        let len = copy_tx(hw, Layout::tx_buf(i), frame);
        let d = Layout::tx_desc(i);
        hw.dma_write(d, &(hw.dma_addr() + Layout::tx_buf(i) as u64).to_le_bytes());
        hw.dma_write32(d + 8, len as u32 | TXD_CMD_EOP | TXD_CMD_IFCS | TXD_CMD_RS);
        hw.dma_write32(d + 12, 0);
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
        // Reset zastavi i DMA, ktere uz bylo rozbehnute.
        let ctrl = self.hw.read32(CTRL) & !CTRL_PHY_RST;
        self.hw.write32(CTRL, ctrl | CTRL_RST);
        self.hw.sleep_us(20_000);
    }

    fn stats(&self) -> Stats {
        self.stats
    }

    fn notes(&self) -> &[&'static str] {
        &self.notes
    }
}
