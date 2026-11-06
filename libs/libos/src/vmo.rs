//! Objekt pameti.

use crate::handle::object_type;
use crate::{Handle, sys};
use abi::Result;

#[derive(Debug)]
pub struct Vmo(Handle);
object_type!(Vmo);

impl Vmo {
    pub fn create(size: u64) -> Result<Self> {
        sys::vmo_create(size).map(|h| Self(Handle::from_raw(h)))
    }

    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
        sys::vmo_read(self.0.raw(), offset, buf)
    }

    pub fn write(&self, offset: u64, data: &[u8]) -> Result<()> {
        sys::vmo_write(self.0.raw(), offset, data)
    }
}
