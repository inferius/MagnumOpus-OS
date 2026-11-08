//! Ovladace sitovych karet s registry v pameti (MMIO) a kruhy deskriptoru
//! v DMA pameti: Intel e1000e (82574, 82583, I217-I219), Intel igc
//! (I225, I226) a Realtek r8169 (RTL8111/8168).
//!
//! Logika cipu ([`e1000e`], [`igc`], [`r8169`]) pracuje jen nad rozhranim
//! [`Hw`]: registry, DMA pamet, bariery a cekani. V systemu ho plni
//! `driver` (feature `libos`), v testech na hostiteli simulace registru
//! a DMA (`tests/`). Smycka ovladace (`driver::run`) je pro vsechny cipy
//! stejna a mluvi se sluzbou `netd` protokolem `netproto::link` jako
//! virtio-net.
//!
//! Spolecne pro vsechny cipy:
//!
//! - jedna fronta prijmu a jedna odesilani po [`RING`] deskriptorech,
//!   buffer 2 KiB na ramec (MTU 1500, zadne jumbo ramce),
//! - jedno preruseni (MSI, kdyz ho funkce nema, MSI-X) pro vsechny priciny,
//! - zadne offloady (viz `netproto::link`), ramec bez FCS.

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod e1000e;
pub mod ids;
pub mod igc;
pub mod r8169;

#[cfg(feature = "libos")]
pub mod driver;

#[cfg(test)]
mod tests;

/// Pocet deskriptoru v kazdem kruhu.
pub const RING: usize = 256;
/// Velikost jednoho deskriptoru (u vsech tri rodin 16 B).
pub const DESC: usize = 16;
/// Buffer jednoho ramce.
pub const BUF: usize = 2048;
/// Nejkratsi ramec Ethernetu bez FCS (kratsi se doplni nulami).
pub const MIN_FRAME: usize = 60;
/// Nejdelsi ramec bez FCS (MTU 1500 + hlavicka 14 B).
pub const MAX_FRAME: usize = 1514;

/// Pristup k hardwaru. Offsety registru jsou od zacatku okna registru
/// (BAR), offsety DMA od zacatku DMA pameti ovladace ([`Layout`]).
pub trait Hw {
    fn read8(&self, reg: u32) -> u8;
    fn read16(&self, reg: u32) -> u16;
    fn read32(&self, reg: u32) -> u32;
    fn write8(&self, reg: u32, v: u8);
    fn write16(&self, reg: u32, v: u16);
    fn write32(&self, reg: u32, v: u32);

    /// Adresa DMA pameti pro zarizeni (fyzicka, s IOMMU adresa v jeho
    /// domene) na offsetu 0.
    fn dma_addr(&self) -> u64;
    fn dma_read(&self, off: usize, buf: &mut [u8]);
    fn dma_write(&self, off: usize, data: &[u8]);
    /// Cteni slova, ktere meni zarizeni (stav deskriptoru) - jednim
    /// pristupem, ne po bajtech.
    fn dma_read32(&self, off: usize) -> u32;
    fn dma_write32(&self, off: usize, v: u32);

    /// Zapisy do DMA pameti pred barierou zarizeni uvidi pred zapisy za ni.
    fn dma_wmb(&self);
    /// Cteni stavu deskriptoru pred ctenim jeho dat.
    fn dma_rmb(&self);
    /// Vsechny zapisy do pameti pred zapisem do registru ("doorbell").
    fn mmio_wmb(&self);

    fn sleep_us(&self, us: u64);
}

/// Opakuje `done`, dokud nevrati `true`, nejvys `timeout_us` (po krocich
/// `step_us`). `false` = vyprselo.
pub fn wait_for<H: Hw + ?Sized>(hw: &H, timeout_us: u64, step_us: u64, mut done: impl FnMut() -> bool) -> bool {
    let mut waited = 0;
    loop {
        if done() {
            return true;
        }
        if waited >= timeout_us {
            return false;
        }
        hw.sleep_us(step_us);
        waited += step_us;
    }
}

/// Zapise 64bitovou adresu do dvojice registru (horni driv: nektere cipy
/// berou adresu pri zapisu dolni poloviny).
pub fn write_addr<H: Hw + ?Sized>(hw: &H, low: u32, high: u32, addr: u64) {
    hw.write32(high, (addr >> 32) as u32);
    hw.write32(low, addr as u32);
}

/// Rozlozeni DMA pameti ovladace: kruh prijmu, kruh odesilani (kazdy na
/// vlastni strance, zarovnani 4 KiB staci vsem cipum), pak buffery
/// prijmu a odesilani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout;

impl Layout {
    pub const RX_RING: usize = 0;
    pub const TX_RING: usize = (RING * DESC).next_multiple_of(4096);
    const BUFFERS: usize = Self::TX_RING + (RING * DESC).next_multiple_of(4096);
    /// Celkova velikost DMA pameti.
    pub const SIZE: usize = Self::BUFFERS + 2 * RING * BUF;

    pub const fn rx_desc(i: usize) -> usize {
        Self::RX_RING + i * DESC
    }

    pub const fn tx_desc(i: usize) -> usize {
        Self::TX_RING + i * DESC
    }

    pub const fn rx_buf(i: usize) -> usize {
        Self::BUFFERS + i * BUF
    }

    pub const fn tx_buf(i: usize) -> usize {
        Self::BUFFERS + (RING + i) * BUF
    }
}

/// Stav linky.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LinkState {
    pub up: bool,
    /// Mb/s, 0 = neznama nebo odpojeno.
    pub speed_mbps: u32,
    pub full_duplex: bool,
}

/// Co se stalo od posledniho preruseni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Events {
    /// Zmena stavu linky (zjistit [`Nic::link`]).
    pub link: bool,
    /// Zarizeni hlasi vazny problem (chyba sbernice); ovladac konci.
    pub fatal: bool,
}

/// Chyba zarizeni: neodpovida, nebo vratilo neplatny stav. Ovladac pak
/// konci (netd uvidi zavreny kanal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwError {
    /// Registry ctou same jednicky: zarizeni zmizelo ze sbernice (vyjmuto,
    /// vypnuto, PCIe linka spadla).
    Gone,
    /// Reset ani nastaveni se v limitu nedokoncily.
    Timeout(&'static str),
    /// Zarizeni vratilo nesmyslny stav (index mimo kruh apod.).
    Corrupted(&'static str),
}

impl core::fmt::Display for HwError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HwError::Gone => f.write_str("zarizeni zmizelo ze sbernice"),
            HwError::Timeout(what) => write!(f, "vyprsel cas: {what}"),
            HwError::Corrupted(what) => write!(f, "neplatny stav zarizeni: {what}"),
        }
    }
}

/// Pocitadla ovladace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    pub rx_frames: u64,
    pub tx_frames: u64,
    /// Ramce s chybou (CRC, delka, rozdeleny do vic bufferu) - zahozene.
    pub rx_errors: u64,
}

/// Karta po inicializaci. Volani jdou z jedne smycky ovladace.
pub trait Nic {
    /// Jmeno cipu pro hlaseni (napr. "82574L").
    fn model(&self) -> &'static str;
    fn mac(&self) -> [u8; 6];
    fn link(&self) -> Result<LinkState, HwError>;
    /// Precte a potvrdi priciny preruseni.
    fn interrupt(&mut self) -> Result<Events, HwError>;
    /// Prijate ramce preda `deliver` (bez FCS) a buffery vrati karte.
    fn receive(&mut self, deliver: &mut dyn FnMut(&[u8])) -> Result<(), HwError>;
    /// Uvolni odeslane buffery.
    fn reclaim(&mut self) -> Result<(), HwError>;
    /// Kolik ramcu jde jeste zaradit k odeslani.
    fn tx_space(&self) -> usize;
    /// Zaradi ramec (14..=1514 B, volajici overil a `tx_space() > 0`).
    /// Odejde az po [`Nic::kick`].
    fn send(&mut self, frame: &[u8]);
    /// Oznami karte zarazene ramce (zapis indexu).
    fn kick(&mut self);
    /// Zastavi prijem, odesilani i preruseni (pred koncem ovladace, aby
    /// karta uz nesahala do jeho pameti).
    fn stop(&mut self);
    fn stats(&self) -> Stats;
    /// Upozorneni z inicializace (ovladac je vypise).
    fn notes(&self) -> &[&'static str] {
        &[]
    }
}

/// Platna adresa jednoho zarizeni (ne nulova, ne skupinova).
pub fn valid_mac(m: &[u8; 6]) -> bool {
    m != &[0; 6] && m[0] & 1 == 0
}

/// Mistne spravovana unicast MAC ze `seed`, kdyz ji karta nema.
pub fn local_mac(seed: u64) -> [u8; 6] {
    let s = seed.to_le_bytes();
    [0x02, s[0], s[1], s[2], s[3], s[4] ^ s[5]]
}

/// Indexy kruhu odesilani: `head` = nejstarsi neodeslany, `tail` = dalsi
/// volny; jeden deskriptor zustava volny (plny kruh se neda odlisit od
/// prazdneho podle indexu karty).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TxRing {
    pub head: usize,
    pub tail: usize,
}

impl TxRing {
    pub fn in_flight(&self) -> usize {
        (self.tail + RING - self.head) % RING
    }

    pub fn space(&self) -> usize {
        RING - 1 - self.in_flight()
    }
}

/// Zkopiruje ramec do bufferu odesilani a doplni ho na [`MIN_FRAME`];
/// vraci delku k odeslani.
pub(crate) fn copy_tx<H: Hw + ?Sized>(hw: &H, off: usize, frame: &[u8]) -> usize {
    hw.dma_write(off, frame);
    if frame.len() < MIN_FRAME {
        hw.dma_write(off + frame.len(), &[0u8; MIN_FRAME][..MIN_FRAME - frame.len()]);
        MIN_FRAME
    } else {
        frame.len()
    }
}
