//! Dalsi bezne funkce libc pro porty (zlib, sqlite, ...): retezce
//! (`strspn`, `strdup`, `strcasecmp`, ...), `qsort`, `bsearch`, `rand`,
//! `div`, `pwd.h` (uzivatel relace), `ioctl`, `locale.h` (jen C a
//! C.UTF-8), `uname`, `system`.

use core::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{c_bytes, c_char, c_int, c_long, errno, size_t};

fn set_of(s: *const c_char) -> [bool; 256] {
    let mut t = [false; 256];
    // SAFETY: retezec C.
    for &b in unsafe { c_bytes(s) } {
        t[b as usize] = true;
    }
    t
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strspn(s: *const c_char, accept: *const c_char) -> size_t {
    let t = set_of(accept);
    // SAFETY: retezec C.
    unsafe { c_bytes(s) }.iter().take_while(|&&b| t[b as usize]).count()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strcspn(s: *const c_char, reject: *const c_char) -> size_t {
    let t = set_of(reject);
    // SAFETY: retezec C.
    unsafe { c_bytes(s) }.iter().take_while(|&&b| !t[b as usize]).count()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strpbrk(s: *const c_char, accept: *const c_char) -> *mut c_char {
    // SAFETY: kontrakt C.
    let n = unsafe { strcspn(s, accept) };
    // SAFETY: n <= delka retezce.
    if unsafe { *s.add(n) } == 0 { core::ptr::null_mut() } else { unsafe { s.add(n) as *mut c_char } }
}

/// `strtok_r` (vlaknove bezpecna varianta; `strtok` zamerne neni).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strtok_r(s: *mut c_char, delim: *const c_char, save: *mut *mut c_char) -> *mut c_char {
    // SAFETY: kontrakt C (s nebo *save je retezec).
    unsafe {
        let mut p = if s.is_null() { *save } else { s };
        if p.is_null() {
            return core::ptr::null_mut();
        }
        p = p.add(strspn(p, delim));
        if *p == 0 {
            *save = core::ptr::null_mut();
            return core::ptr::null_mut();
        }
        let end = p.add(strcspn(p, delim));
        if *end == 0 {
            *save = core::ptr::null_mut();
        } else {
            *end = 0;
            *save = end.add(1);
        }
        p
    }
}

fn dup_bytes(b: &[u8]) -> *mut c_char {
    // SAFETY: malloc z cstd-core.
    let p = unsafe { cstd_core::stdlib::malloc(b.len() + 1) };
    if p.is_null() {
        errno::set(errno::ENOMEM);
        return p;
    }
    // SAFETY: p ma len + 1 bajtu.
    unsafe {
        core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
        *p.add(b.len()) = 0;
    }
    p
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strdup(s: *const c_char) -> *mut c_char {
    // SAFETY: retezec C.
    dup_bytes(unsafe { c_bytes(s) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strndup(s: *const c_char, n: size_t) -> *mut c_char {
    // SAFETY: retezec C (cte nejvys n bajtu).
    let len = unsafe { cstd_core::string::strnlen(s, n) };
    // SAFETY: len bajtu je platnych.
    dup_bytes(unsafe { core::slice::from_raw_parts(s, len) })
}

fn casecmp(a: *const c_char, b: *const c_char, n: usize) -> c_int {
    for i in 0..n {
        // SAFETY: retezce C (konci nulou, cte se jen do ni).
        let (x, y) = unsafe { ((*a.add(i)).to_ascii_lowercase(), (*b.add(i)).to_ascii_lowercase()) };
        if x != y || x == 0 {
            return c_int::from(x) - c_int::from(y);
        }
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strcasecmp(a: *const c_char, b: *const c_char) -> c_int {
    casecmp(a, b, usize::MAX)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strncasecmp(a: *const c_char, b: *const c_char, n: size_t) -> c_int {
    casecmp(a, b, n)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn stpcpy(dst: *mut c_char, src: *const c_char) -> *mut c_char {
    // SAFETY: kontrakt C.
    unsafe {
        let b = c_bytes(src);
        core::ptr::copy_nonoverlapping(b.as_ptr(), dst, b.len() + 1);
        dst.add(b.len())
    }
}

/// `strcoll` v locale C = `strcmp`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strcoll(a: *const c_char, b: *const c_char) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::string::strcmp(a, b) }
}

/// `strerror_r` (XSI): 0, nebo `ERANGE` pri malem bufferu.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strerror_r(e: c_int, buf: *mut c_char, n: size_t) -> c_int {
    let m = errno::message(e);
    let text = &m[..m.len() - 1];
    if n == 0 {
        return errno::ERANGE;
    }
    let k = text.len().min(n - 1);
    // SAFETY: buf ma n bajtu.
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buf, k);
        *buf.add(k) = 0;
    }
    if k < text.len() { errno::ERANGE } else { 0 }
}

// --- stdlib ------------------------------------------------------------------

type Cmp = unsafe extern "C" fn(*const c_void, *const c_void) -> c_int;

/// `qsort`: razeni slevanim nad indexy (stabilni, bez rekurze pres C),
/// pak permutace prvku v poli.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn qsort(base: *mut c_void, n: size_t, size: size_t, cmp: Cmp) {
    if n < 2 || size == 0 {
        return;
    }
    let p = base.cast::<u8>();
    let mut idx: Vec<usize> = (0..n).collect();
    // SAFETY: prvky lezi v poli (kontrakt C).
    idx.sort_by(|&a, &b| unsafe { cmp(p.add(a * size).cast(), p.add(b * size).cast()) }.cmp(&0));
    let mut tmp = vec![0u8; n * size];
    for (i, &src) in idx.iter().enumerate() {
        // SAFETY: zdroj i cil v mezich.
        unsafe { core::ptr::copy_nonoverlapping(p.add(src * size), tmp.as_mut_ptr().add(i * size), size) };
    }
    // SAFETY: pole ma n * size bajtu.
    unsafe { core::ptr::copy_nonoverlapping(tmp.as_ptr(), p, n * size) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn bsearch(key: *const c_void, base: *const c_void, n: size_t, size: size_t, cmp: Cmp) -> *mut c_void {
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        // SAFETY: prvek v poli.
        let e = unsafe { base.cast::<u8>().add(mid * size) };
        // SAFETY: kontrakt C.
        match unsafe { cmp(key, e.cast()) } {
            0 => return e as *mut c_void,
            r if r < 0 => hi = mid,
            _ => lo = mid + 1,
        }
    }
    core::ptr::null_mut()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn labs(v: c_long) -> c_long {
    v.wrapping_abs()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn llabs(v: i64) -> i64 {
    v.wrapping_abs()
}

#[repr(C)]
pub struct div_t {
    pub quot: c_int,
    pub rem: c_int,
}

#[repr(C)]
pub struct ldiv_t {
    pub quot: c_long,
    pub rem: c_long,
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn div(a: c_int, b: c_int) -> div_t {
    div_t { quot: a / b, rem: a % b }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn ldiv(a: c_long, b: c_long) -> ldiv_t {
    ldiv_t { quot: a / b, rem: a % b }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn atol(s: *const c_char) -> c_long {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::stdlib::strtol(s, core::ptr::null_mut(), 10) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn atoll(s: *const c_char) -> i64 {
    // SAFETY: kontrakt C.
    unsafe { cstd_core::stdlib::strtoll(s, core::ptr::null_mut(), 10) }
}

/// Generator `rand` (xorshift64*, ne kryptograficky; `RAND_MAX` 2^31 - 1).
static SEED: AtomicU64 = AtomicU64::new(0x2545_F491_4F6C_DD1D);

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn rand() -> c_int {
    let mut x = SEED.load(Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    SEED.store(x, Ordering::Relaxed);
    ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 0x7fff_ffff) as c_int
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn srand(seed: u32) {
    SEED.store(u64::from(seed).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, Ordering::Relaxed);
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn random() -> c_long {
    c_long::from(rand())
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn srandom(seed: u32) {
    srand(seed);
}

/// `system`: shell systemu z knihovny C spoustet nejde (procesy jen
/// `posix_spawn` [navrzeno]); `system(NULL)` = 0 (shell neni).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn system(cmd: *const c_char) -> c_int {
    if cmd.is_null() {
        return 0;
    }
    errno::set(errno::ENOSYS);
    -1
}

// --- pwd.h, uzivatele -----------------------------------------------------------

/// `struct passwd` (jako glibc).
#[repr(C)]
pub struct passwd {
    pub pw_name: *mut c_char,
    pub pw_passwd: *mut c_char,
    pub pw_uid: u32,
    pub pw_gid: u32,
    pub pw_gecos: *mut c_char,
    pub pw_dir: *mut c_char,
    pub pw_shell: *mut c_char,
}

struct Pw(passwd);
// SAFETY: retezce jsou staticke a jen pro cteni.
unsafe impl Send for Pw {}

static PW: Mutex<Option<Pw>> = Mutex::new(None);

/// Uzivatel relace: jmeno z `USER`, domov z `HOME` (blok parametru).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getpwuid(_uid: u32) -> *mut passwd {
    let mut g = PW.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let leak = |s: String| std::ffi::CString::new(s).unwrap_or_default().into_raw().cast::<c_char>();
        let name = std::env::var("USER").unwrap_or_else(|_| String::from("user"));
        let home = std::env::var("HOME").unwrap_or_else(|_| String::from("/"));
        *g = Some(Pw(passwd {
            pw_name: leak(name),
            pw_passwd: leak(String::from("x")),
            pw_uid: 0,
            pw_gid: 0,
            pw_gecos: leak(String::new()),
            pw_dir: leak(home),
            pw_shell: leak(String::from("/bin/sh")),
        }));
    }
    g.as_mut().map_or(core::ptr::null_mut(), |p| &mut p.0 as *mut passwd)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getpwnam(_name: *const c_char) -> *mut passwd {
    getpwuid(0)
}

/// Cisla uzivatelu a skupin POSIX: identita je v relaci FS, ne cislo;
/// vsechny 0 (vlastnik vlastnich souboru), `chown` nema ucinek.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getuid() -> u32 {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn geteuid() -> u32 {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getgid() -> u32 {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getegid() -> u32 {
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn fchown(fd: c_int, _u: u32, _g: u32) -> c_int {
    errno::ret(crate::sys::status(fd).map(|_| ()))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn chown(path: *const c_char, _u: u32, _g: u32) -> c_int {
    // SAFETY: retezec C.
    errno::ret(crate::sys::stat(unsafe { c_bytes(path) }).map(|_| ()))
}

/// `readlink`: symlinky FS zatim nevytvari, kazda cesta = `EINVAL`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn readlink(path: *const c_char, _buf: *mut c_char, _n: size_t) -> isize {
    // SAFETY: retezec C.
    match crate::sys::stat(unsafe { c_bytes(path) }) {
        Ok(_) => errno::ret_val::<usize>(Err(errno::EINVAL)),
        Err(e) => errno::ret_val::<usize>(Err(e)),
    }
}

/// `ioctl`: terminal zatim nema ovladani (`ENOTTY`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn ioctl(fd: c_int, _req: u64, _args: ...) -> c_int {
    match crate::sys::status(fd) {
        Ok(_) => errno::ret(Err(errno::ENOTTY)),
        Err(e) => errno::ret(Err(e)),
    }
}

// --- uname ----------------------------------------------------------------------

#[repr(C)]
pub struct utsname {
    pub sysname: [c_char; 65],
    pub nodename: [c_char; 65],
    pub release: [c_char; 65],
    pub version: [c_char; 65],
    pub machine: [c_char; 65],
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn uname(u: *mut utsname) -> c_int {
    // SAFETY: kontrakt C.
    let Some(u) = (unsafe { u.as_mut() }) else { return errno::ret(Err(errno::EFAULT)) };
    let put = |dst: &mut [c_char; 65], s: &str| {
        let n = s.len().min(64);
        dst[..n].copy_from_slice(&s.as_bytes()[..n]);
        dst[n] = 0;
    };
    put(&mut u.sysname, "MagnumOpus");
    put(&mut u.nodename, "magnumopus");
    put(&mut u.release, env!("CARGO_PKG_VERSION"));
    put(&mut u.version, "MagnumOpus");
    put(&mut u.machine, if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" });
    0
}

// --- casy souboru a prostredky --------------------------------------------------

/// `utimes`: nastaveni casu souboru protokol FS zatim nema; uspech, kdyz
/// soubor existuje (cas zmeny urci posledni zapis).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn utimes(path: *const c_char, _tv: *const c_void) -> c_int {
    // SAFETY: retezec C.
    errno::ret(crate::sys::stat(unsafe { c_bytes(path) }).map(|_| ()))
}

/// `getrusage`: cas CPU jadro nehlasi, vsechno nuly.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getrusage(_who: c_int, u: *mut u8) -> c_int {
    if u.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    // SAFETY: struct rusage ma 144 B (dva timeval + 14 long).
    unsafe { core::ptr::write_bytes(u, 0, 144) };
    0
}

#[repr(C)]
pub struct rlimit {
    pub cur: u64,
    pub max: u64,
}

/// `getrlimit`: pocet fd 4096, ostatni bez limitu (rozpocet jobu je
/// v bajtech pameti, ne v POSIX limitech).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getrlimit(res: c_int, r: *mut rlimit) -> c_int {
    // SAFETY: kontrakt C.
    let Some(r) = (unsafe { r.as_mut() }) else { return errno::ret(Err(errno::EFAULT)) };
    let v = if res == 7 { 4096 } else { u64::MAX };
    *r = rlimit { cur: v, max: v };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setrlimit(_res: c_int, _r: *const rlimit) -> c_int {
    errno::ret(Err(errno::EPERM))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn utime(path: *const c_char, _t: *const c_void) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { utimes(path, core::ptr::null()) }
}

/// Symlinky a pevne odkazy FS zatim nevytvari (FS v3).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn symlink(_t: *const c_char, _p: *const c_char) -> c_int {
    errno::ret(Err(errno::ENOTSUP))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn link(_o: *const c_char, _p: *const c_char) -> c_int {
    errno::ret(Err(errno::ENOTSUP))
}
