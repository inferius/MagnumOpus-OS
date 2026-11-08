//! Radic NVMe (specifikace NVM Express Base 1.4): registry, admin fronta,
//! jedna I/O fronta a prikazy.
//!
//! Ovladac je synchronni (jeden prikaz najednou), proto staci jedna dvojice
//! front I/O o 16 polozkach a jedno preruseni MSI-X (vektor 0 pro admin
//! i I/O frontu dokoncenych).
//!
//! Puvod: napsano podle verejne specifikace, bez prevzateho kodu jinych
//! projektu (zadny kod z Linuxu ani BSD).

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::{Interrupt, Resource};
use libos::{Error, Mapping, Result, Vmo, barrier, println};
use pci::msix::MsixTable;
use pci::{Address, Ecam};

// Registry radice (BAR0).
const CAP: u64 = 0x00;
const VS: u64 = 0x08;
const CC: u64 = 0x14;
const CSTS: u64 = 0x1C;
const AQA: u64 = 0x24;
const ASQ: u64 = 0x28;
const ACQ: u64 = 0x30;
const DOORBELLS: u64 = 0x1000;

const CC_EN: u32 = 1 << 0;
/// Velikost polozek: SQ 2^6 = 64 B, CQ 2^4 = 16 B.
const CC_QUEUE_SIZES: u32 = (6 << 16) | (4 << 20);
const CC_SHN_MASK: u32 = 3 << 14;
const CC_SHN_NORMAL: u32 = 1 << 14;
const CSTS_RDY: u32 = 1 << 0;
const CSTS_CFS: u32 = 1 << 1;
const CSTS_SHST_MASK: u32 = 3 << 2;
const CSTS_SHST_DONE: u32 = 2 << 2;

// Prikazy admin.
const ADM_CREATE_SQ: u8 = 0x01;
const ADM_CREATE_CQ: u8 = 0x05;
const ADM_IDENTIFY: u8 = 0x06;
const ADM_SET_FEATURES: u8 = 0x09;
const FEAT_NUM_QUEUES: u32 = 0x07;
// Prikazy NVM.
pub const IO_FLUSH: u8 = 0x00;
pub const IO_WRITE: u8 = 0x01;
pub const IO_READ: u8 = 0x02;
pub const IO_DSM: u8 = 0x09;

/// Stranka pameti radice (CC.MPS = 0).
pub const PAGE: u64 = 4096;
const QUEUE_ENTRIES: u16 = 16;

// Rozlozeni DMA pameti (vse na hranici stranky NVMe).
const ASQ_OFF: u64 = 0x0000;
const ACQ_OFF: u64 = 0x1000;
const IOSQ_OFF: u64 = 0x2000;
const IOCQ_OFF: u64 = 0x3000;
/// Vysledek IDENTIFY, seznam useku DSM.
pub const AUX_OFF: u64 = 0x4000;
const PRP_LIST_OFF: u64 = 0x5000;
pub const DATA_OFF: u64 = 0x6000;
pub const DATA_LEN: u64 = blockproto::MAX_SECTORS as u64 * blockproto::SECTOR as u64;
const DMA_SIZE: u64 = DATA_OFF + DATA_LEN;

/// Jak dlouho cekat na dokonceni prikazu.
const COMMAND_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(10);
/// Jak dlouho cekat na ulozeni dat pri vypnuti radice.
const SHUTDOWN_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(5);

/// Fronta prikazu a k ni fronta dokoncenych.
struct Queue {
    qid: u16,
    sq_off: u64,
    cq_off: u64,
    tail: u16,
    head: u16,
    phase: bool,
}

impl Queue {
    const fn new(qid: u16, sq_off: u64, cq_off: u64) -> Self {
        Self {
            qid,
            sq_off,
            cq_off,
            tail: 0,
            head: 0,
            phase: true,
        }
    }
}

/// Prikaz (64 B; nepouzita dvojslova jsou nulova).
#[derive(Default, Clone, Copy)]
pub struct Command {
    pub opcode: u8,
    pub nsid: u32,
    pub prp1: u64,
    pub prp2: u64,
    pub cdw10: u32,
    pub cdw11: u32,
    pub cdw12: u32,
}

/// Udaje z IDENTIFY CONTROLLER.
pub struct Identity {
    /// Volatile Write Cache: zarizeni ma mezipamet zapisu (FLUSH ma smysl).
    pub write_cache: bool,
    /// Umi Dataset Management (TRIM).
    pub dsm: bool,
    /// Nejvic sektoru po 512 B v jednom prenosu.
    pub max_sectors: u32,
}

/// Jmenny prostor (disk) z IDENTIFY NAMESPACE.
pub struct Namespace {
    pub nsid: u32,
    pub sectors: u64,
    pub write_protected: bool,
}

pub struct Controller {
    regs: Mapping,
    /// Offset registru v mapovani (BAR nemusi zacinat na strance).
    off: u64,
    /// Rozestup doorbell registru (4 << CAP.DSTRD).
    stride: u64,
    entries: u16,
    timeout_ready: core::time::Duration,
    irq: Interrupt,
    _msix: MsixTable,
    _dma: Vmo,
    pub mem: Mapping,
    pub phys: u64,
    admin: Queue,
    io: Queue,
    next_cid: u16,
    /// Radic neodpovida: dalsi prikazy se neposilaji (radic je vypnuty,
    /// aby uz nesahal do DMA pameti).
    pub failed: bool,
}

fn read64(m: &Mapping, o: u64) -> u64 {
    m.read::<u32>(o) as u64 | (m.read::<u32>(o + 4) as u64) << 32
}

fn write64(m: &Mapping, o: u64, v: u64) {
    m.write::<u32>(o, v as u32);
    m.write::<u32>(o + 4, (v >> 32) as u32);
}

impl Controller {
    /// Reset radice, admin fronta, MSI-X a I/O fronta 1.
    pub fn open(ecam: &Ecam, a: Address, resource: &Resource, bar: u64, bar_size: u64) -> Result<Self> {
        let page = libos::vm::page_size();
        let base = bar & !(page - 1);
        let off = bar - base;
        let len = (off + bar_size).next_multiple_of(page);
        let regs = Mapping::new(&resource.device_memory(base, len)?, 0, len, MAP_READ | MAP_WRITE)?;
        ecam.enable(a);
        let cap = read64(&regs, off + CAP);
        let mqes = (cap & 0xFFFF) as u16 + 1;
        let stride = 4u64 << ((cap >> 32) & 0xF);
        let timeout_ready = core::time::Duration::from_millis(((cap >> 24) & 0xFF).max(2) * 500);
        let mps_min = (cap >> 48) & 0xF;
        // NVM command set (CAP.CSS bit 37), stranka 4 KiB, doorbelly v BARu.
        if cap & (1 << 37) == 0 || mps_min != 0 || mqes < 2 || off + DOORBELLS + 4 * stride > len {
            println!("nvme {a}: nepodporovany radic (CAP {cap:#x})");
            return Err(Error::NotSupported);
        }
        let entries = QUEUE_ENTRIES.min(mqes);
        let vs = regs.read::<u32>(off + VS);
        println!("nvme {a}: NVMe {}.{}, fronty po {entries} polozkach", vs >> 16, (vs >> 8) & 0xFF);

        let dma = resource.dma_memory(DMA_SIZE.next_multiple_of(page))?;
        let phys = resource.phys_addr(&dma, 0)?;
        let mem = Mapping::new(&dma, 0, DMA_SIZE, MAP_READ | MAP_WRITE)?;
        // Seznam PRP pro datovy buffer: stranky 2..N (prvni jde v PRP1).
        for i in 1..DATA_LEN / PAGE {
            mem.write::<u64>(PRP_LIST_OFF + (i - 1) * 8, phys + DATA_OFF + i * PAGE);
        }
        let irq = resource.interrupt()?;
        let msix = MsixTable::open(ecam, a, resource)?;
        msix.set(0, irq.msi())?;
        msix.enable(ecam);

        let mut c = Self {
            regs,
            off,
            stride,
            entries,
            timeout_ready,
            irq,
            _msix: msix,
            _dma: dma,
            mem,
            phys,
            admin: Queue::new(0, ASQ_OFF, ACQ_OFF),
            io: Queue::new(1, IOSQ_OFF, IOCQ_OFF),
            next_cid: 0,
            failed: false,
        };
        c.reset_and_enable()?;
        // Jedna fronta I/O (pocty jsou od nuly).
        c.admin_command(Command {
            opcode: ADM_SET_FEATURES,
            cdw10: FEAT_NUM_QUEUES,
            ..Default::default()
        })?;
        let size = (entries as u32 - 1) << 16;
        c.admin_command(Command {
            opcode: ADM_CREATE_CQ,
            prp1: c.phys + IOCQ_OFF,
            cdw10: size | 1,
            // Preruseni zapnuto (IEN), vektor 0, souvisla pamet (PC).
            cdw11: (1 << 1) | 1,
            ..Default::default()
        })?;
        c.admin_command(Command {
            opcode: ADM_CREATE_SQ,
            prp1: c.phys + IOSQ_OFF,
            cdw10: size | 1,
            // Fronta dokoncenych 1, souvisla pamet.
            cdw11: (1 << 16) | 1,
            ..Default::default()
        })?;
        Ok(c)
    }

    fn reg(&self, r: u64) -> u32 {
        self.regs.read::<u32>(self.off + r)
    }

    fn set_reg(&self, r: u64, v: u32) {
        self.regs.write::<u32>(self.off + r, v)
    }

    /// Ceka, az `CSTS & mask == want` (nejvys `timeout`).
    fn wait_status(&self, mask: u32, want: u32, timeout: core::time::Duration) -> Result<()> {
        let deadline = libos::time::deadline_after(timeout);
        loop {
            let s = self.reg(CSTS);
            if s & mask == want {
                return Ok(());
            }
            // Vsechny jednicky = zarizeni zmizelo ze sbernice.
            if s == u32::MAX || libos::time::now() > deadline {
                return Err(Error::TimedOut);
            }
            libos::time::sleep(core::time::Duration::from_millis(1));
        }
    }

    fn reset_and_enable(&mut self) -> Result<()> {
        if self.reg(CC) & CC_EN != 0 {
            self.set_reg(CC, 0);
        }
        self.wait_status(CSTS_RDY, 0, self.timeout_ready)?;
        let n = self.entries as u32 - 1;
        self.set_reg(AQA, (n << 16) | n);
        write64(&self.regs, self.off + ASQ, self.phys + ASQ_OFF);
        write64(&self.regs, self.off + ACQ, self.phys + ACQ_OFF);
        barrier::mmio_wmb();
        self.set_reg(CC, CC_QUEUE_SIZES | CC_EN);
        self.wait_status(CSTS_RDY, CSTS_RDY, self.timeout_ready)?;
        if self.reg(CSTS) & CSTS_CFS != 0 {
            return Err(Error::IoError);
        }
        Ok(())
    }

    /// Normalni vypnuti: radic ulozi mezipamet a prestane pouzivat DMA.
    pub fn shutdown(&mut self) {
        if self.failed {
            return;
        }
        let cc = self.reg(CC);
        self.set_reg(CC, (cc & !CC_SHN_MASK) | CC_SHN_NORMAL);
        if self.wait_status(CSTS_SHST_MASK, CSTS_SHST_DONE, SHUTDOWN_TIMEOUT).is_err() {
            println!("nvme: radic nedokoncil vypnuti vcas");
        }
        self.set_reg(CC, 0);
    }

    /// Radic se chova chybne: vypnout ho (prestane cist a psat DMA pamet).
    fn fail(&mut self, why: Error) -> Error {
        self.failed = true;
        self.set_reg(CC, 0);
        println!("nvme: radic neodpovida spravne ({why}), dalsi pozadavky odmitnuty");
        Error::IoError
    }

    pub fn admin_command(&mut self, cmd: Command) -> Result<u32> {
        self.submit(false, cmd)
    }

    pub fn io_command(&mut self, cmd: Command) -> Result<u32> {
        self.submit(true, cmd)
    }

    /// Posle prikaz a pocka na jeho dokonceni; vraci dvojslovo 0 vysledku.
    fn submit(&mut self, io: bool, cmd: Command) -> Result<u32> {
        if self.failed {
            return Err(Error::IoError);
        }
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1);
        let entries = self.entries;
        let (sq_off, tail, qid) = {
            let q = if io { &self.io } else { &self.admin };
            (q.sq_off, q.tail, q.qid)
        };
        let e = sq_off + tail as u64 * 64;
        let mut dw = [0u32; 16];
        dw[0] = cmd.opcode as u32 | (cid as u32) << 16;
        dw[1] = cmd.nsid;
        dw[6] = cmd.prp1 as u32;
        dw[7] = (cmd.prp1 >> 32) as u32;
        dw[8] = cmd.prp2 as u32;
        dw[9] = (cmd.prp2 >> 32) as u32;
        dw[10] = cmd.cdw10;
        dw[11] = cmd.cdw11;
        dw[12] = cmd.cdw12;
        for (i, v) in dw.iter().enumerate() {
            self.mem.write::<u32>(e + i as u64 * 4, *v);
        }
        let new_tail = (tail + 1) % entries;
        if io {
            self.io.tail = new_tail;
        } else {
            self.admin.tail = new_tail;
        }
        barrier::mmio_wmb();
        self.set_reg(DOORBELLS + (2 * qid as u64) * self.stride, new_tail as u32);
        match self.complete(io, cid) {
            Ok(r) => r,
            Err(e) => Err(self.fail(e)),
        }
    }

    /// Ceka na polozku fronty dokoncenych. Vnejsi `Err` = radic se chova
    /// chybne (vyprsel cas, cizi cislo prikazu); vnitrni = stav prikazu.
    fn complete(&mut self, io: bool, cid: u16) -> Result<Result<u32>> {
        let deadline = libos::time::deadline_after(COMMAND_TIMEOUT);
        let entries = self.entries;
        loop {
            let q = if io { &mut self.io } else { &mut self.admin };
            let e = q.cq_off + q.head as u64 * 16;
            let status = self.mem.read::<u16>(e + 14);
            if (status & 1 == 1) == q.phase {
                barrier::dma_rmb();
                let dw0 = self.mem.read::<u32>(e);
                let done_cid = self.mem.read::<u16>(e + 12);
                q.head += 1;
                if q.head == entries {
                    q.head = 0;
                    q.phase = !q.phase;
                }
                let (head, qid) = (q.head, q.qid);
                self.set_reg(DOORBELLS + (2 * qid as u64 + 1) * self.stride, head as u32);
                if done_cid != cid {
                    return Err(Error::Corrupted);
                }
                return Ok(status_result(status >> 1).map(|_| dw0));
            }
            match self.irq.wait(Some(deadline)) {
                Ok(()) => {}
                // Posledni kontrola po vyprseni (preruseni se mohlo ztratit).
                Err(Error::TimedOut) if libos::time::now() > deadline => {
                    let q = if io { &self.io } else { &self.admin };
                    let status = self.mem.read::<u16>(q.cq_off + q.head as u64 * 16 + 14);
                    if (status & 1 == 1) != q.phase {
                        return Err(Error::TimedOut);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// IDENTIFY (vysledek 4 KiB v `AUX_OFF`).
    fn identify(&mut self, cns: u32, nsid: u32) -> Result<()> {
        self.admin_command(Command {
            opcode: ADM_IDENTIFY,
            nsid,
            prp1: self.phys + AUX_OFF,
            cdw10: cns,
            ..Default::default()
        })
        .map(|_| ())
    }

    pub fn identify_controller(&mut self) -> Result<Identity> {
        self.identify(1, 0)?;
        let m = &self.mem;
        let mdts = m.read::<u8>(AUX_OFF + 77);
        let oncs = m.read::<u16>(AUX_OFF + 520);
        let vwc = m.read::<u8>(AUX_OFF + 525);
        let max_bytes = if mdts == 0 || mdts > 20 {
            DATA_LEN
        } else {
            (PAGE << mdts).min(DATA_LEN)
        };
        let mut model = [0u8; 40];
        m.read_bytes(AUX_OFF + 24, &mut model);
        let model = core::str::from_utf8(&model).unwrap_or("?").trim();
        println!("nvme: radic '{model}'");
        Ok(Identity {
            write_cache: vwc & 1 != 0,
            dsm: oncs & (1 << 2) != 0,
            max_sectors: (max_bytes / blockproto::SECTOR as u64) as u32,
        })
    }

    /// Aktivni jmenne prostory s logickym sektorem 512 B bez metadat.
    /// Ostatni se zaloguji a vynechaji (TODO: sektor 4 KiB).
    pub fn namespaces(&mut self, limit: usize) -> Result<alloc::vec::Vec<Namespace>> {
        self.identify(2, 0)?;
        let mut ids = alloc::vec::Vec::new();
        for i in 0..1024u64 {
            let id = self.mem.read::<u32>(AUX_OFF + i * 4);
            if id == 0 {
                break;
            }
            ids.push(id);
        }
        let mut out = alloc::vec::Vec::new();
        for nsid in ids.into_iter().take(limit) {
            self.identify(0, nsid)?;
            let m = &self.mem;
            let size = m.read::<u64>(AUX_OFF);
            let flbas = m.read::<u8>(AUX_OFF + 26);
            let format = m.read::<u32>(AUX_OFF + 128 + (flbas & 0xF) as u64 * 4);
            let meta = format & 0xFFFF;
            let lbads = (format >> 16) & 0xFF;
            let wp = m.read::<u8>(AUX_OFF + 99) & 1 != 0;
            if lbads != 9 || meta != 0 {
                println!(
                    "nvme: jmenny prostor {nsid} ma sektor {} B a metadata {meta} B - zatim nepodporovano, vynechan",
                    1u64 << lbads.min(63)
                );
                continue;
            }
            out.push(Namespace {
                nsid,
                sectors: size,
                write_protected: wp,
            });
        }
        Ok(out)
    }

    /// PRP pro prenos `len` bajtu z datoveho bufferu.
    pub fn data_prps(&self, len: u64) -> (u64, u64) {
        let prp1 = self.phys + DATA_OFF;
        let pages = len.div_ceil(PAGE);
        let prp2 = match pages {
            0 | 1 => 0,
            2 => prp1 + PAGE,
            _ => self.phys + PRP_LIST_OFF,
        };
        (prp1, prp2)
    }
}

/// Stavove pole dokonceni (bez bitu faze) -> vysledek.
fn status_result(sf: u16) -> Result<()> {
    let sc = sf & 0xFF;
    let sct = (sf >> 8) & 7;
    match (sct, sc) {
        (0, 0) => Ok(()),
        // Neplatny prikaz nebo pole.
        (0, 0x01) | (0, 0x02) => Err(Error::NotSupported),
        // LBA mimo rozsah, kapacita prekrocena.
        (0, 0x80) | (0, 0x81) => Err(Error::OutOfRange),
        // Jmenny prostor chraneny proti zapisu.
        (0, 0x20) => Err(Error::ReadOnly),
        _ => Err(Error::IoError),
    }
}
