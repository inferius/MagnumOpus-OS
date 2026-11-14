//! Pristup k initfs (obraz predany procesu init pri startu).

use abi::vm::MAP_READ;
use abi::{Error, Result};

use alloc::vec::Vec;

use crate::rt::{Startup, self_process};
use crate::sync::Mutex;
use crate::{Handle, Vmo, sys};

/// Namapovane obrazy initfs (zacatek, delka). Obsah initfs je nemenny
/// (nikdo do nej nepise), takze spousteni smi programy z nej sdilet podle
/// offsetu (`offset_of`, sablony v `spawn`).
static MAPPED: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

/// Prvni namapovany initfs: obsah (zavadec `ld.so` pro `spawn` programu
/// s `PT_INTERP`) a syrovy handle jeho objektu (zavadec potomka z nej cte
/// knihovny, `abi::bootstrap::LOADER_MAGIC`).
static GLOBAL: Mutex<Option<(initfs::Initfs<'static>, u32)>> = Mutex::new(None);

/// Soubor `name` v initfs tohoto procesu (`None` = proces initfs nema).
pub(crate) fn global_find(name: &str) -> Option<&'static [u8]> {
    GLOBAL.lock().as_ref().and_then(|(fs, _)| fs.find(name).map(|f| f.data))
}

/// Novy handle na objekt initfs jen pro cteni a mapovani (pro zavadec
/// potomka).
pub(crate) fn global_vmo() -> Result<Handle> {
    let raw = GLOBAL.lock().as_ref().map(|(_, h)| *h).ok_or(Error::NotFound)?;
    let r = abi::object::Rights(abi::object::Rights::READ.0 | abi::object::Rights::MAP.0 | abi::object::Rights::BASIC.0);
    let raw = sys::handle_duplicate(raw, Some(r.0))?;
    Ok(Handle::from_raw(raw))
}

/// Offset obrazu `image` v initfs, lezi-li cely v nekterem namapovanem
/// initfs; jinak `None` (obraz odjinud se nesdili).
pub(crate) fn offset_of(image: &[u8]) -> Option<u64> {
    let (p, len) = (image.as_ptr() as u64, image.len() as u64);
    MAPPED
        .lock()
        .iter()
        .find(|&&(a, l)| p >= a && p.checked_add(len).is_some_and(|e| e <= a + l))
        .map(|&(a, _)| p - a)
}

/// Namapovany obraz initfs (jen pro cteni, zije do konce procesu).
pub struct Initfs {
    fs: initfs::Initfs<'static>,
}

impl Initfs {
    /// Namapuje initfs predany ve startovni zprave.
    pub fn open(startup: &Startup) -> Result<Self> {
        let vmo = startup.initfs.as_ref().ok_or(Error::NotFound)?;
        Self::map(vmo)
    }

    /// Namapuje initfs z objektu `vmo` (zavadec `ld.so`: handle ze zpravy
    /// zavadece, ne ze startovni zpravy).
    pub fn from_vmo(vmo: &Vmo) -> Result<Self> {
        Self::map(vmo)
    }

    fn map(vmo: &Vmo) -> Result<Self> {
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
            self_process(),
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
        MAPPED.lock().push((addr, len));
        let mut g = GLOBAL.lock();
        if g.is_none()
            && let Ok(raw) = sys::handle_duplicate(vmo.handle().raw(), None)
        {
            *g = Some((fs, raw));
        }
        Ok(Self { fs })
    }

    /// Odmapuje initfs a zavre ulozeny handle (zavadec pred skokem do
    /// programu: program initfs nedostane). Rezy z `find` pak neplati.
    pub fn close(self) {
        let start = self.fs.bytes().as_ptr() as u64;
        let mut mapped = MAPPED.lock();
        if let Some(i) = mapped.iter().position(|&(a, _)| a == start) {
            let (a, l) = mapped.remove(i);
            let _ = sys::vm_unmap(self_process(), a, l);
        }
        drop(mapped);
        if let Some((_, h)) = GLOBAL.lock().take() {
            let _ = sys::handle_close(h);
        }
    }

    pub fn find(&self, name: &str) -> Option<&'static [u8]> {
        self.fs.find(name).map(|f| f.data)
    }

    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.fs.files().map(|f| f.name)
    }
}
