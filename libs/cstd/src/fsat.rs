//! Funkce `*at` (relativne k otevrenemu adresari), `realpath`, `pathconf`
//! a `sendfile` (`std::filesystem` z libc++, docs/cs/cstd.md).
//!
//! FS (fsd) pracuje s cestami: `openat(dir, "x")` slozi cestu adresare
//! zapamatovanou pri otevreni (`osrt::fs::File::path`) se jmenem. Proto
//! nechrani pred zavodem "adresar se mezitim prejmenoval" jako na Linuxu
//! (tam je `*at` vazane na inode adresare).

use crate::stat::{fill, stat};
use crate::unistd::AT_FDCWD;
use crate::{c_bytes, c_char, c_int, errno, size_t, sys};

/// `unlinkat`: odebrat adresar (`rmdir`).
pub const AT_REMOVEDIR: c_int = 0x200;
/// `fstatat`: neprochazet symlink (FS symlinky zatim nevytvari).
pub const AT_SYMLINK_NOFOLLOW: c_int = 0x100;

/// Nejdelsi cesta vcetne nuly (`limits.h`).
pub const PATH_MAX: usize = 4096;
/// Nejdelsi jmeno polozky, ktere hlasime.
pub const NAME_MAX: usize = 255;

/// Jmena `pathconf` (hodnoty jako Linux).
pub mod pc {
    use crate::c_int;
    pub const LINK_MAX: c_int = 0;
    pub const NAME_MAX: c_int = 3;
    pub const PATH_MAX: c_int = 4;
    pub const PIPE_BUF: c_int = 5;
}

/// Cesta pro `*at`: absolutni nebo `AT_FDCWD` beze zmeny, jinak cesta
/// adresare `dir_path` + `/` + `path`.
pub fn join_at(dir_path: Option<&[u8]>, path: &[u8]) -> Result<Vec<u8>, c_int> {
    if path.is_empty() {
        return Err(errno::ENOENT);
    }
    match dir_path {
        Some(d) if path[0] != b'/' => {
            let mut out = d.to_vec();
            if out.last() != Some(&b'/') {
                out.push(b'/');
            }
            out.extend_from_slice(path);
            Ok(out)
        }
        _ => Ok(path.to_vec()),
    }
}

/// Cesta pro `*at` z C argumentu.
///
/// # Safety
///
/// `path` je retezec C nebo NULL.
pub(crate) unsafe fn at_path(dir: c_int, path: *const c_char) -> Result<Vec<u8>, c_int> {
    if path.is_null() {
        return Err(errno::EFAULT);
    }
    // SAFETY: retezec C.
    let p = unsafe { c_bytes(path) };
    if dir == AT_FDCWD || p.first() == Some(&b'/') {
        return join_at(None, p);
    }
    let d = sys::fd_path(dir)?;
    join_at(Some(&d), p)
}

/// Retezec C (s nulou) z cesty.
fn c_path(p: Vec<u8>) -> Vec<u8> {
    let mut p = p;
    p.push(0);
    p
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn openat(dir: c_int, path: *const c_char, flags: c_int, mut mode: ...) -> c_int {
    // SAFETY: s O_CREAT predava volajici treti argument (mode_t).
    let m = (flags & crate::unistd::O_CREAT != 0).then(|| unsafe { mode.next_arg::<u32>() });
    // SAFETY: kontrakt C.
    match unsafe { at_path(dir, path) } {
        // SAFETY: retezec zakonceny nulou zije po dobu volani.
        Ok(p) => unsafe { crate::unistd::open_mode(c_path(p).as_ptr(), flags, m) },
        Err(e) => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn unlinkat(dir: c_int, path: *const c_char, flags: c_int) -> c_int {
    if flags & !AT_REMOVEDIR != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    let r = unsafe { at_path(dir, path) }.and_then(|p| if flags & AT_REMOVEDIR != 0 { sys::rmdir(&p) } else { sys::unlink(&p) });
    errno::ret(r)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mkdirat(dir: c_int, path: *const c_char, mode: u32) -> c_int {
    // SAFETY: kontrakt C.
    errno::ret(unsafe { at_path(dir, path) }.and_then(|p| sys::mkdir_mode(&p, crate::stat::masked(mode))))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fstatat(dir: c_int, path: *const c_char, out: *mut stat, flags: c_int) -> c_int {
    if flags & !AT_SYMLINK_NOFOLLOW != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    let p = unsafe { at_path(dir, path) };
    fill(p.and_then(|p| if flags & AT_SYMLINK_NOFOLLOW != 0 { sys::lstat(&p) } else { sys::stat(&p) }), out)
}

/// `fchmodat`: jako `chmod` jen overi existenci (prava jsou granty FS).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fchmodat(dir: c_int, path: *const c_char, mode: u32, flags: c_int) -> c_int {
    if flags & !AT_SYMLINK_NOFOLLOW != 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    errno::ret(unsafe { at_path(dir, path) }.and_then(|p| sys::chmod(&p, (mode & 0o777) as u16)))
}

/// `realpath`: kanonicka absolutni cesta k existujici polozce (odkazy,
/// `.` a `..` vyresi fsd ve jmennem prostoru relace).
/// `resolved` NULL = alokovat (`free`), jinak buffer `PATH_MAX`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn realpath(path: *const c_char, resolved: *mut c_char) -> *mut c_char {
    if path.is_null() {
        errno::set(errno::EINVAL);
        return core::ptr::null_mut();
    }
    // SAFETY: retezec C.
    let r = sys::realpath(unsafe { c_bytes(path) });
    let p = match r {
        Ok(p) if p.len() < PATH_MAX => p,
        Ok(_) => {
            errno::set(errno::ENAMETOOLONG);
            return core::ptr::null_mut();
        }
        Err(e) => {
            errno::set(e);
            return core::ptr::null_mut();
        }
    };
    let out = if resolved.is_null() {
        // SAFETY: alokace pro volajiciho (uvolni free).
        let m = unsafe { cstd_core::stdlib::malloc(p.len() + 1) };
        if m.is_null() {
            errno::set(errno::ENOMEM);
            return core::ptr::null_mut();
        }
        m
    } else {
        resolved
    };
    // SAFETY: `out` ma aspon PATH_MAX (kontrakt C) nebo p.len() + 1 bajtu.
    unsafe {
        core::ptr::copy_nonoverlapping(p.as_ptr(), out, p.len());
        *out.add(p.len()) = 0;
    }
    out
}

/// Hodnota `pathconf` (stejna pro vsechny cesty jednoho FS).
pub fn conf(name: c_int) -> Result<i64, c_int> {
    match name {
        pc::PATH_MAX => Ok(PATH_MAX as i64),
        pc::NAME_MAX => Ok(NAME_MAX as i64),
        pc::LINK_MAX => Ok(1),
        pc::PIPE_BUF => Ok(4096),
        _ => Err(errno::EINVAL),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pathconf(path: *const c_char, name: c_int) -> i64 {
    if path.is_null() {
        return errno::ret_val::<i64>(Err(errno::EFAULT)) as i64;
    }
    // SAFETY: retezec C.
    let r = sys::stat(unsafe { c_bytes(path) }).and_then(|_| conf(name));
    errno::ret_val(r) as i64
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fpathconf(fd: c_int, name: c_int) -> i64 {
    errno::ret_val(sys::fstat(fd).and_then(|_| conf(name))) as i64
}

/// `sendfile` (Linux): kopie `count` bajtu z `input` do `output` pres
/// buffer (fsd nema kopii mezi soubory na strane sluzby). `offset` NULL =
/// od pozice `input` (posune ji), jinak od `*offset` (posune jen jej).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sendfile(output: c_int, input: c_int, offset: *mut i64, count: size_t) -> isize {
    let mut buf = vec![0u8; count.clamp(1, 1 << 16)];
    let mut done = 0usize;
    while done < count {
        let want = (count - done).min(buf.len());
        let r = if offset.is_null() {
            sys::read(input, &mut buf[..want])
        } else {
            // SAFETY: kontrakt C.
            let at = unsafe { *offset };
            if at < 0 {
                return errno::ret_val::<usize>(Err(errno::EINVAL));
            }
            sys::pread(input, &mut buf[..want], at as u64)
        };
        let n = match r {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if done == 0 => return errno::ret_val::<usize>(Err(e)),
            Err(_) => break,
        };
        let mut w = 0;
        while w < n {
            match sys::write(output, &buf[w..n]) {
                Ok(0) => break,
                Ok(k) => w += k,
                Err(e) if done + w == 0 => return errno::ret_val::<usize>(Err(e)),
                Err(_) => break,
            }
        }
        if !offset.is_null() {
            // SAFETY: kontrakt C.
            unsafe { *offset += w as i64 };
        }
        done += w;
        if w < n {
            break;
        }
    }
    done as isize
}
