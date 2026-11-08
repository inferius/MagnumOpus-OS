//! Kruhy deskriptoru v DMA pameti.
//!
//! Dva vzory, ktere pokryji vetsinu zarizeni:
//!
//! - [`DescRing`]: ovladac zapisuje deskriptory na konec (tail), zarizeni
//!   je zpracovava a posouva hlavu (head), kterou hlasi registrem nebo
//!   zapisem do pameti. Tak funguji kruhy e1000/igc, r8169, NVMe
//!   submission queue, AHCI seznam prikazu.
//! - [`PhaseRing`]: zarizeni zapisuje polozky dokonceni a kazda nese fazovy
//!   bit, ktery se po kazdem obehu kruhu prevrati. Ovladac pozna novou
//!   polozku podle faze bez cteni registru (NVMe completion queue, xHCI
//!   event ring).
//!
//! Kruh hlida meze (slot je vzdy uvnitr bufferu) a stav (plny kruh se
//! neprepise). Bariery pred zapisem indexu do registru zarizeni dela
//! ovladac pres [`Platform::dma_wmb`](crate::Platform::dma_wmb).

use crate::dma::{DmaBuffer, DmaMemory};
use crate::{DriverError, Result};

/// Kruh deskriptoru pevne velikosti (producent = ovladac).
pub struct DescRing<M: DmaMemory> {
    buf: DmaBuffer<M>,
    name: &'static str,
    entry_size: u64,
    count: u32,
    /// Prvni slot, ktery zarizeni jeste nezpracovalo.
    head: u32,
    /// Prvni volny slot (sem pise ovladac).
    tail: u32,
}

impl<M: DmaMemory> DescRing<M> {
    /// Kruh `count` polozek po `entry_size` bajtech nad bufferem `buf`.
    /// Jeden slot zustava vzdy volny (plny a prazdny kruh se lisi), takze
    /// najednou je ve fronte nejvys `count - 1` deskriptoru, stejne jako
    /// u hardwaru s registry head/tail.
    pub fn new(buf: DmaBuffer<M>, name: &'static str, entry_size: u64, count: u32) -> Result<Self> {
        if count < 2 || entry_size == 0 {
            return Err(DriverError::Config(alloc::format!(
                "kruh '{name}': aspon 2 polozky nenulove velikosti"
            )));
        }
        let need = entry_size * count as u64;
        if need > buf.len() {
            return Err(DriverError::DmaBounds {
                buffer: buf.name(),
                off: 0,
                len: need,
                size: buf.len(),
            });
        }
        buf.zero();
        Ok(Self {
            buf,
            name,
            entry_size,
            count,
            head: 0,
            tail: 0,
        })
    }

    pub fn count(&self) -> u32 {
        self.count
    }

    pub fn head(&self) -> u32 {
        self.head
    }

    /// Index pro registr "tail" zarizeni.
    pub fn tail(&self) -> u32 {
        self.tail
    }

    /// Pocet deskriptoru, ktere ceka na zarizeni.
    pub fn pending(&self) -> u32 {
        (self.tail + self.count - self.head) % self.count
    }

    pub fn free(&self) -> u32 {
        self.count - 1 - self.pending()
    }

    /// Adresa zacatku kruhu (do registru "base" zarizeni).
    pub fn base_iova(&self) -> u64 {
        self.buf.iova()
    }

    /// Bajty kruhu (do registru "length").
    pub fn bytes(&self) -> u64 {
        self.entry_size * self.count as u64
    }

    fn slot_off(&self, slot: u32) -> u64 {
        slot as u64 * self.entry_size
    }

    /// Zapise deskriptor na konec kruhu a vrati jeho slot. Index pro
    /// zarizeni ([`tail`](Self::tail)) zapise ovladac do registru az po
    /// `dma_wmb`.
    pub fn push(&mut self, desc: &[u8]) -> Result<u32> {
        if desc.len() as u64 > self.entry_size {
            return Err(DriverError::DmaBounds {
                buffer: self.buf.name(),
                off: self.slot_off(self.tail),
                len: desc.len() as u64,
                size: self.entry_size,
            });
        }
        if self.free() == 0 {
            return Err(DriverError::RingFull { ring: self.name });
        }
        let slot = self.tail;
        let off = self.slot_off(slot);
        self.buf.write_bytes(off, desc)?;
        // Zbytek polozky nulovat: zadne zbytky stareho deskriptoru.
        let rest = (self.entry_size - desc.len() as u64) as usize;
        if rest > 0 {
            let zeros = alloc::vec![0u8; rest];
            self.buf.write_bytes(off + desc.len() as u64, &zeros)?;
        }
        self.tail = (self.tail + 1) % self.count;
        Ok(slot)
    }

    /// Zarizeni hlasi novou hlavu `device_head`: vrati sloty, ktere tim
    /// dokoncilo (v poradi). Hlava mimo kruh nebo pred koncem fronty (by
    /// "dokoncila" i nevlozene deskriptory) je vadne zarizeni.
    pub fn complete_to(&mut self, device_head: u32) -> Result<CompletedSlots> {
        if device_head >= self.count {
            return Err(DriverError::Device(alloc::format!(
                "kruh '{}': hlava {device_head} mimo kruh ({} polozek)",
                self.name, self.count
            )));
        }
        let done = (device_head + self.count - self.head) % self.count;
        if done > self.pending() {
            return Err(DriverError::Device(alloc::format!(
                "kruh '{}': zarizeni dokoncilo {done} deskriptoru, ve fronte je {}",
                self.name,
                self.pending()
            )));
        }
        let first = self.head;
        self.head = device_head;
        Ok(CompletedSlots {
            next: first,
            left: done,
            count: self.count,
        })
    }

    /// Precte deskriptor ve slotu (napr. stav, ktery do nej zapsalo
    /// zarizeni).
    pub fn read_entry(&self, slot: u32, out: &mut [u8]) -> Result<()> {
        self.check_slot(slot, out.len() as u64)?;
        self.buf.read_bytes(self.slot_off(slot), out)
    }

    /// Prepise deskriptor ve slotu (napr. vraceni bufferu prijmu).
    pub fn write_entry(&self, slot: u32, data: &[u8]) -> Result<()> {
        self.check_slot(slot, data.len() as u64)?;
        self.buf.write_bytes(self.slot_off(slot), data)
    }

    fn check_slot(&self, slot: u32, len: u64) -> Result<()> {
        if slot >= self.count || len > self.entry_size {
            return Err(DriverError::DmaBounds {
                buffer: self.buf.name(),
                off: self.slot_off(slot),
                len,
                size: self.entry_size,
            });
        }
        Ok(())
    }

    /// Pristup k bufferu kruhu.
    pub fn buffer(&self) -> &DmaBuffer<M> {
        &self.buf
    }
}

/// Sloty dokoncene zarizenim (iterator z [`DescRing::complete_to`]).
pub struct CompletedSlots {
    next: u32,
    left: u32,
    count: u32,
}

impl Iterator for CompletedSlots {
    type Item = u32;
    fn next(&mut self) -> Option<u32> {
        if self.left == 0 {
            return None;
        }
        let s = self.next;
        self.next = (self.next + 1) % self.count;
        self.left -= 1;
        Some(s)
    }
}

/// Kruh dokonceni s fazovym bitem (producent = zarizeni).
pub struct PhaseRing<M: DmaMemory> {
    buf: DmaBuffer<M>,
    entry_size: u64,
    count: u32,
    head: u32,
    /// Ocekavana faze nove polozky (po resetu 1).
    phase: bool,
    /// Kde je fazovy bit: bajt v polozce a bit v nem.
    phase_byte: u64,
    phase_bit: u8,
}

impl<M: DmaMemory> PhaseRing<M> {
    /// Kruh `count` polozek po `entry_size` B; fazovy bit je bit
    /// `phase_bit` bajtu `phase_byte` polozky (NVMe CQ: bajt 14, bit 0).
    pub fn new(
        buf: DmaBuffer<M>,
        entry_size: u64,
        count: u32,
        phase_byte: u64,
        phase_bit: u8,
    ) -> Result<Self> {
        if count == 0 || phase_byte >= entry_size || phase_bit > 7 {
            return Err(DriverError::Config(alloc::string::String::from(
                "kruh dokonceni: fazovy bit mimo polozku",
            )));
        }
        let need = entry_size * count as u64;
        if need > buf.len() {
            return Err(DriverError::DmaBounds {
                buffer: buf.name(),
                off: 0,
                len: need,
                size: buf.len(),
            });
        }
        buf.zero();
        Ok(Self {
            buf,
            entry_size,
            count,
            head: 0,
            phase: true,
            phase_byte,
            phase_bit,
        })
    }

    pub fn base_iova(&self) -> u64 {
        self.buf.iova()
    }

    /// Index pro registr "head" (doorbell) zarizeni.
    pub fn head(&self) -> u32 {
        self.head
    }

    /// Dalsi nova polozka: precte ji do `out` a posune hlavu. `None` =
    /// zarizeni zatim nic nezapsalo. Ovladac pred ctenim zavola `dma_rmb`.
    pub fn pop(&mut self, out: &mut [u8]) -> Result<Option<u32>> {
        let off = self.head as u64 * self.entry_size;
        let b: u8 = self.buf.read(off + self.phase_byte)?;
        if (b >> self.phase_bit) & 1 != self.phase as u8 {
            return Ok(None);
        }
        let n = (out.len() as u64).min(self.entry_size) as usize;
        self.buf.read_bytes(off, &mut out[..n])?;
        let slot = self.head;
        self.head += 1;
        if self.head == self.count {
            self.head = 0;
            self.phase = !self.phase;
        }
        Ok(Some(slot))
    }
}
