//! Pametovy fd (`memfd_create`) nad `osrt::memfd`: vytvoreni, velikost,
//! mapovani `MAP_SHARED` primo na objekt pameti, pecete a predani handlu
//! objektu jinemu procesu (`<magnumopus/memfd.h>`).

use std::os::magnumopus::runtime as rt;

use rt::fd::{self, Description, Object};
use rt::memfd::Memory;

use super::R;
use crate::errno;

fn e(err: rt::Error) -> crate::c_int {
    errno::from_os(err as i32)
}

fn memory<T>(fd: i32, f: impl FnOnce(&Memory) -> R<T>) -> R<T> {
    let d = fd::get(fd).map_err(|_| errno::EBADF)?;
    match &d.object {
        Object::Memory(m) => f(m),
        _ => Err(errno::EINVAL),
    }
}

/// Je `fd` pametovy fd?
pub fn is_memfd(fd: i32) -> bool {
    fd::get(fd).is_ok_and(|d| matches!(d.object, Object::Memory(_)))
}

fn insert(m: Memory, cloexec: bool) -> R<i32> {
    let st = if m.writable() { super::status::RDWR } else { super::status::RDONLY };
    fd::insert(Description::new(Object::Memory(m), st), 0, cloexec).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        err => e(err),
    })
}

pub fn memfd_create(capacity: u64, cloexec: bool, sealing: bool) -> R<i32> {
    let m = Memory::create(capacity, sealing).map_err(|err| match err {
        rt::Error::InvalidArgs => errno::EINVAL,
        err => e(err),
    })?;
    insert(m, cloexec)
}

/// Velikost pametoveho fd (`fstat`).
pub fn memfd_size(fd: i32) -> R<u64> {
    memory(fd, |m| m.size().map_err(e))
}

pub fn memfd_truncate(fd: i32, len: u64) -> R<()> {
    memory(fd, |m| {
        m.set_len(len).map_err(|err| match err {
            // Pecet nebo jen pro cteni: jako Linux.
            rt::Error::AccessDenied => errno::EPERM,
            rt::Error::OutOfRange => errno::EFBIG,
            err => e(err),
        })
    })
}

/// `MAP_SHARED` nad pametovym fd: `addr` 0 = kam jadro chce.
pub fn memfd_map(fd: i32, off: u64, len: u64, prot: u32, addr: u64) -> R<u64> {
    memory(fd, |m| {
        m.map(off, len, prot, addr).map_err(|err| match err {
            rt::Error::AccessDenied => errno::EACCES,
            rt::Error::OutOfRange => errno::ENXIO,
            err => e(err),
        })
    })
}

/// `mmap` souboru pres objekt pameti z fsd: `MAP_PRIVATE` = klon s kopii
/// pri zapisu (nic se nekopiruje), sdilene jen pro cteni = primo objekt.
/// `ENOTSUP` = soubor jde jen kopii (prazdny, velky, rozsah za koncem).
pub fn file_map(fd: i32, off: u64, len: u64, prot: u32, addr: u64, private: bool) -> R<u64> {
    let d = rt::fd::get(fd).map_err(|_| errno::EBADF)?;
    let f = d.file().ok_or(errno::ENOTSUP)?;
    f.map(off, len, prot, addr, private).map_err(|err| match err {
        rt::Error::NotSupported | rt::Error::WrongType => errno::ENOTSUP,
        rt::Error::NoMemory => errno::ENOMEM,
        err => e(err),
    })
}

pub fn memfd_add_seals(fd: i32, seals: u32) -> R<()> {
    memory(fd, |m| {
        m.add_seals(seals).map_err(|err| match err {
            rt::Error::AccessDenied => errno::EPERM,
            rt::Error::BadState => errno::EBUSY,
            err => e(err),
        })
    })
}

pub fn memfd_seals(fd: i32) -> R<u32> {
    memory(fd, |m| Ok(m.seals()))
}

/// Handle objektu pameti za fd (novy, vlastni ho volajici): `readonly` =
/// bez prava zapisu a zmeny velikosti.
pub fn memfd_export(fd: i32, readonly: bool) -> R<u32> {
    memory(fd, |m| m.export_raw(readonly).map_err(e))
}

/// Pametovy fd z handlu objektu pameti (fd ho prevezme); `writable` urci
/// volajici (podle toho, co mu odesilatel poslal).
pub fn memfd_import(handle: u32, writable: bool, cloexec: bool) -> R<i32> {
    insert(Memory::import_raw(handle, writable), cloexec)
}

/// Handle predany rodicem ve startovni zprave se stitkem `tag` (jen
/// jednou).
pub fn start_handle(tag: u32) -> Option<u32> {
    rt::start::take_tagged(tag)
}
