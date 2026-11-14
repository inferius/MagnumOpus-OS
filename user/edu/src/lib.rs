//! edu: ukazkovy ovladac na driverkitu pro vyukove zarizeni QEMU `edu`
//! (`-device edu`, PCI 1234:11e8).
//!
//! Zarizeni umi presne to, co potrebuje kazdy ovladac: registry v BAR0,
//! preruseni (MSI) po dokonceni prace a DMA do pameti pocitace. Proto je
//! to prvni ovladac v prirucce (`docs/cs/ovladace.md`).
//!
//! Protokol pro klienta (text, jedna zprava = jeden prikaz):
//!
//! | zprava       | odpoved                                   |
//! |--------------|-------------------------------------------|
//! | `fact <n>`   | `fact <n> <n!>` (32 bitu, pretece jako hw) |
//! | `dma <text>` | `dma <text>`: text projde DMA do zarizeni a zpet |
//! | `sg <text>`  | `sg <text>`: jako `dma`, ale pres buffer po strankach (`DMA_MAP`) pres hranice stranek |
//! | `stale`      | `stale <iova>`: zarizeni zapise na adresu odmapovaneho bufferu (jen s IOMMU; jadro pristup zablokuje a zaloguje) |
//! | jiny prikaz  | `chyba <popis>`                           |
//!
//! Ovladac je genericky (`Edu<P: Platform>`): v systemu bezi nad jadrem,
//! v testech nad simulaci (`src/model.rs`).
//!
//! Fakta o zarizeni: QEMU `docs/specs/edu.rst` (kod QEMU je pod GPL,
//! prevzata jsou jen fakta o registrech).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod model;
#[cfg(test)]
mod tests;

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;

use driverkit::{DmaBuffer, Driver, DriverError, IrqId, Platform, Result};

/// Identifikace zarizeni na PCI.
pub const VENDOR: u16 = 0x1234;
pub const DEVICE: u16 = 0x11e8;

/// Vnitrni buffer zarizeni pro DMA (adresa z pohledu zarizeni) a jeho
/// velikost.
pub const EDU_BUF: u64 = 0x40000;
pub const EDU_BUF_SIZE: u64 = 4096;

/// Buffer po strankach pro `sg` (tri stranky: text lezi pres hranici).
pub const SG_SIZE: u64 = 3 * 4096;

driverkit::registers! {
    /// Identifikace `0xRRrr00ed`: RR hlavni verze, rr vedlejsi.
    pub ID @ 0x00: u32 {
        MAGIC: 0..=7,
        MINOR: 16..=23,
        MAJOR: 24..=31,
    }
    /// Kontrola zivota: precte se zapsana hodnota s prevracenymi bity.
    pub LIVENESS @ 0x04: u32;
    /// Zapis cisla spusti vypocet faktorialu, cteni vrati vysledek.
    pub FACTORIAL @ 0x08: u32;
    pub STATUS @ 0x20: u32 {
        /// Vypocet probiha (jen cteni).
        BUSY: 0,
        /// Po vypoctu vyvolat preruseni.
        IRQ_FACT: 7,
    }
    /// Proc zarizeni vyvolalo preruseni.
    pub IRQ_STATUS @ 0x24: u32 {
        FACT: 0,
        DMA: 8,
    }
    /// Zapis potvrdi preruseni (smaze zapsane bity IRQ_STATUS).
    pub IRQ_ACK @ 0x64: u32;
    pub DMA_SRC @ 0x80: u64;
    pub DMA_DST @ 0x88: u64;
    pub DMA_COUNT @ 0x90: u64;
    pub DMA_CMD @ 0x98: u64 {
        /// Spustit prenos (zarizeni bit smaze po dokonceni).
        START: 0,
        /// Smer: 0 = pamet -> zarizeni, 1 = zarizeni -> pamet.
        TO_RAM: 1,
        /// Po prenosu vyvolat preruseni.
        IRQ: 2,
    }
}

/// Jeden prenos DMA: (zdroj, cil, bajtu, do pameti).
type Transfer = (u64, u64, u64, bool);

/// Co zarizeni prave dela.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    Idle,
    Factorial(u32),
    /// DMA tam (pamet -> zarizeni), pak zpet do druheho bufferu.
    DmaToDevice(u64),
    DmaToRam(u64),
    /// `sg`: prenosy po kusech (kus = souvisly usek pro zarizeni); `back`
    /// = uz probiha cesta zpet, `len` delka textu.
    Sg { queue: VecDeque<Transfer>, back: bool, len: u64 },
    /// `stale`: zarizeni zapisuje na adresu odmapovaneho bufferu.
    Stale(u64),
}

/// Kde lezi text v bufferu `sg`: tam (pres hranici stranek 0/1) a zpet
/// (pres hranici 1/2).
fn sg_offsets(len: u64) -> (u64, u64) {
    (4096 - len / 2, 2 * 4096 - len / 2)
}

/// Ovladac zarizeni edu.
pub struct Edu<P: Platform> {
    regs: P::Mmio,
    irq: IrqId,
    /// Data pro zarizeni a data od zarizeni (dva buffery: DMA zpet nesmi
    /// prepsat zdroj, at jde roundtrip overit).
    out: DmaBuffer<P::Dma>,
    back: DmaBuffer<P::Dma>,
    /// Buffer po strankach (pripnute nesouvisle stranky, `DMA_MAP`).
    sg: DmaBuffer<P::Dma>,
    op: Op,
}

impl<P: Platform> Driver<P> for Edu<P> {
    fn probe(p: &P) -> Result<Self> {
        let regs = p.map_bar(0)?;
        let id = ID.read(&regs);
        if ID::MAGIC.get(id) != 0xed {
            return Err(DriverError::Device(format!("neznama identifikace {id:#x}")));
        }
        // Kontrola zivota: karta vrati prevracene bity.
        LIVENESS.write(&regs, 0x1234_5678);
        if LIVENESS.read(&regs) != !0x1234_5678 {
            return Err(DriverError::Device(String::from("karta neprosla kontrolou zivota")));
        }
        let irq = p.enable_irq()?;
        let out = p.alloc_dma("edu-tam", EDU_BUF_SIZE)?;
        let back = p.alloc_dma("edu-zpet", EDU_BUF_SIZE)?;
        let sg = p.alloc_dma_pages("edu-sg", SG_SIZE)?;
        driverkit::log!(
            p,
            "karta edu {}.{} zije",
            ID::MAJOR.get(id),
            ID::MINOR.get(id)
        );
        Ok(Self {
            regs,
            irq,
            out,
            back,
            sg,
            op: Op::Idle,
        })
    }

    fn message(&mut self, p: &P, msg: &[u8]) -> Result<()> {
        let text = core::str::from_utf8(msg).unwrap_or("");
        if self.op != Op::Idle {
            return reply(p, "chyba zarizeni je zaneprazdnene");
        }
        if let Some(n) = text.strip_prefix("fact ") {
            let Ok(n) = n.trim().parse::<u32>() else {
                return reply(p, "chyba fact ocekava cislo");
            };
            STATUS::IRQ_FACT.set(&self.regs);
            FACTORIAL.write(&self.regs, n);
            self.op = Op::Factorial(n);
            return Ok(());
        }
        if let Some(data) = text.strip_prefix("dma ") {
            let len = data.len() as u64;
            if len == 0 || len > EDU_BUF_SIZE {
                return reply(p, "chyba dma: 1-4096 bajtu");
            }
            self.out.write_bytes(0, data.as_bytes())?;
            self.start_dma(p, self.out.iova_of(0, len)?, EDU_BUF, len, false)?;
            self.op = Op::DmaToDevice(len);
            return Ok(());
        }
        if let Some(data) = text.strip_prefix("sg ") {
            let len = data.len() as u64;
            if len == 0 || len > EDU_BUF_SIZE {
                return reply(p, "chyba sg: 1-4096 bajtu");
            }
            let (there, back) = sg_offsets(len);
            self.sg.write_bytes(there, data.as_bytes())?;
            // Cil cesty zpet vynulovat: stara data by test oklamala.
            self.sg.write_bytes(back, &alloc::vec![0u8; len as usize])?;
            p.dma_wmb();
            let mut queue = VecDeque::new();
            let mut dev = EDU_BUF;
            for (addr, n) in self.sg.device_chunks(there, len)? {
                queue.push_back((addr, dev, n, false));
                dev += n;
            }
            self.op = Op::Sg { queue, back: false, len };
            return self.next_sg(p);
        }
        if text == "stale" {
            // Bez IOMMU by zarizeni zapsalo do stranky, kterou uz mohl
            // dostat kdokoli jiny: test ochrany jen s overenou IOMMU.
            if !self.sg.is_protected() {
                return reply(p, "chyba stale: bez IOMMU nejde (zarizeni by zapsalo do uvolnene pameti)");
            }
            let iova = {
                let tmp = p.alloc_dma_pages("edu-stale", 4096)?;
                tmp.iova_of(0, 64)?
                // `tmp` zanikne: odmapovani s vyprazdnenim IOTLB.
            };
            self.start_dma(p, EDU_BUF, iova, 64, true)?;
            self.op = Op::Stale(iova);
            return Ok(());
        }
        reply(p, "chyba neznamy prikaz")
    }

    fn interrupt(&mut self, p: &P, irq: IrqId) -> Result<()> {
        debug_assert_eq!(irq, self.irq);
        let why = IRQ_STATUS.read(&self.regs);
        // Potvrdit driv nez dalsi prace: nove preruseni se neztrati.
        IRQ_ACK.write(&self.regs, why);
        match self.op.clone() {
            Op::Factorial(n) if IRQ_STATUS::FACT.get(why) != 0 => {
                let v = FACTORIAL.read(&self.regs);
                self.op = Op::Idle;
                reply(p, &format!("fact {n} {v}"))
            }
            Op::DmaToDevice(len) if IRQ_STATUS::DMA.get(why) != 0 => {
                self.start_dma(p, EDU_BUF, self.back.iova_of(0, len)?, len, true)?;
                self.op = Op::DmaToRam(len);
                Ok(())
            }
            Op::DmaToRam(len) if IRQ_STATUS::DMA.get(why) != 0 => {
                p.dma_rmb();
                let mut data = alloc::vec![0u8; len as usize];
                self.back.read_bytes(0, &mut data)?;
                self.op = Op::Idle;
                reply(p, &format!("dma {}", String::from_utf8_lossy(&data)))
            }
            Op::Sg { .. } if IRQ_STATUS::DMA.get(why) != 0 => self.next_sg(p),
            Op::Stale(iova) if IRQ_STATUS::DMA.get(why) != 0 => {
                // Pristup zablokovala IOMMU; hlaseni vybere jadro do logu
                // pri DMA_SYNC (SMMUv3 nema preruseni udalosti).
                p.dma_sync_for_cpu(&self.sg);
                self.op = Op::Idle;
                reply(p, &format!("stale {iova:#x}"))
            }
            // Cizi nebo opozdene preruseni (napr. po restartu ovladace).
            _ => Ok(()),
        }
    }

    fn stop(&mut self, _p: &P) {
        // Zadne dalsi preruseni ani DMA: vypnout preruseni po vypoctu.
        // Rozbehnuty prenos edu nejde zrusit, dobehne do bufferu, ktere
        // patri ovladaci (s IOMMU je po konci procesu zarizeni nedosahne).
        STATUS::IRQ_FACT.clear(&self.regs);
    }
}

impl<P: Platform> Edu<P> {
    /// Dalsi kus `sg`: prenos z fronty, po ceste tam cesta zpet, po ni
    /// odpoved.
    fn next_sg(&mut self, p: &P) -> Result<()> {
        let Op::Sg { queue, back, len } = &mut self.op else {
            return Ok(());
        };
        let (back_done, len) = (*back, *len);
        if let Some((src, dst, n, to_ram)) = queue.pop_front() {
            return self.start_dma(p, src, dst, n, to_ram);
        }
        let (_, back_at) = sg_offsets(len);
        if !back_done {
            let mut queue = VecDeque::new();
            let mut dev = EDU_BUF;
            for (addr, n) in self.sg.device_chunks(back_at, len)? {
                queue.push_back((dev, addr, n, true));
                dev += n;
            }
            self.op = Op::Sg { queue, back: true, len };
            return self.next_sg(p);
        }
        p.dma_sync_for_cpu(&self.sg);
        p.dma_rmb();
        let mut data = alloc::vec![0u8; len as usize];
        self.sg.read_bytes(back_at, &mut data)?;
        self.op = Op::Idle;
        reply(p, &format!("sg {}", String::from_utf8_lossy(&data)))
    }

    /// Spusti prenos DMA `count` bajtu ze `src` do `dst` s prerusenim po
    /// dokonceni.
    fn start_dma(&self, p: &P, src: u64, dst: u64, count: u64, to_ram: bool) -> Result<()> {
        DMA_SRC.write(&self.regs, src);
        DMA_DST.write(&self.regs, dst);
        DMA_COUNT.write(&self.regs, count);
        let cmd = DMA_CMD::START.value(1)? | DMA_CMD::IRQ.value(1)? | DMA_CMD::TO_RAM.value(to_ram as u64)?;
        // Data v pameti driv, nez zarizeni zacne prenos.
        p.mmio_wmb();
        DMA_CMD.write(&self.regs, cmd);
        Ok(())
    }
}

fn reply<P: Platform>(p: &P, text: &str) -> Result<()> {
    p.send(text.as_bytes())?;
    Ok(())
}
