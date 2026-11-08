//! Radic AHCI (specifikace Serial ATA AHCI 1.3.1): globalni registry,
//! porty, seznam prikazu a odeslani jednoho prikazu ATA.
//!
//! Kazdy port ma vlastni DMA pamet: seznam prikazu (pouziva se jen slot 0),
//! oblast pro prijate FIS, tabulku prikazu s jednou polozkou PRDT a datovy
//! buffer. Ovladac je synchronni: na kazdem portu nejvys jeden prikaz.
//!
//! Puvod: napsano podle verejne specifikace, bez prevzateho kodu jinych
//! projektu (zadny kod z Linuxu ani BSD).

extern crate alloc;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Error, Mapping, Result, Vmo, barrier};

// Globalni registry (ABAR).
pub const CAP: u64 = 0x00;
const GHC: u64 = 0x04;
const IS: u64 = 0x08;
pub const PI: u64 = 0x0C;
pub const VS: u64 = 0x10;
const CAP2: u64 = 0x24;
const BOHC: u64 = 0x28;

const GHC_HR: u32 = 1 << 0;
const GHC_IE: u32 = 1 << 1;
const GHC_AE: u32 = 1 << 31;
const CAP_SSS: u32 = 1 << 27;
/// Radic umi 64bitove adresy DMA (jinak jen pod 4 GiB).
const CAP_S64A: u32 = 1 << 31;
const CAP2_BOH: u32 = 1 << 0;
const BOHC_BOS: u32 = 1 << 0;
const BOHC_OOS: u32 = 1 << 1;
const BOHC_BB: u32 = 1 << 4;

// Registry portu (0x100 + 0x80 * port).
const P_CLB: u64 = 0x00;
const P_FB: u64 = 0x08;
const P_IS: u64 = 0x10;
const P_IE: u64 = 0x14;
const P_CMD: u64 = 0x18;
const P_TFD: u64 = 0x20;
const P_SIG: u64 = 0x24;
const P_SSTS: u64 = 0x28;
const P_SCTL: u64 = 0x2C;
const P_SERR: u64 = 0x30;
const P_CI: u64 = 0x38;

const CMD_ST: u32 = 1 << 0;
const CMD_SUD: u32 = 1 << 1;
const CMD_FRE: u32 = 1 << 4;
const CMD_FR: u32 = 1 << 14;
const CMD_CR: u32 = 1 << 15;
const TFD_ERR: u32 = 1 << 0;
const TFD_DRQ: u32 = 1 << 3;
const TFD_BSY: u32 = 1 << 7;
/// Chyby portu (PxIS): Task File Error, Host Bus Fatal/Data, Interface.
const IS_ERRORS: u32 = (1 << 30) | (1 << 29) | (1 << 28) | (1 << 27);
/// Preruseni portu: D2H Register FIS, PIO Setup, DMA Setup, Set Device
/// Bits, Descriptor Processed a chyby.
const IE_MASK: u32 = 0b10_1111 | IS_ERRORS;

/// Podpis disku SATA (ne ATAPI, ne port multiplier).
const SIG_ATA: u32 = 0x0000_0101;

// Rozlozeni DMA pameti portu.
const CL_OFF: u64 = 0x0000;
const FIS_OFF: u64 = 0x0400;
const CT_OFF: u64 = 0x0500;
pub const AUX_OFF: u64 = 0x0800;
pub const DATA_OFF: u64 = 0x1000;
pub const DATA_LEN: u64 = 0x8000;
const PORT_DMA: u64 = DATA_OFF + DATA_LEN;

const COMMAND_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(20);
const STOP_TIMEOUT: core::time::Duration = core::time::Duration::from_millis(500);
/// Jak dlouho cekat na pripojeni linky a na pripraveny disk (roztoceni).
const LINK_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(1);
/// Port, na kterem po teto dobe neni ani nahlaseno zarizeni (DET = 0), je
/// prazdny (navazani linky trva desitky ms).
const NO_DEVICE_AFTER: core::time::Duration = core::time::Duration::from_millis(200);
const READY_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(10);

pub struct Hba {
    regs: Mapping,
    off: u64,
    pub irq: Option<Interrupt>,
    /// Radic umi 64bitove adresy (CAP.S64A). Bez toho musi vsechna DMA
    /// pamet lezet pod 4 GiB, jinak by radic horni polovinu adresy zahodil
    /// a psal do cizi pameti.
    s64a: bool,
}

/// Lezi usek `addr..addr+len` cely pod 4 GiB?
fn below_4g(addr: u64, len: u64) -> bool {
    addr.checked_add(len).is_some_and(|end| end <= 1 << 32)
}

/// Jeden port s pripojenym diskem.
pub struct Port {
    pub index: u32,
    _dma: Vmo,
    pub mem: Mapping,
    pub phys: u64,
    /// Port prestal odpovidat: je zastaveny, pozadavky vraci `IO_ERROR`.
    pub failed: bool,
}

/// Prikaz ATA (registry H2D FIS).
#[derive(Default, Clone, Copy)]
pub struct Ata {
    pub command: u8,
    pub feature: u16,
    pub lba: u64,
    pub count: u16,
    /// Prikaz zapisuje na disk (data jdou z pameti).
    pub write: bool,
    /// Bajtu dat (0 = bez dat).
    pub bytes: u32,
    /// Fyzicka adresa dat (`None` = datovy nebo pomocny buffer portu).
    pub data_phys: Option<u64>,
}

/// Ceka na podminku (nejvys `timeout`, krok 1 ms).
fn poll(timeout: core::time::Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = libos::time::deadline_after(timeout);
    loop {
        if done() {
            return true;
        }
        if libos::time::now() > deadline {
            return false;
        }
        libos::time::sleep(core::time::Duration::from_millis(1));
    }
}

impl Hba {
    pub fn open(resource: &Resource, bar: u64, size: u64) -> Result<Self> {
        let page = libos::vm::page_size();
        let base = bar & !(page - 1);
        let off = bar - base;
        let len = (off + size).next_multiple_of(page);
        let regs = Mapping::new(&resource.device_memory(base, len)?, 0, len, MAP_READ | MAP_WRITE)?;
        let s64a = regs.read::<u32>(off + CAP) & CAP_S64A != 0;
        Ok(Self {
            regs,
            off,
            irq: None,
            s64a,
        })
    }

    pub fn reg(&self, r: u64) -> u32 {
        self.regs.read::<u32>(self.off + r)
    }

    fn set(&self, r: u64, v: u32) {
        self.regs.write::<u32>(self.off + r, v)
    }

    fn preg(&self, port: u32, r: u64) -> u32 {
        self.reg(0x100 + port as u64 * 0x80 + r)
    }

    fn pset(&self, port: u32, r: u64, v: u32) {
        self.set(0x100 + port as u64 * 0x80 + r, v)
    }

    /// Prevzeti od firmwaru (BIOS/OS handoff) a reset radice.
    pub fn reset(&self) -> Result<()> {
        if self.reg(CAP2) & CAP2_BOH != 0 {
            self.set(BOHC, self.reg(BOHC) | BOHC_OOS);
            poll(core::time::Duration::from_millis(25), || self.reg(BOHC) & BOHC_BOS == 0);
            if self.reg(BOHC) & BOHC_BB != 0 {
                poll(core::time::Duration::from_secs(2), || self.reg(BOHC) & BOHC_BOS == 0);
            }
        }
        self.set(GHC, self.reg(GHC) | GHC_AE);
        self.set(GHC, self.reg(GHC) | GHC_HR);
        if !poll(core::time::Duration::from_secs(1), || self.reg(GHC) & GHC_HR == 0) {
            return Err(Error::TimedOut);
        }
        self.set(GHC, self.reg(GHC) | GHC_AE);
        Ok(())
    }

    /// Zapne preruseni radice (po nastaveni MSI).
    pub fn enable_interrupts(&self) {
        self.set(IS, u32::MAX);
        self.set(GHC, self.reg(GHC) | GHC_IE);
    }

    /// Zastavi zpracovani prikazu a prijem FIS na portu (pak uz port
    /// nesaha do sve DMA pameti).
    pub fn stop(&self, p: u32) -> bool {
        let cmd = self.preg(p, P_CMD);
        self.pset(p, P_CMD, cmd & !CMD_ST);
        let a = poll(STOP_TIMEOUT, || self.preg(p, P_CMD) & CMD_CR == 0);
        self.pset(p, P_CMD, self.preg(p, P_CMD) & !CMD_FRE);
        let b = poll(STOP_TIMEOUT, || self.preg(p, P_CMD) & CMD_FR == 0);
        a && b
    }

    /// Pripravi port `p` (seznam prikazu, prijem FIS, roztoceni). Linku
    /// pak overi [`Self::wait_links`] spolecne pro vsechny porty.
    pub fn prepare_port(&self, resource: &Resource, p: u32) -> Result<Port> {
        if !self.stop(p) {
            return Err(Error::TimedOut);
        }
        let page = libos::vm::page_size();
        let dma = resource.dma_memory(PORT_DMA.next_multiple_of(page))?;
        let phys = resource.phys_addr(&dma, 0)?;
        if !self.s64a && !below_4g(phys, PORT_DMA) {
            libos::println!("ahci: port {p}: radic umi jen 32bitove adresy a DMA pamet je nad 4 GiB - vynechan");
            return Err(Error::NotSupported);
        }
        let mem = Mapping::new(&dma, 0, PORT_DMA, MAP_READ | MAP_WRITE)?;
        mem.write_bytes(0, &[0u8; DATA_OFF as usize]);
        let clb = phys + CL_OFF;
        let fb = phys + FIS_OFF;
        self.pset(p, P_CLB, clb as u32);
        self.pset(p, P_CLB + 4, (clb >> 32) as u32);
        self.pset(p, P_FB, fb as u32);
        self.pset(p, P_FB + 4, (fb >> 32) as u32);
        self.pset(p, P_SERR, u32::MAX);
        self.pset(p, P_IS, u32::MAX);
        let mut cmd = self.preg(p, P_CMD) | CMD_FRE;
        if self.reg(CAP) & CAP_SSS != 0 {
            cmd |= CMD_SUD;
        }
        self.pset(p, P_CMD, cmd);
        Ok(Port {
            index: p,
            _dma: dma,
            mem,
            phys,
            failed: false,
        })
    }

    /// COMRESET portu (port musi byt zastaveny, ST = 0): po resetu radice
    /// nekterych cipsetu linka sama nenaskoci. DET = 1 aspon 1 ms, pak 0.
    pub fn comreset(&self, p: u32) {
        let sctl = self.preg(p, P_SCTL) & !0xF;
        self.pset(p, P_SCTL, sctl | 1);
        libos::time::sleep(core::time::Duration::from_millis(2));
        self.pset(p, P_SCTL, sctl);
        self.pset(p, P_SERR, u32::MAX);
    }

    /// Pocka na navazani linky (DET = 3) na portech `ports`, nejvys
    /// `LINK_TIMEOUT` pro vsechny dohromady. Port, na kterem po
    /// `NO_DEVICE_AFTER` stale nic neni (DET = 0), se dal neceka.
    /// Vraci, na kterych portech linka je.
    pub fn wait_links(&self, ports: &[u32]) -> alloc::vec::Vec<bool> {
        let start = libos::time::now();
        let quiet = libos::time::deadline_after(NO_DEVICE_AFTER) - start;
        let mut up = alloc::vec![false; ports.len()];
        poll(LINK_TIMEOUT, || {
            let mut pending = false;
            for (i, &port) in ports.iter().enumerate() {
                let det = self.preg(port, P_SSTS) & 0xF;
                up[i] = det == 3;
                if !up[i] && (det != 0 || libos::time::now() - start < quiet) {
                    pending = true;
                }
            }
            !pending
        });
        up
    }

    /// Dokonci port s linkou: ceka na pripraveny disk, overi podpis SATA
    /// a spusti zpracovani prikazu. `Ok(None)` = na portu neni disk SATA.
    pub fn start_port(&self, port: Port) -> Result<Option<Port>> {
        let p = port.index;
        self.pset(p, P_SERR, u32::MAX);
        if !poll(READY_TIMEOUT, || self.preg(p, P_TFD) & (TFD_BSY | TFD_DRQ) == 0) {
            libos::println!("ahci: port {p}: disk neni pripraveny (TFD {:#x})", self.preg(p, P_TFD));
            self.stop(p);
            return Ok(None);
        }
        let sig = self.preg(p, P_SIG);
        if sig != SIG_ATA {
            libos::println!("ahci: port {p}: zarizeni s podpisem {sig:#x} (ne disk SATA) vynechano");
            self.stop(p);
            return Ok(None);
        }
        self.pset(p, P_IS, u32::MAX);
        self.pset(p, P_IE, IE_MASK);
        self.pset(p, P_CMD, self.preg(p, P_CMD) | CMD_ST);
        Ok(Some(port))
    }

    /// Port bez disku: zastavit prijem FIS (pamet portu se pak uvolni).
    pub fn release_port(&self, port: Port) {
        self.stop(port.index);
    }

    /// Provede prikaz ATA na portu (slot 0) a pocka na dokonceni.
    pub fn command(&self, port: &mut Port, c: Ata) -> Result<()> {
        if port.failed {
            return Err(Error::IoError);
        }
        let p = port.index;
        if let Some(addr) = c.data_phys
            && !self.s64a
            && !below_4g(addr, c.bytes as u64)
        {
            return Err(Error::NotSupported);
        }
        let m = &port.mem;
        // Tabulka prikazu: H2D Register FIS (20 B) + jedna polozka PRDT.
        let ct = CT_OFF;
        let mut fis = [0u8; 20];
        fis[0] = 0x27; // Register FIS, host -> zarizeni
        fis[1] = 0x80; // C: prikaz
        fis[2] = c.command;
        fis[3] = c.feature as u8;
        fis[4] = c.lba as u8;
        fis[5] = (c.lba >> 8) as u8;
        fis[6] = (c.lba >> 16) as u8;
        fis[7] = 1 << 6; // LBA
        fis[8] = (c.lba >> 24) as u8;
        fis[9] = (c.lba >> 32) as u8;
        fis[10] = (c.lba >> 40) as u8;
        fis[11] = (c.feature >> 8) as u8;
        fis[12] = c.count as u8;
        fis[13] = (c.count >> 8) as u8;
        m.write_bytes(ct, &[0u8; 0x80]);
        m.write_bytes(ct, &fis);
        let prdtl = if c.bytes > 0 {
            let data = c.data_phys.unwrap_or(port.phys + DATA_OFF);
            m.write::<u64>(ct + 0x80, data);
            m.write::<u32>(ct + 0x88, 0);
            m.write::<u32>(ct + 0x8C, c.bytes - 1);
            1u32
        } else {
            0
        };
        // Hlavicka slotu 0: delka FIS v dvojslovech, smer, pocet PRDT.
        let flags = 5 | (c.write as u32) << 6 | prdtl << 16;
        m.write::<u32>(CL_OFF, flags);
        m.write::<u32>(CL_OFF + 4, 0);
        m.write::<u64>(CL_OFF + 8, port.phys + ct);
        barrier::mmio_wmb();
        self.pset(p, P_IS, u32::MAX);
        self.pset(p, P_CI, 1);
        let r = self.wait(p);
        // Potvrdit preruseni portu a radice.
        self.pset(p, P_IS, u32::MAX);
        self.set(IS, 1 << p);
        match r {
            Ok(()) => {
                barrier::dma_rmb();
                if self.preg(p, P_TFD) & TFD_ERR != 0 {
                    self.recover(p);
                    return Err(Error::IoError);
                }
                Ok(())
            }
            Err(Error::IoError) => {
                self.recover(p);
                Err(Error::IoError)
            }
            Err(e) => {
                // Port neodpovida: zastavit (uz nesmi sahat do pameti).
                port.failed = true;
                self.stop(p);
                libos::println!("ahci: port {p}: neodpovida ({e}), dalsi pozadavky odmitnuty");
                Err(Error::IoError)
            }
        }
    }

    /// Ceka, az port dokonci slot 0 (`IO_ERROR` = chyba prikazu).
    fn wait(&self, p: u32) -> Result<()> {
        let deadline = libos::time::deadline_after(COMMAND_TIMEOUT);
        loop {
            let is = self.preg(p, P_IS);
            if is & IS_ERRORS != 0 {
                return Err(Error::IoError);
            }
            if self.preg(p, P_CI) & 1 == 0 {
                return Ok(());
            }
            if libos::time::now() > deadline {
                return Err(Error::TimedOut);
            }
            match &self.irq {
                // Kratky krok: preruseni muze patrit jinemu portu.
                Some(irq) => match irq.wait(Some(deadline.min(libos::time::deadline_after(
                    core::time::Duration::from_millis(100),
                )))) {
                    Ok(()) | Err(Error::TimedOut) => {}
                    Err(e) => return Err(e),
                },
                None => libos::time::sleep(core::time::Duration::from_micros(200)),
            }
        }
    }

    /// Obnova po chybe prikazu: restart portu (ST 0 -> 1), smazani chyb.
    fn recover(&self, p: u32) {
        self.stop(p);
        self.pset(p, P_SERR, u32::MAX);
        self.pset(p, P_IS, u32::MAX);
        self.pset(p, P_CMD, self.preg(p, P_CMD) | CMD_FRE);
        poll(READY_TIMEOUT, || self.preg(p, P_TFD) & (TFD_BSY | TFD_DRQ) == 0);
        self.pset(p, P_CMD, self.preg(p, P_CMD) | CMD_ST);
    }
}
