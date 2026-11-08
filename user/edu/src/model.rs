//! Model zarizeni edu pro testy na hostiteli (podle QEMU
//! `docs/specs/edu.rst`): faktorial za 10 us, DMA za 100 ms, preruseni
//! po dokonceni.

use driverkit::sim::{Bus, DeviceModel, RegFile};

use crate::{EDU_BUF, EDU_BUF_SIZE};

const ID: u64 = 0x00;
const LIVENESS: u64 = 0x04;
const FACTORIAL: u64 = 0x08;
const STATUS: u64 = 0x20;
const IRQ_STATUS: u64 = 0x24;
const IRQ_RAISE: u64 = 0x60;
const IRQ_ACK: u64 = 0x64;
const DMA_SRC: u64 = 0x80;
const DMA_DST: u64 = 0x88;
const DMA_COUNT: u64 = 0x90;
const DMA_CMD: u64 = 0x98;

pub struct EduModel {
    regs: RegFile,
    buf: [u8; EDU_BUF_SIZE as usize],
    fact_done: Option<u64>,
    dma_done: Option<u64>,
}

impl EduModel {
    pub fn new() -> Self {
        Self {
            regs: RegFile::new()
                .ro(ID, 0x0100_00ed)
                .rw(LIVENESS, 0)
                .rw(FACTORIAL, 0)
                // BUSY (bit 0) jen cteni.
                .reg(STATUS, 0, 1, 0)
                .ro(IRQ_STATUS, 0)
                .rw(DMA_SRC, 0)
                .rw(DMA_DST, 0)
                .rw(DMA_COUNT, 0)
                .rw(DMA_CMD, 0),
            buf: [0; EDU_BUF_SIZE as usize],
            fact_done: None,
            dma_done: None,
        }
    }

    /// Vnitrni buffer zarizeni (pro kontrolu v testu).
    pub fn buffer(&self) -> &[u8] {
        &self.buf
    }

    fn raise(&mut self, bus: &mut Bus, why: u64) {
        self.regs.set_bits(IRQ_STATUS, why);
        bus.raise_irq(0);
    }

    fn dma(&mut self, bus: &mut Bus) {
        let cmd = self.regs.get(DMA_CMD);
        let (src, dst, n) = (self.regs.get(DMA_SRC), self.regs.get(DMA_DST), self.regs.get(DMA_COUNT));
        let to_ram = cmd & 2 != 0;
        // Adresa uvnitr zarizeni musi lezet v jeho bufferu.
        let dev = if to_ram { src } else { dst };
        if dev < EDU_BUF || dev + n > EDU_BUF + EDU_BUF_SIZE {
            bus.note("edu: adresa bufferu zarizeni mimo rozsah, prenos zahozen");
        } else {
            let at = (dev - EDU_BUF) as usize;
            let part = &mut self.buf[at..at + n as usize];
            // Chyba DMA (adresa mimo pamet ovladace) zaznamena simulace.
            let _ = if to_ram { bus.dma_write(dst, part) } else { bus.dma_read(src, part) };
        }
        self.regs.set(DMA_CMD, cmd & !1);
        if cmd & 4 != 0 {
            self.raise(bus, 0x100);
        }
    }
}

impl DeviceModel for EduModel {
    fn bar_size(&self, index: u8) -> Option<u64> {
        (index == 0).then_some(1 << 20)
    }

    fn read(&mut self, _: &mut Bus, _: u8, off: u64, _: u8) -> u64 {
        self.regs.read(off)
    }

    fn write(&mut self, bus: &mut Bus, _: u8, off: u64, _: u8, v: u64) {
        match off {
            LIVENESS => self.regs.set(LIVENESS, !v & 0xFFFF_FFFF),
            FACTORIAL if self.regs.get(STATUS) & 1 == 0 => {
                self.regs.set(FACTORIAL, v);
                self.regs.set_bits(STATUS, 1);
                self.fact_done = Some(bus.now() + 10_000);
            }
            IRQ_RAISE => self.raise(bus, v),
            IRQ_ACK => self.regs.clear_bits(IRQ_STATUS, v),
            DMA_CMD if v & 1 != 0 && self.regs.get(DMA_CMD) & 1 == 0 => {
                self.regs.set(DMA_CMD, v);
                self.dma_done = Some(bus.now() + 100_000_000);
            }
            _ => {
                self.regs.write(off, v);
            }
        }
    }

    fn tick(&mut self, bus: &mut Bus) {
        if self.fact_done.is_some_and(|t| bus.now() >= t) {
            self.fact_done = None;
            let n = self.regs.get(FACTORIAL) as u32;
            let v = (1..=n).fold(1u32, |a, k| a.wrapping_mul(k));
            self.regs.set(FACTORIAL, v as u64);
            self.regs.clear_bits(STATUS, 1);
            if self.regs.get(STATUS) & 0x80 != 0 {
                self.raise(bus, 1);
            }
        }
        if self.dma_done.is_some_and(|t| bus.now() >= t) {
            self.dma_done = None;
            self.dma(bus);
        }
    }

    fn as_any(&mut self) -> &mut dyn core::any::Any {
        self
    }
}
