//! Pristup k initfs (obraz predany procesu init pri startu).

use abi::vm::MAP_READ;
use abi::{Error, Result};

use crate::rt::Startup;
use crate::{Vmo, sys};

/// Namapovany obraz initfs (jen pro cteni, zije do konce procesu).
pub struct Initfs {
    fs: initfs::Initfs<'static>,
}

impl Initfs {
    /// Namapuje initfs predany ve startovni zprave.
    pub fn open(startup: &Startup) -> Result<Self> {
        let vmo = startup.initfs.as_ref().ok_or(Error::NotFound)?;
        Self::map(vmo, startup)
    }

    fn map(vmo: &Vmo, startup: &Startup) -> Result<Self> {
        // Velikost zjistime z hlavicky (prvni 4 KiB), pak namapujeme cele.
        let mut header = [0u8; 4096];
        vmo.read(0, &mut header)?;
        let count = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
        let mut end = 0u64;
        for i in 0..count {
            let e = initfs::HEADER_SIZE + i * initfs::ENTRY_SIZE;
            let mut entry = [0u8; initfs::ENTRY_SIZE];
            vmo.read(e as u64, &mut entry)?;
            let off = u64::from_le_bytes(entry[8..16].try_into().unwrap());
            let len = u64::from_le_bytes(entry[16..24].try_into().unwrap());
            end = end.max(off + len);
        }
        let page = crate::vm::page_size();
        let len = end.max(page).next_multiple_of(page);
        let addr = sys::vm_map(
            startup.process.handle().raw(),
            vmo.handle().raw(),
            0,
            len,
            MAP_READ,
            0,
        )?;
        // SAFETY: rozsah je namapovany jen pro cteni a zustane namapovany do
        // konce procesu (nikdo ho neodmapuje).
        let bytes = unsafe { core::slice::from_raw_parts(addr as *const u8, len as usize) };
        let fs = initfs::Initfs::parse(bytes).map_err(|_| Error::Corrupted)?;
        Ok(Self { fs })
    }

    pub fn find(&self, name: &str) -> Option<&'static [u8]> {
        self.fs.find(name).map(|f| f.data)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.fs.files().map(|f| f.name)
    }
}
