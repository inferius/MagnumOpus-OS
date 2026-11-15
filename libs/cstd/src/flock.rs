//! Zamky souboru: `fcntl(F_GETLK/F_SETLK/F_SETLKW)` a `flock`.
//!
//! Zamky drzi fsd (zprava `POSIX_LOCK`, rozhodnuti 8B): vlastnik `fcntl`
//! je proces (relace FS), `flock` otevreny soubor (handle). Uvolni je
//! zavreni posledniho fd procesu na souboru (`flock`: zavreni handlu)
//! a konec procesu. `F_SETLKW` a `flock` bez `LOCK_NB` cekaji opakovanym
//! zkousenim (blokujici dotaz by drzel kanal relace a zastavil ostatni
//! souborove operace procesu); fsd pritom hlida cyklus cekani (`EDEADLK`).

use core::time::Duration;

use crate::sys::{self, LockResult, Seek};
use crate::unistd::{F_GETLK, F_SETLKW};
use crate::{c_int, errno};

/// `struct flock` (jako Linux).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct flock {
    pub l_type: i16,
    pub l_whence: i16,
    pub l_start: i64,
    pub l_len: i64,
    pub l_pid: i32,
}

pub const F_RDLCK: i16 = 0;
pub const F_WRLCK: i16 = 1;
pub const F_UNLCK: i16 = 2;
pub const LOCK_SH: c_int = 1;
pub const LOCK_EX: c_int = 2;
pub const LOCK_NB: c_int = 4;
pub const LOCK_UN: c_int = 8;

/// Prikazy a tridy zpravy `POSIX_LOCK` (`fsproto::v2::posix_lock`).
mod wire {
    pub const TEST: u8 = 0;
    pub const SET: u8 = 1;
    pub const SET_WAIT: u8 = 2;
    pub const UNLOCK: u8 = 3;
    pub const FCNTL: u8 = 0;
    pub const FLOCK: u8 = 1;
}

/// Nejdelsi krok cekani mezi pokusy (zacina na milisekunde, zdvojnasobuje se).
const MAX_STEP: Duration = Duration::from_millis(50);

/// Absolutni rozsah z `struct flock`: (start, delka; 0 = do konce).
/// `cur` = aktualni pozice, `size` = velikost souboru (pro `SEEK_END`).
pub fn abs_range(l: &flock, cur: u64, size: u64) -> Result<(u64, u64), c_int> {
    let base: i128 = match l.l_whence as c_int {
        0 => 0,
        1 => i128::from(cur),
        2 => i128::from(size),
        _ => return Err(errno::EINVAL),
    };
    let mut start = base + i128::from(l.l_start);
    let mut len = i128::from(l.l_len);
    // Zaporna delka = rozsah pred startem (POSIX).
    if len < 0 {
        start += len;
        len = -len;
    }
    if start < 0 {
        return Err(errno::EINVAL);
    }
    Ok((start as u64, u64::try_from(len).map_err(|_| errno::EOVERFLOW)?))
}

/// Zamkne s cekanim (opakovane `SET_WAIT`); cyklus = `EDEADLK`.
fn lock_wait(fd: c_int, class: u8, write: bool, start: u64, len: u64) -> Result<(), c_int> {
    let mut step = Duration::from_millis(1);
    loop {
        match sys::file_lock(fd, wire::SET_WAIT, class, write, start, len)? {
            LockResult::Done => return Ok(()),
            LockResult::Deadlock => return Err(errno::EDEADLK),
            LockResult::Conflict { .. } => {
                crate::pthread::testcancel();
                std::thread::sleep(step);
                step = (step * 2).min(MAX_STEP);
            }
        }
    }
}

/// Zamky z `fcntl`.
///
/// # Safety
///
/// `p` ukazuje na `struct flock` (nebo NULL).
pub unsafe fn fcntl_lock(fd: c_int, cmd: c_int, p: *mut flock) -> c_int {
    // SAFETY: kontrakt C.
    let Some(l) = (unsafe { p.as_mut() }) else { return errno::ret(Err(errno::EFAULT)) };
    let r = (|| -> Result<(), c_int> {
        let st = sys::status(fd)?;
        let cur = sys::seek(fd, Seek::Cur(0)).unwrap_or(0);
        let size = if l.l_whence == 2 { sys::fstat(fd)?.size } else { 0 };
        let (start, len) = abs_range(l, cur, size)?;
        let write = match l.l_type {
            F_RDLCK => false,
            F_WRLCK => true,
            F_UNLCK => {
                if cmd == F_GETLK {
                    return Err(errno::EINVAL);
                }
                return sys::file_lock(fd, wire::UNLOCK, wire::FCNTL, false, start, len).map(|_| ());
            }
            _ => return Err(errno::EINVAL),
        };
        // Zamek pro cteni potrebuje fd pro cteni, pro zapis fd pro zapis.
        let acc = st & sys::status::ACCMODE;
        if cmd != F_GETLK
            && ((write && acc == sys::status::RDONLY) || (!write && acc == sys::status::WRONLY))
        {
            return Err(errno::EBADF);
        }
        if cmd == F_GETLK {
            match sys::file_lock(fd, wire::TEST, wire::FCNTL, write, start, len)? {
                LockResult::Conflict { write, start, len, pid } => {
                    l.l_type = if write { F_WRLCK } else { F_RDLCK };
                    l.l_whence = 0;
                    l.l_start = start as i64;
                    l.l_len = len as i64;
                    // Cislo procesu drzitele (jeho getpid); nezname = -1.
                    l.l_pid = if pid > 0 { pid } else { -1 };
                }
                _ => l.l_type = F_UNLCK,
            }
            return Ok(());
        }
        if cmd == F_SETLKW {
            return lock_wait(fd, wire::FCNTL, write, start, len);
        }
        match sys::file_lock(fd, wire::SET, wire::FCNTL, write, start, len)? {
            LockResult::Done => Ok(()),
            LockResult::Conflict { .. } => Err(errno::EAGAIN),
            LockResult::Deadlock => Err(errno::EDEADLK),
        }
    })();
    errno::ret(r)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn flock(fd: c_int, op: c_int) -> c_int {
    let nb = op & LOCK_NB != 0;
    let r = match op & !LOCK_NB {
        LOCK_UN => sys::file_lock(fd, wire::UNLOCK, wire::FLOCK, false, 0, 0).map(|_| ()),
        k @ (LOCK_SH | LOCK_EX) => {
            let write = k == LOCK_EX;
            if nb {
                match sys::file_lock(fd, wire::SET, wire::FLOCK, write, 0, 0) {
                    Ok(LockResult::Done) => Ok(()),
                    Ok(LockResult::Conflict { .. }) => Err(errno::EAGAIN),
                    Ok(LockResult::Deadlock) => Err(errno::EDEADLK),
                    Err(e) => Err(e),
                }
            } else {
                lock_wait(fd, wire::FLOCK, write, 0, 0)
            }
        }
        _ => Err(errno::EINVAL),
    };
    errno::ret(r)
}

/// `lockf` (zamek od aktualni pozice, `F_LOCK`/`F_TLOCK`/`F_ULOCK`/`F_TEST`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn lockf(fd: c_int, cmd: c_int, len: i64) -> c_int {
    let mut l = flock { l_type: F_WRLCK, l_whence: 1, l_start: 0, l_len: len, l_pid: 0 };
    let c = match cmd {
        0 => {
            l.l_type = F_UNLCK;
            crate::unistd::F_SETLK
        }
        1 => F_SETLKW,
        2 => crate::unistd::F_SETLK,
        3 => {
            // SAFETY: platny ukazatel.
            if unsafe { fcntl_lock(fd, F_GETLK, &mut l) } != 0 {
                return -1;
            }
            return if l.l_type == F_UNLCK { 0 } else { errno::ret(Err(errno::EACCES)) };
        }
        _ => return errno::ret(Err(errno::EINVAL)),
    };
    // SAFETY: platny ukazatel.
    unsafe { fcntl_lock(fd, c, &mut l) }
}
