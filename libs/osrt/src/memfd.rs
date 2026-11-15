//! Pametovy fd (C `memfd_create`): zvetsitelny objekt pameti
//! (`libos::Vmo::create_resizable`, jadro `VMO_SET_SIZE`) s pozici pro
//! `read`/`write`/`lseek`. Mapovani `MAP_SHARED` sdili ramce objektu mezi
//! procesy (ne kopie). Predani jinemu procesu = handle objektu v kanalu
//! ([`Memory::export`] / [`Memory::import`]); jen pro cteni = handle bez
//! prava `WRITE` a `RESIZE`.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use abi::object::Rights;
use libos::{Handle, Vmo};

use crate::fs::SeekFrom;
use crate::{Error, Result};

/// Vychozi kapacita (nejvetsi velikost) pametoveho fd: jadro za ni plati
/// 8 B seznamu stranek na stranku kapacity (64 MiB = 128 KiB), pamet jen
/// za aktualni velikost.
pub const DEFAULT_CAPACITY: u64 = 64 << 20;
/// Nejvic bajtu jednoho `VMO_READ`/`VMO_WRITE` (limit jadra).
const IO_MAX: usize = 1 << 20;

/// Pecete (`fcntl(F_ADD_SEALS)`, hodnoty jako Linux).
pub mod seal {
    pub const SEAL: u32 = 1;
    pub const SHRINK: u32 = 2;
    pub const GROW: u32 = 4;
    pub const WRITE: u32 = 8;
}

pub struct Memory {
    vmo: Vmo,
    pos: AtomicU64,
    writable: bool,
    seals: AtomicU32,
}

impl Memory {
    /// Novy prazdny pametovy fd s kapacitou `capacity`. Bez `sealing`
    /// (`MFD_ALLOW_SEALING`) je od zacatku pecet `SEAL` (dalsi nejdou).
    pub fn create(capacity: u64, sealing: bool) -> Result<Memory> {
        let vmo = Vmo::create_resizable(0, capacity)?;
        let seals = if sealing { 0 } else { seal::SEAL };
        Ok(Memory { vmo, pos: AtomicU64::new(0), writable: true, seals: AtomicU32::new(seals) })
    }

    /// Prevezme handle objektu pameti od jineho procesu. Bez prava
    /// `WRITE` je jen pro cteni (zapis, zmena velikosti a mapovani pro
    /// zapis selzou).
    pub fn import(h: Handle, writable: bool) -> Memory {
        Memory { vmo: Vmo::from_handle(h), pos: AtomicU64::new(0), writable, seals: AtomicU32::new(seal::SEAL) }
    }

    /// Pametovy fd zdedeny od rodice ([`crate::inherit`]): pecete
    /// a pozice jako u rodice (potomek ma vlastni kopii pozice).
    pub(crate) fn inherited(h: Handle, writable: bool, seals: u32, pos: u64) -> Memory {
        Memory { vmo: Vmo::from_handle(h), pos: AtomicU64::new(pos), writable, seals: AtomicU32::new(seals) }
    }

    /// Aktualni pozice.
    pub fn tell(&self) -> u64 {
        self.pos.load(Ordering::Acquire)
    }

    /// Jako [`Memory::import`] se syrovym cislem handlu (C, cstd).
    /// Prava handlu jadro zjistit neumi: `writable` rika volajici; bez
    /// prava `WRITE` jadro zapis i mapovani pro zapis stejne odmitne.
    pub fn import_raw(raw: u32, writable: bool) -> Memory {
        Self::import(Handle::from_raw(raw), writable)
    }

    /// Syrove cislo noveho handlu pro predani (viz [`Memory::export`]).
    pub fn export_raw(&self, readonly: bool) -> Result<u32> {
        self.export(readonly).map(Handle::into_raw)
    }

    /// Handle objektu pro predani jinemu procesu: `readonly` = bez prava
    /// zapisu a zmeny velikosti.
    pub fn export(&self, readonly: bool) -> Result<Handle> {
        let rights = if readonly || !self.writable {
            Rights::BASIC | Rights::READ | Rights::MAP
        } else {
            Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP | Rights::RESIZE
        };
        self.vmo.handle().duplicate(Some(rights))
    }

    pub fn vmo(&self) -> &Vmo {
        &self.vmo
    }

    pub fn writable(&self) -> bool {
        self.writable
    }

    pub fn size(&self) -> Result<u64> {
        self.vmo.size()
    }

    pub fn seals(&self) -> u32 {
        self.seals.load(Ordering::Acquire)
    }

    /// Prida pecete (`F_ADD_SEALS`). Zapecetene `SEAL` = `AccessDenied`
    /// (C `EPERM`). `WRITE` zapeceti objekt v jadre (obsah se uz nezmeni;
    /// mapovani pro zapis musi byt pryc, jinak `BadState`, C `EBUSY`).
    pub fn add_seals(&self, add: u32) -> Result<()> {
        if add & !(seal::SEAL | seal::SHRINK | seal::GROW | seal::WRITE) != 0 {
            return Err(Error::InvalidArgs);
        }
        if self.seals() & seal::SEAL != 0 {
            return Err(Error::AccessDenied);
        }
        if add & seal::WRITE != 0 {
            self.vmo.seal()?;
        }
        self.seals.fetch_or(add, Ordering::AcqRel);
        Ok(())
    }

    /// Nova velikost (`ftruncate`); pecete `SHRINK`/`GROW` ji omezi.
    pub fn set_len(&self, len: u64) -> Result<()> {
        if !self.writable {
            return Err(Error::AccessDenied);
        }
        let cur = self.size()?;
        let s = self.seals();
        if (len < cur && s & seal::SHRINK != 0) || (len > cur && s & seal::GROW != 0) {
            return Err(Error::AccessDenied);
        }
        self.vmo.set_size(len)
    }

    pub fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize> {
        let size = self.size()?;
        if off >= size {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(size - off) as usize;
        for (i, chunk) in buf[..n].chunks_mut(IO_MAX).enumerate() {
            self.vmo.read(off + (i * IO_MAX) as u64, chunk)?;
        }
        Ok(n)
    }

    /// Zapis na pozici; za koncem objekt zvetsi (jako soubor).
    pub fn write_at(&self, off: u64, data: &[u8]) -> Result<usize> {
        if !self.writable || self.seals() & seal::WRITE != 0 {
            return Err(Error::AccessDenied);
        }
        let end = off.checked_add(data.len() as u64).ok_or(Error::OutOfRange)?;
        if end > self.size()? {
            self.set_len(end)?;
        }
        for (i, chunk) in data.chunks(IO_MAX).enumerate() {
            self.vmo.write(off + (i * IO_MAX) as u64, chunk)?;
        }
        Ok(data.len())
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let n = self.read_at(self.pos.load(Ordering::Acquire), buf)?;
        self.pos.fetch_add(n as u64, Ordering::AcqRel);
        Ok(n)
    }

    pub fn write(&self, data: &[u8]) -> Result<usize> {
        let n = self.write_at(self.pos.load(Ordering::Acquire), data)?;
        self.pos.fetch_add(n as u64, Ordering::AcqRel);
        Ok(n)
    }

    pub fn seek(&self, pos: SeekFrom) -> Result<u64> {
        let base = match pos {
            SeekFrom::Start(o) => {
                self.pos.store(o, Ordering::Release);
                return Ok(o);
            }
            SeekFrom::Current(d) => (self.pos.load(Ordering::Acquire), d),
            SeekFrom::End(d) => (self.size()?, d),
        };
        let new = base.0.checked_add_signed(base.1).ok_or(Error::InvalidArgs)?;
        self.pos.store(new, Ordering::Release);
        Ok(new)
    }

    /// Namapuje `len` bajtu od `off` do vlastniho procesu: `addr` 0 = kam
    /// jadro chce, jinak presne tam (volajici adresy predtim uvolnil).
    /// `flags` = `abi::vm::MAP_*`. Muze pokryt celou kapacitu; pristup za
    /// aktualni velikost je vyjimka (C `SIGBUS`).
    pub fn map(&self, off: u64, len: u64, flags: u32, addr: u64) -> Result<u64> {
        if flags & abi::vm::MAP_WRITE != 0 && (!self.writable || self.seals() & seal::WRITE != 0) {
            return Err(Error::AccessDenied);
        }
        libos::sys::vm_map(libos::rt::self_process(), self.vmo.handle().raw(), off, len, flags, addr)
    }
}
