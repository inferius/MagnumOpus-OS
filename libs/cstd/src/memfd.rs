//! Pametovy fd (`memfd_create`, `sys/mman.h`) a predani mezi procesy
//! (`<magnumopus/memfd.h>`), docs/cs/cstd.md kapitola Pametovy fd.
//!
//! Pametovy fd je zvetsitelny objekt pameti jadra (`VMO_RESIZABLE`) s
//! kapacitou [`osrt::memfd::DEFAULT_CAPACITY`] (64 MiB; vetsi pres
//! `mo_memfd_create`). `ftruncate` meni velikost v jadre (vidi ji i jine
//! procesy), `mmap(MAP_SHARED)` mapuje primo objekt (sdilene ramce mezi
//! procesy), pristup za konec = `SIGBUS`. Pecete `fcntl(F_ADD_SEALS)`.
//! Predani: handle objektu (`mo_memfd_export`) posle program jinemu procesu
//! vlastnim kanalem, prijemce si z nej udela fd (`mo_memfd_import`);
//! `SCM_RIGHTS` neni.

use crate::{c_char, c_int, errno, sys};

pub const MFD_CLOEXEC: u32 = 1;
pub const MFD_ALLOW_SEALING: u32 = 2;
pub const MFD_HUGETLB: u32 = 4;

pub const F_ADD_SEALS: c_int = 1033;
pub const F_GET_SEALS: c_int = 1034;

/// Kapacita bez volby (`osrt::memfd::DEFAULT_CAPACITY`).
pub const DEFAULT_CAPACITY: u64 = 64 << 20;

fn create(flags: u32, capacity: u64) -> c_int {
    if flags & !(MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_HUGETLB) != 0 || capacity == 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    errno::ret_val(sys::memfd_create(capacity, flags & MFD_CLOEXEC != 0, flags & MFD_ALLOW_SEALING != 0)) as c_int
}

/// `memfd_create`: jmeno slouzi jen k ladeni (neuklada se). `MFD_HUGETLB`
/// se prijme bez ucinku (stranky 4 KiB).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn memfd_create(name: *const c_char, flags: u32) -> c_int {
    if name.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    create(flags, DEFAULT_CAPACITY)
}

/// `mo_memfd_create`: jako `memfd_create` s vlastni kapacitou (nejvetsi
/// velikost; jadro za ni plati 8 B na stranku 4 KiB).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_memfd_create(name: *const c_char, flags: u32, capacity: u64) -> c_int {
    if name.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    create(flags, capacity)
}

/// `mo_memfd_export`: novy handle objektu pameti za pametovym fd (posle
/// se jinemu procesu kanalem; vlastni ho volajici). `readonly` = prijemce
/// nesmi zapisovat ani menit velikost. Vraci 0, nebo -1 s `errno`.
///
/// # Safety
///
/// `out` je platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mo_memfd_export(fd: c_int, readonly: c_int, out: *mut u32) -> c_int {
    if out.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    match sys::memfd_export(fd, readonly != 0) {
        Ok(h) => {
            // SAFETY: kontrakt C.
            unsafe { *out = h };
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

/// `mo_memfd_import`: pametovy fd z handlu objektu pameti (fd ho
/// prevezme). `flags`: `O_RDONLY` / `O_RDWR`, `O_CLOEXEC`; zapis dovoli az
/// pravo handlu (handle jen pro cteni zapis odmitne v jadre).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_memfd_import(handle: u32, flags: c_int) -> c_int {
    let writable = flags & crate::unistd::O_ACCMODE == crate::unistd::O_RDWR;
    let cloexec = flags & crate::unistd::O_CLOEXEC != 0;
    errno::ret_val(sys::memfd_import(handle, writable, cloexec)) as c_int
}

/// `mo_start_handle`: handle, ktery rodic predal ve startovni zprave se
/// stitkem `tag` (jen jednou; 0 = neni).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_start_handle(tag: u32) -> u32 {
    sys::start_handle(tag).unwrap_or(0)
}

/// Prikazy `fcntl` pro pecete; `None` = jiny prikaz.
pub fn fcntl(fd: c_int, cmd: c_int, arg: c_int) -> Option<c_int> {
    match cmd {
        F_ADD_SEALS => Some(errno::ret(sys::memfd_add_seals(fd, arg as u32))),
        F_GET_SEALS => Some(errno::ret_val(sys::memfd_seals(fd)) as c_int),
        _ => None,
    }
}
