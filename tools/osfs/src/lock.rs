//! Zamek souboru s obrazem: nejvys jeden zapisovatel.
//!
//! Dva procesy, ktere by obraz meli otevreny pro zapis zaroven (`osfs mount`
//! a `osfs put`, nebo dva `mount` na ruzna pismena), by si navzajem
//! prepisovaly bloky a superbloky - svazek by se poskodil. Knihovna `fs`
//! zarizeni nezamyka, proto zamek drzi `osfs`:
//!
//! - zapis (`mount`, `put`) = vylucny zamek,
//! - cteni (`check`, `ls`, `cat`) = sdileny zamek; kdyz ho nejde ziskat,
//!   obraz prave nekdo zapisuje a cteni jen upozorni (vysledek muze byt
//!   o par sekund stary).
//!
//! Windows: `LockFileEx` na jednom bajtu daleko za koncem souboru, na
//! vlastnim handlu. Zamky ve Windows jsou povinne pro prekryvajici se
//! rozsah, ale tento rozsah nikdo necte ani nezapisuje, takze neblokuje
//! vstup/vystup `FileDevice` (ani v jinem procesu). Zamek plati napric
//! prihlasenimi a uvolni se i pri padu procesu.
//! Jinde: `File::try_lock` (poradni zamek).

use std::fs::File;
use std::path::Path;

/// Drzeny zamek; uvolni se pri zahozeni (nebo skonceni procesu).
pub struct ImageLock {
    _file: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockMode {
    Exclusive,
    Shared,
}

/// Zkusi ziskat zamek bez cekani. `Ok(None)` = drzi ho jiny proces.
pub fn try_lock(path: &Path, mode: LockMode) -> std::io::Result<Option<ImageLock>> {
    let file = File::open(path)?;
    if sys::try_lock(&file, mode)? {
        Ok(Some(ImageLock { _file: file }))
    } else {
        Ok(None)
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    use super::LockMode;

    #[repr(C)]
    struct Overlapped {
        internal: usize,
        internal_high: usize,
        offset: u32,
        offset_high: u32,
        event: *mut c_void,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LockFileEx(
            file: *mut c_void,
            flags: u32,
            reserved: u32,
            len_low: u32,
            len_high: u32,
            ov: *mut Overlapped,
        ) -> i32;
    }

    const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x1;
    const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x2;
    const ERROR_LOCK_VIOLATION: i32 = 33;
    const ERROR_IO_PENDING: i32 = 997;

    pub fn try_lock(file: &File, mode: LockMode) -> std::io::Result<bool> {
        // Bajt na 2^63 - 16: daleko za kazdym obrazem.
        let mut ov = Overlapped {
            internal: 0,
            internal_high: 0,
            offset: 0xFFFF_FFF0,
            offset_high: 0x7FFF_FFFF,
            event: std::ptr::null_mut(),
        };
        let mut flags = LOCKFILE_FAIL_IMMEDIATELY;
        if mode == LockMode::Exclusive {
            flags |= LOCKFILE_EXCLUSIVE_LOCK;
        }
        // SAFETY: platny handle souboru a struktura OVERLAPPED; soubor je
        // otevreny synchronne, volani se vrati hned.
        let ok = unsafe { LockFileEx(file.as_raw_handle(), flags, 0, 1, 0, &mut ov) };
        if ok != 0 {
            return Ok(true);
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(ERROR_LOCK_VIOLATION) | Some(ERROR_IO_PENDING) => Ok(false),
            _ => Err(err),
        }
    }
}

#[cfg(not(windows))]
mod sys {
    use std::fs::{File, TryLockError};

    use super::LockMode;

    pub fn try_lock(file: &File, mode: LockMode) -> std::io::Result<bool> {
        let r = match mode {
            LockMode::Exclusive => file.try_lock(),
            LockMode::Shared => file.try_lock_shared(),
        };
        match r {
            Ok(()) => Ok(true),
            Err(TryLockError::WouldBlock) => Ok(false),
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}
