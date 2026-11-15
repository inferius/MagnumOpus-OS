//! Objekty pameti souboru pro `mmap` (fsproto v2 `MAP_VMO`, docs/cs/souborovy-system.md,
//! kapitola Soubory jako pamet).
//!
//! fsd drzi pro otevreny soubor objekt pameti s jeho obsahem (cache podle
//! dataset + oid), naplni ho cely pri prvnim mapovani a klientovi da
//! handle **jen pro cteni**; `MAP_PRIVATE` si klient udela klonem s kopii
//! pri zapisu (`VMO_CLONE`), takze se nic nekopiruje. Platnost hlida verze
//! obsahu (`Revision::version`): po zmene souboru dostane dalsi mapovani
//! novy objekt, existujici soukrome mapovani zustane snimkem (POSIX to
//! nechava nespecifikovane).
//!
//! Pamet objektu se pocita do rozpoctu fsd (vytvari je fsd), ne do jobu
//! klienta; zneuziti brani kvota na relaci (`SESSION_QUOTA`) a limit
//! souboru (`MAX_FILE`). Cache bez odkazu se zahazuje od nejstarsiho
//! pouziti (`CACHE_MAX`); klientum zustanou jejich handly a klony.
//!
//! Pozdeji (pager v jadre, plneni po strankach pri vypadku) se zmeni jen
//! plneni zde: klient dal dostane handle a nic dalsiho.

use alloc::collections::BTreeMap;

use libos::{Error, Result, Vmo};

/// Nejvetsi soubor, ktery se mapuje pres objekt pameti (vetsi = kopie
/// pres `READ_VMO` v klientovi).
pub const MAX_FILE: u64 = 256 * 1024 * 1024;
/// Nejvic bajtu ruznych souboru, ktere jedna relace dostala k mapovani.
pub const SESSION_QUOTA: u64 = 1024 * 1024 * 1024;
/// Nejvic bajtu cache objektu ve fsd (stare se zahodi).
pub const CACHE_MAX: u64 = 512 * 1024 * 1024;

struct Entry {
    vmo: Vmo,
    version: u64,
    size: u64,
    used: u64,
}

/// Cache objektu pameti souboru.
#[derive(Default)]
pub struct FileVmos {
    map: BTreeMap<(u64, u64), Entry>,
    bytes: u64,
    clock: u64,
}

impl FileVmos {
    /// Objekt pro soubor (dataset, oid) ve verzi `version` o velikosti
    /// `size`; `fill(vmo)` ho naplni, kdyz v cache neni nebo je stary.
    pub fn get(&mut self, key: (u64, u64), version: u64, size: u64, fill: impl FnOnce(&Vmo) -> Result<()>) -> Result<&Vmo> {
        self.clock += 1;
        let fresh = self.map.get(&key).is_some_and(|e| e.version == version && e.size == size);
        if !fresh {
            if let Some(old) = self.map.remove(&key) {
                self.bytes -= old.size;
            }
            let page = libos::vm::page_size();
            let vmo = Vmo::create(size.div_ceil(page) * page)?;
            fill(&vmo)?;
            self.bytes += size;
            self.map.insert(key, Entry { vmo, version, size, used: self.clock });
            self.evict(key);
        }
        let e = self.map.get_mut(&key).ok_or(Error::NotFound)?;
        e.used = self.clock;
        Ok(&e.vmo)
    }

    /// Zahodi nejstarsi objekty nad `CACHE_MAX` (krome `keep`).
    fn evict(&mut self, keep: (u64, u64)) {
        while self.bytes > CACHE_MAX {
            let Some((&k, _)) = self.map.iter().filter(|(k, _)| **k != keep).min_by_key(|(_, e)| e.used) else { return };
            if let Some(e) = self.map.remove(&k) {
                self.bytes -= e.size;
            }
        }
    }
}

/// Zapise `data` do VMO od `at` pres docasne mapovani (`len` = velikost
/// mapovani, cele stranky).
pub fn write_mapped(vmo: &Vmo, len: u64, mut fill: impl FnMut(&mut [u8]) -> Result<()>) -> Result<()> {
    if len == 0 {
        return Ok(());
    }
    let me = libos::rt::self_process();
    let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
    let addr = libos::sys::vm_map(me, vmo.handle().raw(), 0, len, flags, 0)?;
    // SAFETY: [addr, addr + len) je prave namapovany objekt pameti s pravem
    // zapisu, jinde ve fsd na nej nic neukazuje; po `fill` se odmapuje.
    let buf = unsafe { core::slice::from_raw_parts_mut(addr as *mut u8, len as usize) };
    let r = fill(buf);
    let _ = libos::sys::vm_unmap(me, addr, len);
    r
}
