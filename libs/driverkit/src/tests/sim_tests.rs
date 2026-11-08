//! Simulace: maly model "kopirky" (DMA ze zdroje do cile, preruseni po
//! dokonceni) a ovladac nad ni pres celou smycku driverkitu.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::dma::DmaBuffer;
use crate::ring::{DescRing, PhaseRing};
use crate::sim::{Bus, DeviceModel, RegFile, Sim};
use crate::{DeviceInfo, Driver, DriverError, Exit, IrqId, Platform, Result, Runner, Sent, run};

const ID: u64 = 0x00;
const SRC: u64 = 0x08;
const DST: u64 = 0x10;
const LEN: u64 = 0x18;
const CMD: u64 = 0x1C;
const STATUS: u64 = 0x20;
const MAGIC: u64 = 0xC0FE;

crate::registers! {
    R_ID @ 0x00: u32;
    R_SRC @ 0x08: u64;
    R_DST @ 0x10: u64;
    R_LEN @ 0x18: u32;
    R_CMD @ 0x1C: u32 { START: 0 }
    R_STATUS @ 0x20: u32 { DONE: 0 }
}

/// Model: po zapisu START za 1 ms zkopiruje LEN bajtu ze SRC do DST.
struct Copier {
    regs: RegFile,
    busy_until: Option<u64>,
    copies: u32,
}

impl Copier {
    fn new() -> Self {
        Self {
            regs: RegFile::new()
                .ro(ID, MAGIC)
                .rw(SRC, 0)
                .rw(DST, 0)
                .rw(LEN, 0)
                .rw(CMD, 0)
                .reg(STATUS, 0, 0, 1),
            busy_until: None,
            copies: 0,
        }
    }
}

impl DeviceModel for Copier {
    fn bar_size(&self, index: u8) -> Option<u64> {
        (index == 0).then_some(0x1000)
    }
    fn read(&mut self, _: &mut Bus, _: u8, off: u64, _: u8) -> u64 {
        self.regs.read(off)
    }
    fn write(&mut self, bus: &mut Bus, _: u8, off: u64, _: u8, v: u64) {
        self.regs.write(off, v);
        if off == CMD && v & 1 != 0 {
            self.busy_until = Some(bus.now() + 1_000_000);
        }
    }
    fn tick(&mut self, bus: &mut Bus) {
        if self.busy_until.is_some_and(|t| bus.now() >= t) {
            self.busy_until = None;
            let len = self.regs.get(LEN) as usize;
            let mut data = alloc::vec![0u8; len];
            if bus.dma_read(self.regs.get(SRC), &mut data).is_ok()
                && bus.dma_write(self.regs.get(DST), &data).is_ok()
            {
                self.copies += 1;
            }
            self.regs.set_bits(STATUS, 1);
            bus.raise_irq(0);
        }
    }
    fn as_any(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

struct CopyDriver<P: Platform> {
    regs: P::Mmio,
    src: DmaBuffer<P::Dma>,
    dst: DmaBuffer<P::Dma>,
    irq: IrqId,
    len: u32,
    /// Chyba v ovladaci pro test: posle zarizeni adresu za koncem bufferu.
    bad_address: bool,
    /// Po zavreni kanalu klientem bezet dal (obnova sluzeb).
    keep: bool,
    sent_without_client: u32,
}

impl<P: Platform> Driver<P> for CopyDriver<P> {
    fn probe(p: &P) -> Result<Self> {
        let regs = p.map_bar(0)?;
        if R_ID.read(&regs) as u64 != MAGIC {
            return Err(DriverError::Device(String::from("spatne ID")));
        }
        let irq = p.enable_irq()?;
        Ok(Self {
            regs,
            src: p.alloc_dma("zdroj", 256)?,
            dst: p.alloc_dma("cil", 256)?,
            irq,
            len: 0,
            bad_address: false,
            keep: false,
            sent_without_client: 0,
        })
    }

    fn start(&mut self, p: &P) -> Result<()> {
        p.send(b"hello")?;
        Ok(())
    }

    fn message(&mut self, p: &P, msg: &[u8]) -> Result<()> {
        if msg == b"keep" {
            self.keep = true;
            return Ok(());
        }
        if msg == b"bug" {
            self.bad_address = true;
            return Ok(());
        }
        self.src.write_bytes(0, msg)?;
        self.len = msg.len() as u32;
        let dst = if self.bad_address {
            // Spatne: adresa za koncem bufferu (obejiti iova_of).
            self.dst.iova() + 4096
        } else {
            self.dst.iova_of(0, msg.len() as u64)?
        };
        R_SRC.write(&self.regs, self.src.iova_of(0, msg.len() as u64)?);
        R_DST.write(&self.regs, dst);
        R_LEN.write(&self.regs, self.len);
        p.mmio_wmb();
        R_CMD::START.set(&self.regs);
        Ok(())
    }

    fn interrupt(&mut self, p: &P, irq: IrqId) -> Result<()> {
        assert_eq!(irq, self.irq);
        if !R_STATUS::DONE.is_set(&self.regs) {
            return Ok(());
        }
        R_STATUS.write(&self.regs, R_STATUS::DONE.value(1)?);
        p.dma_rmb();
        let mut out = alloc::vec![0u8; self.len as usize];
        self.dst.read_bytes(0, &mut out)?;
        if p.send(&out)? == Sent::NoClient {
            self.sent_without_client += 1;
        }
        crate::log!(p, "zkopirovano {} B", self.len);
        Ok(())
    }

    fn stop(&mut self, _p: &P) {
        R_CMD.write(&self.regs, 0);
    }

    fn client_closed(&mut self, _p: &P) -> bool {
        self.keep
    }
}

fn info() -> DeviceInfo {
    DeviceInfo { bus: 0, device: 3, function: 0, vendor: 0x1234, device_id: 0x5678 }
}

#[test]
fn driver_runs_over_simulated_device() {
    let sim = Sim::new("kopirka", info(), Copier::new()).bars(&[0]);
    sim.send(b"dk:ping");
    sim.send(b"ahoj svete");
    sim.close();
    let exit = run::<_, CopyDriver<_>>(&sim.platform());
    // Klient zavrel kanal driv, nez kopie dobehla: ovladac skonci hned.
    assert_eq!(exit, Exit::ClientClosed);
    assert_eq!(sim.sent_text(), ["hello", "dk:pong kopirka"]);
    // Kopie nedobehla, ale stop zarizeni zastavil (CMD = 0).
    let last = sim.trace().last().copied().unwrap();
    assert!(last.write && last.off == CMD && last.value == 0);
}

#[test]
fn interrupt_completes_request() {
    let sim = Sim::new("kopirka", info(), Copier::new()).bars(&[0]);
    let p = sim.platform();
    let mut r = Runner::<CopyDriver<_>>::new(&p).ok().unwrap();
    sim.send(b"data");
    assert_eq!(r.step(&p), None); // zprava -> start kopie
    assert_eq!(r.step(&p), None); // preruseni po 1 ms virtualniho casu
    assert!(sim.now() >= 1_000_000);
    assert_eq!(sim.sent_text(), ["hello", "data"]);
    assert_eq!(sim.with_model(|m: &mut Copier, _| m.copies), 1);
    assert!(sim.logs().iter().any(|l| l == "kopirka 00:03.0: zkopirovano 4 B"));
    assert!(sim.faults().is_empty());
    sim.close();
    let exit = r.step(&p).unwrap();
    r.finish(&p, &exit);
    assert_eq!(exit.code(), 0);
}

#[test]
fn dma_outside_buffers_is_reported_like_iommu() {
    let sim = Sim::new("kopirka", info(), Copier::new()).bars(&[0]);
    let p = sim.platform();
    let mut r = Runner::<CopyDriver<_>>::new(&p).ok().unwrap();
    sim.send(b"bug");
    sim.send(b"xyz");
    assert_eq!(r.step(&p), None);
    assert_eq!(r.step(&p), None);
    assert_eq!(r.step(&p), None);
    let faults = sim.faults();
    assert_eq!(faults.len(), 1);
    let dst = sim.dma_regions()[1].1;
    assert_eq!(faults[0], DriverError::DmaFault { iova: dst + 4096, len: 3, write: true });
}

#[test]
fn missing_bar_and_quotas_are_readable() {
    let sim = Sim::new("kopirka", info(), Copier::new()).bars(&[2]);
    let e = run::<_, CopyDriver<_>>(&sim.platform());
    let Exit::Failed { probe: true, err } = &e else { panic!("{e:?}") };
    assert!(format!("{err}").contains("resources.bars"), "{err}");
    assert_eq!(e.code(), 3);

    let sim = Sim::new("kopirka", info(), Copier::new()).irq_max(0);
    let Exit::Failed { err, .. } = run::<_, CopyDriver<_>>(&sim.platform()) else { panic!() };
    assert!(matches!(err, DriverError::IrqQuota { .. }));

    let sim = Sim::new("kopirka", info(), Copier::new()).dma_max(300);
    let Exit::Failed { err, .. } = run::<_, CopyDriver<_>>(&sim.platform()) else { panic!() };
    assert_eq!(err, DriverError::DmaQuota { buffer: "cil", size: 256 });
}

#[test]
fn reserved_messages() {
    let sim = Sim::new("kopirka", info(), Copier::new());
    sim.send(b"dk:info");
    sim.send(b"dk:restart");
    let e = run::<_, CopyDriver<_>>(&sim.platform());
    assert_eq!(e, Exit::Restart);
    assert_eq!(e.code(), crate::proto::EXIT_RESTART);
    assert_eq!(sim.sent_text()[1], "dk:info kopirka 00:03.0 1234:5678");

    let sim = Sim::new("kopirka", info(), Copier::new());
    sim.send(b"dk:crash");
    assert_eq!(run::<_, CopyDriver<_>>(&sim.platform()).code(), crate::proto::EXIT_CRASH);
}

#[test]
fn waiting_forever_ends_with_timeout() {
    let sim = Sim::new("kopirka", info(), Copier::new()).time_limit(5_000_000);
    let e = run::<_, CopyDriver<_>>(&sim.platform());
    let Exit::Failed { err, .. } = e else { panic!() };
    assert!(matches!(err, DriverError::Timeout { .. }));
    assert!(sim.logs().iter().any(|l| l.contains("nic se nedeje")));
}

#[test]
fn client_full() {
    let sim = Sim::new("kopirka", info(), Copier::new());
    sim.set_client_full(true);
    assert_eq!(sim.platform().send(b"x").unwrap(), Sent::Full);
}

#[test]
fn desc_ring_head_tail() {
    let sim = Sim::new("kopirka", info(), Copier::new());
    let p = sim.platform();
    let mut ring = DescRing::new(p.alloc_dma("tx", 64).unwrap(), "tx", 16, 4).unwrap();
    assert_eq!(ring.free(), 3);
    for i in 0..3u8 {
        assert_eq!(ring.push(&[i; 8]).unwrap(), i as u32);
    }
    assert_eq!(ring.push(&[9]), Err(DriverError::RingFull { ring: "tx" }));
    assert_eq!(ring.tail(), 3);
    let done: Vec<u32> = ring.complete_to(2).unwrap().collect();
    assert_eq!(done, [0, 1]);
    assert_eq!(ring.pending(), 1);
    // Obeh kruhu.
    ring.push(&[3]).unwrap();
    ring.push(&[4]).unwrap();
    assert_eq!(ring.tail(), 1);
    let done: Vec<u32> = ring.complete_to(1).unwrap().collect();
    assert_eq!(done, [2, 3, 0]);
    let mut e = [0u8; 16];
    ring.read_entry(0, &mut e).unwrap();
    assert_eq!(e[0], 4);
    assert_eq!(e[1], 0); // zbytek polozky vynulovany
    // Vadne zarizeni: hlava mimo kruh nebo "dokonci" nevlozene.
    assert!(matches!(ring.complete_to(7), Err(DriverError::Device(_))));
    assert!(matches!(ring.complete_to(3), Err(DriverError::Device(_))));
    assert!(DescRing::new(p.alloc_dma("maly", 16).unwrap(), "maly", 16, 4).is_err());
}

#[test]
fn phase_ring() {
    let sim = Sim::new("kopirka", info(), Copier::new());
    let p = sim.platform();
    let buf = p.alloc_dma("cq", 32).unwrap();
    let iova = buf.iova();
    let mut cq = PhaseRing::new(buf, 16, 2, 14, 0).unwrap();
    let mut out = [0u8; 16];
    assert_eq!(cq.pop(&mut out).unwrap(), None);
    // Zarizeni zapise dve polozky s fazi 1, pak prvni s fazi 0.
    sim.with_model(|_: &mut Copier, bus| {
        bus.dma_write(iova, &[0xAA]).unwrap();
        bus.dma_write(iova + 14, &[1]).unwrap();
        bus.dma_write(iova + 16 + 14, &[1]).unwrap();
    });
    assert_eq!(cq.pop(&mut out).unwrap(), Some(0));
    assert_eq!(out[0], 0xAA);
    assert_eq!(cq.pop(&mut out).unwrap(), Some(1));
    assert_eq!(cq.head(), 0);
    assert_eq!(cq.pop(&mut out).unwrap(), None); // faze 1, ceka se 0
    sim.with_model(|_: &mut Copier, bus| bus.dma_write(iova + 14, &[0]).unwrap());
    assert_eq!(cq.pop(&mut out).unwrap(), Some(0));
}

#[test]
fn dma_buffer_bounds() {
    let sim = Sim::new("kopirka", info(), Copier::new());
    let b = sim.platform().alloc_dma("buf", 16).unwrap();
    b.write::<u32>(12, 0xDEAD_BEEF).unwrap();
    assert_eq!(b.read::<u32>(12).unwrap(), 0xDEAD_BEEF);
    assert_eq!(b.read::<u16>(14).unwrap(), 0xDEAD);
    assert_eq!(
        b.write::<u64>(12, 1),
        Err(DriverError::DmaBounds { buffer: "buf", off: 12, len: 8, size: 16 })
    );
    assert!(b.iova_of(8, 9).is_err());
    assert_eq!(b.iova_of(8, 8).unwrap(), b.iova() + 8);
}

#[test]
fn driver_survives_client_loss_when_asked() {
    let sim = Sim::new("kopirka", info(), Copier::new()).time_limit(50_000_000);
    let p = sim.platform();
    let mut r = Runner::<CopyDriver<_>>::new(&p).ok().unwrap();
    sim.send(b"keep");
    sim.send(b"data");
    sim.close();
    assert_eq!(r.step(&p), None); // keep
    assert_eq!(r.step(&p), None); // data -> start kopie
    assert_eq!(r.step(&p), None); // klient zavrel: ovladac bezi dal
    assert_eq!(r.step(&p), None); // preruseni: odpoved nema komu
    assert_eq!(r.driver.sent_without_client, 1);
    assert_eq!(sim.sent_text(), ["hello"]);
    assert_eq!(p.send(b"x").unwrap(), Sent::NoClient);
    // Bez klienta a bez preruseni uz nic neprijde: simulace to ukonci.
    assert!(matches!(r.step(&p), Some(Exit::Failed { .. })));
}
