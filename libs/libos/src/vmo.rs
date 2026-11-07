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

    /// Objekt pameti na ucet jobu procesu `process` (rodic tak vytvari
    /// pamet potomka; potrebuje pravo `WRITE` na proces).
    pub fn create_for(size: u64, process: &crate::Process) -> Result<Self> {
        sys::vmo_create(size, process.handle().raw()).map(|h| Self(Handle::from_raw(h)))
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
