//! DMA buffery: pamet, kterou cte a zapisuje zarizeni.
//!
//! Buffer alokuje [`Platform::alloc_dma`](crate::Platform::alloc_dma). Nad
//! jadrem je to fyzicky souvisla pamet vytvorena pres omezeny Resource
//! ovladace: s IOMMU je to jedina pamet, kam zarizeni smi (jina adresa
//! skonci zablokovanim a hlaskou jadra `IOMMU: zablokovan ...`).
//!
//! Adresu pro zarizeni (IOVA) dava jen [`DmaBuffer::iova_of`], ktera hlida,
//! ze cela oblast lezi v bufferu. Ovladac tak nemuze omylem predat zarizeni
//! adresu za koncem bufferu (preteceni indexu, spatna delka).
//!
//! Cisla ve sdilene pameti jsou little endian (vsechna bezna zarizeni PCI).

use crate::io::Width;
use crate::{DriverError, Result};

/// Pamet sdilena se zarizenim (implementuje backend: jadro nebo simulace).
pub trait DmaMemory {
    /// Adresa bajtu 0 z pohledu zarizeni (IOVA; bez IOMMU fyzicka adresa).
    fn iova(&self) -> u64;
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Cteni a zapis; volajici zarucuje meze (dela to [`DmaBuffer`]).
    fn read_bytes(&self, off: u64, buf: &mut [u8]);
    fn write_bytes(&self, off: u64, data: &[u8]);
}

/// DMA buffer s kontrolou mezi a jmenem (pro hlasky).
pub struct DmaBuffer<M: DmaMemory> {
    mem: M,
    name: &'static str,
}

impl<M: DmaMemory> DmaBuffer<M> {
    pub fn new(mem: M, name: &'static str) -> Self {
        Self { mem, name }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn len(&self) -> u64 {
        self.mem.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mem.is_empty()
    }

    /// Pamet pod bufferem (pro backend a knihovny nad nim).
    pub fn memory(&self) -> &M {
        &self.mem
    }

    fn check(&self, off: u64, len: u64) -> Result<()> {
        if crate::io::fits(off, len, self.mem.len()) {
            Ok(())
        } else {
            Err(DriverError::DmaBounds {
                buffer: self.name,
                off,
                len,
                size: self.mem.len(),
            })
        }
    }

    /// Adresa oblasti `off..off+len` pro zarizeni. Jedine misto, odkud ma
    /// ovladac brat adresy, ktere zapisuje do registru a deskriptoru.
    pub fn iova_of(&self, off: u64, len: u64) -> Result<u64> {
        self.check(off, len)?;
        Ok(self.mem.iova() + off)
    }

    /// Adresa celeho bufferu.
    pub fn iova(&self) -> u64 {
        self.mem.iova()
    }

    pub fn read_bytes(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        self.check(off, buf.len() as u64)?;
        self.mem.read_bytes(off, buf);
        Ok(())
    }

    pub fn write_bytes(&self, off: u64, data: &[u8]) -> Result<()> {
        self.check(off, data.len() as u64)?;
        self.mem.write_bytes(off, data);
        Ok(())
    }

    /// Cislo little endian na `off`.
    pub fn read<W: Width>(&self, off: u64) -> Result<W> {
        let mut b = [0u8; 8];
        let n = (W::BITS / 8) as usize;
        self.read_bytes(off, &mut b[..n])?;
        Ok(W::from_u64(u64::from_le_bytes(b)))
    }

    pub fn write<W: Width>(&self, off: u64, v: W) -> Result<()> {
        let n = (W::BITS / 8) as usize;
        self.write_bytes(off, &v.to_u64().to_le_bytes()[..n])
    }

    /// Vynuluje cely buffer (napr. pred predanim zarizeni: zadna stara
    /// data jineho pozadavku).
    pub fn zero(&self) {
        let zeros = [0u8; 256];
        let mut off = 0;
        while off < self.mem.len() {
            let n = (self.mem.len() - off).min(zeros.len() as u64);
            self.mem.write_bytes(off, &zeros[..n as usize]);
            off += n;
        }
    }
}
