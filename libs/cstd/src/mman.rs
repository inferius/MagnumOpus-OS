//! `sys/mman.h`: `mmap`, `munmap`, `mprotect`, `madvise`, `mseal` nad
//! spravou pameti (`libos::vm` pres osrt).
//!
//! - Anonymni pamet: stranky na pozadani (prvni zapis), `PROT_NONE` =
//!   jen rezervace adres; `mprotect` na rezervaci pamet doplni.
//! - `MAP_SHARED` anonymni = v ramci procesu totez co soukrome (`fork`
//!   neni).
//! - Soubor: obsah se nacte do anonymni pameti (`MAP_PRIVATE`, nebo
//!   `MAP_SHARED` jen pro cteni - zmeny souboru se pak neprojevi);
//!   `MAP_SHARED` se zapisem = `ENOTSUP`, dokud fsd neda objekt pameti
//!   souboru.
//! - `PROT_EXEC` na nezapecetene pameti jen s opravnenim JIT (W^X), jinak
//!   `EACCES`.

use core::ffi::c_void;

use crate::sys;
use crate::{c_int, errno, size_t};

pub const PROT_NONE: c_int = 0;
pub const PROT_READ: c_int = 1;
pub const PROT_WRITE: c_int = 2;
pub const PROT_EXEC: c_int = 4;
pub const MAP_SHARED: c_int = 0x01;
pub const MAP_PRIVATE: c_int = 0x02;
pub const MAP_FIXED: c_int = 0x10;
pub const MAP_ANONYMOUS: c_int = 0x20;
pub const MAP_NORESERVE: c_int = 0x4000;
pub const MAP_FIXED_NOREPLACE: c_int = 0x100000;
pub const MAP_FAILED: *mut c_void = usize::MAX as *mut c_void;
pub const MADV_NORMAL: c_int = 0;
pub const MADV_RANDOM: c_int = 1;
pub const MADV_SEQUENTIAL: c_int = 2;
pub const MADV_WILLNEED: c_int = 3;
pub const MADV_DONTNEED: c_int = 4;
pub const MADV_FREE: c_int = 8;

/// Pozadavek `mmap` po kontrole argumentu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub addr: Option<u64>,
    pub len: u64,
    pub prot: u32,
    /// Pevna adresa: `true` nahradit (`MAP_FIXED`), `false` jen pokud volno.
    pub replace: bool,
    pub file: Option<(c_int, u64)>,
    pub shared: bool,
}

/// Zkontroluje argumenty `mmap` (zarovnani, kombinace priznaku).
pub fn check(addr: u64, len: size_t, prot: c_int, flags: c_int, fd: c_int, off: i64, page: u64) -> Result<Request, c_int> {
    if len == 0 || prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return Err(errno::EINVAL);
    }
    let kind = flags & (MAP_SHARED | MAP_PRIVATE);
    if kind == 0 || kind == (MAP_SHARED | MAP_PRIVATE) {
        return Err(errno::EINVAL);
    }
    let len = (len as u64).checked_next_multiple_of(page).ok_or(errno::ENOMEM)?;
    let fixed = flags & (MAP_FIXED | MAP_FIXED_NOREPLACE) != 0;
    if fixed && (addr == 0 || addr % page != 0) {
        return Err(errno::EINVAL);
    }
    let file = if flags & MAP_ANONYMOUS != 0 {
        None
    } else {
        if off < 0 || off as u64 % page != 0 {
            return Err(errno::EINVAL);
        }
        if fd < 0 {
            return Err(errno::EBADF);
        }
        if kind == MAP_SHARED && prot & PROT_WRITE != 0 {
            return Err(errno::ENOTSUP);
        }
        Some((fd, off as u64))
    };
    Ok(Request {
        addr: fixed.then_some(addr),
        len,
        prot: prot as u32,
        replace: flags & MAP_FIXED != 0,
        file,
        shared: kind == MAP_SHARED,
    })
}

/// Anonymni pamet (nebo rezervace pro `PROT_NONE`) podle pozadavku.
fn anon(r: &Request, prot: u32) -> Result<u64, c_int> {
    match r.addr {
        Some(a) => {
            if r.replace {
                // MAP_FIXED nahrazuje, co tam bylo (chyba = nebylo nic).
                let _ = sys::vm_release(a, r.len);
            }
            sys::vm_reserve_at(a, r.len)?;
            if prot != 0
                && let Err(e) = sys::vm_commit(a, r.len, prot)
            {
                let _ = sys::vm_release(a, r.len);
                return Err(e);
            }
            Ok(a)
        }
        None if prot == 0 => sys::vm_reserve(r.len),
        None => sys::vm_alloc(r.len, prot),
    }
}

fn map(r: Request) -> Result<u64, c_int> {
    let Some((fd, off)) = r.file else { return anon(&r, r.prot) };
    // Soubor: nacist do pameti s pravem zapisu, pak nastavit prava.
    let st = sys::status(fd)?;
    if st & sys::status::ACCMODE == sys::status::WRONLY {
        return Err(errno::EACCES);
    }
    let base = anon(&r, (PROT_READ | PROT_WRITE) as u32)?;
    // SAFETY: prave namapovano `r.len` bajtu s pravem zapisu.
    let buf = unsafe { core::slice::from_raw_parts_mut(base as *mut u8, r.len as usize) };
    let mut done = 0usize;
    while done < buf.len() {
        match sys::pread(fd, &mut buf[done..], off + done as u64) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) => {
                let _ = sys::vm_release(base, r.len);
                return Err(if e == errno::ESPIPE { errno::ENODEV } else { e });
            }
        }
    }
    if r.prot != (PROT_READ | PROT_WRITE) as u32
        && let Err(e) = sys::vm_protect(base, r.len, r.prot)
    {
        let _ = sys::vm_release(base, r.len);
        return Err(e);
    }
    Ok(base)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mmap(addr: *mut c_void, len: size_t, prot: c_int, flags: c_int, fd: c_int, off: i64) -> *mut c_void {
    let r = check(addr as u64, len, prot, flags, fd, off, sys::page_size()).and_then(map);
    match r {
        Ok(a) => a as *mut c_void,
        Err(e) => {
            errno::set(e);
            MAP_FAILED
        }
    }
}

fn range(addr: *mut c_void, len: size_t) -> Result<(u64, u64), c_int> {
    let page = sys::page_size();
    let a = addr as u64;
    if a % page != 0 || len == 0 {
        return Err(errno::EINVAL);
    }
    Ok((a, (len as u64).checked_next_multiple_of(page).ok_or(errno::EINVAL)?))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn munmap(addr: *mut c_void, len: size_t) -> c_int {
    errno::ret(range(addr, len).and_then(|(a, l)| sys::vm_release(a, l)))
}

/// `mprotect`: cast rozsahu, ktera je jen rezervace (`PROT_NONE` z
/// `mmap`), dostane pamet; zbytek zmeni prava.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mprotect(addr: *mut c_void, len: size_t, prot: c_int) -> c_int {
    if prot & !(PROT_READ | PROT_WRITE | PROT_EXEC) != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    let r = range(addr, len).and_then(|(a, l)| {
        let end = a + l;
        let mut at = a;
        while at < end {
            let reg = sys::vm_query(at)?;
            let reg_end = (reg.start + reg.len).min(end);
            let piece = reg_end.saturating_sub(at);
            if piece == 0 {
                return Err(errno::ENOMEM);
            }
            match reg.kind {
                1 if prot != 0 => sys::vm_commit(at, piece, prot as u32)?,
                1 => {}
                2 => sys::vm_protect(at, piece, prot as u32)?,
                _ => return Err(errno::ENOMEM),
            }
            at = reg_end;
        }
        Ok(())
    });
    errno::ret(r)
}

/// `madvise`: `MADV_DONTNEED`/`MADV_FREE` vrati pamet (dalsi cteni = nuly),
/// rady bez ucinku se prijmou.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn madvise(addr: *mut c_void, len: size_t, advice: c_int) -> c_int {
    let r = range(addr, len).and_then(|(a, l)| match advice {
        MADV_NORMAL | MADV_RANDOM | MADV_SEQUENTIAL | MADV_WILLNEED => Ok(()),
        MADV_DONTNEED | MADV_FREE => {
            let reg = sys::vm_query(a)?;
            if reg.kind != 2 {
                return Ok(());
            }
            // Pamet zpet a hned nova (nulova) se stejnymi pravy.
            sys::vm_decommit(a, l)?;
            if reg.prot != 0 { sys::vm_commit(a, l, reg.prot) } else { Ok(()) }
        }
        _ => Err(errno::EINVAL),
    });
    errno::ret(r)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_madvise(addr: *mut c_void, len: size_t, advice: c_int) -> c_int {
    // Vraci cislo chyby primo (POSIX).
    // SAFETY: kontrakt C.
    if unsafe { madvise(addr, len, advice) } == 0 { 0 } else { errno::get() }
}

/// `mseal` (Linux 6.10): zapecetí rozsah se soucasnymi pravy.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mseal(addr: *mut c_void, len: size_t, flags: u64) -> c_int {
    if flags != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    let r = range(addr, len).and_then(|(a, l)| {
        let reg = sys::vm_query(a)?;
        sys::vm_seal(a, l, reg.prot)
    });
    errno::ret(r)
}

/// `msync`: sdilene mapovani souboru neni, neni co zapsat.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn msync(addr: *mut c_void, len: size_t, _flags: c_int) -> c_int {
    errno::ret(range(addr, len).map(|_| ()))
}

/// `mlock`/`munlock`: pamet se neodklada na disk, uspech.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mlock(_addr: *const c_void, _len: size_t) -> c_int {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn munlock(_addr: *const c_void, _len: size_t) -> c_int {
    0
}
