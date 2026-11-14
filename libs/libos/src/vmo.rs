//! Objekt pameti.

use crate::handle::object_type;
use crate::{Handle, sys};
use abi::Result;

/// Nejvic bajtu jednoho `VMO_READ`/`VMO_WRITE` (limit jadra).
pub const IO_MAX: usize = 1 << 20;

#[derive(Debug)]
pub struct Vmo(Handle);
object_type!(Vmo);

impl Vmo {
    pub fn create(size: u64) -> Result<Self> {
        sys::vmo_create(size, 0).map(|h| Self(Handle::from_raw(h)))
    }

    /// Pripnuty objekt pameti (`VMO_PINNED`): vsechny stranky hned
    /// a pripnute, aby k nim smelo zarizeni (`DMA_MAP`, buffer pro ovladac).
    /// Strhava se i rozpocet pripnute pameti jobu (`job::pool::DMA`).
    pub fn create_pinned(size: u64) -> Result<Self> {
        sys::vmo_create_with(size, 0, abi::vm::VMO_PINNED).map(|h| Self(Handle::from_raw(h)))
    }

    /// Objekt pameti na ucet jobu procesu `process` (rodic tak vytvari
    /// pamet potomka; potrebuje pravo `WRITE` na proces).
    pub fn create_for(size: u64, process: &crate::Process) -> Result<Self> {
        sys::vmo_create(size, process.handle().raw()).map(|h| Self(Handle::from_raw(h)))
    }

    /// Zapeceti objekt (`VMO_SEAL`): obsah se uz nikdy nezmeni. Bez
    /// opravneni JIT jde spoustet jen zapecetena pamet (kod nahrany
    /// zapisem, pak pecet, pak mapovani pro spusteni).
    pub fn seal(&self) -> Result<()> {
        sys::vmo_seal(self.handle().raw())
    }

    /// Klon `len` bajtu od `offset` s kopii pri zapisu (`VMO_CLONE`): sdili
    /// stranky, dokud do nich nekdo z obou nezapise. Na ucet vlastniho jobu.
    pub fn clone_range(&self, offset: u64, len: u64) -> Result<Self> {
        sys::vmo_clone(self.handle().raw(), offset, len, 0).map(|h| Self(Handle::from_raw(h)))
    }

    /// Jako [`Vmo::clone_range`], ale na ucet jobu procesu `process`
    /// (rodic pri spousteni programu).
    pub fn clone_for(&self, offset: u64, len: u64, process: &crate::Process) -> Result<Self> {
        sys::vmo_clone(self.handle().raw(), offset, len, process.handle().raw())
            .map(|h| Self(Handle::from_raw(h)))
    }

    /// Precte data objektu (libovolne dlouhy usek; jadro prenese nejvys
    /// `IO_MAX` na volani, proto po castech).
    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        for (i, chunk) in buf.chunks_mut(IO_MAX).enumerate() {
            sys::vmo_read(self.0.raw(), offset + (i * IO_MAX) as u64, chunk)?;
        }
        Ok(())
    }

    /// Zapise data do objektu (po castech jako `read`).
    pub fn write(&self, offset: u64, data: &[u8]) -> Result<()> {
        for (i, chunk) in data.chunks(IO_MAX).enumerate() {
            sys::vmo_write(self.0.raw(), offset + (i * IO_MAX) as u64, chunk)?;
        }
        Ok(())
    }
}
