//! `spawn.h`: `posix_spawn` a `posix_spawnp` s akcemi souboru a atributy
//! (rozhodnuti 4A, docs/cs/cstd.md). `fork` v systemu neni: potomka
//! spusti launchd stejne jako `std::process::Command`
//! (`sys::spawn_child`), cekani a stav potomku jsou ve [`wait`].
//!
//! Potomek dedi fd rodice bez `FD_CLOEXEC` (rozhodnuti T4A, jako POSIX).
//! Akce souboru se vyhodnoti uz v rodici nad kopii tabulky fd (`plan`):
//! - `addclose`, `addclosefrom` = cislo potomkovi nepredat;
//! - `adddup2(x, y)` = `y` ukazuje na totez co `x` (bez `FD_CLOEXEC`;
//!   `x == y` jen priznak zrusi);
//! - `addopen(y, ...)` otevre soubor rodic pred spustenim; `/dev/null`
//!   = prazdny vstup / zahozeny vystup (nad 2 prazdny fd);
//! - `addchdir`, `addfchdir` = pracovni adresar potomka.
//!
//! Na konci se zahodi cisla s `FD_CLOEXEC`. Fd 0-2 dostane potomek jako
//! proudy (soubor = proud z fsd, `OPEN_STREAM`), vyssi cisla jako zdedene
//! fd v bloku parametru (`osrt::inherit`: roura a pametovy fd = duplikat
//! handlu, soubor = `REOPEN` ve fsd s kopii pozice, socket TCP/UDP = novy
//! kanal ke stejnemu socketu u netd). `epoll` zdedit nejde: zdedeny se
//! preskoci, z akce `adddup2` = `ENOTSUP`.
//!
//! Program: cesta s `/` (relativni vuci adresari potomka), u `posix_spawn`
//! holy nazev = soubor v adresari potomka, kdyz existuje; `posix_spawnp`
//! hleda holy nazev v `PATH`. Nenalezeny holy nazev dostane launchd jako
//! program systemu (registr prikazu, `/system/bin`, initfs).

pub mod wait;

use crate::signal::Set;
use crate::sys::{self, ChildIo, Kind, SpawnReq};
use crate::{c_bytes, c_char, c_int, errno};

pub const POSIX_SPAWN_RESETIDS: i16 = 0x01;
pub const POSIX_SPAWN_SETPGROUP: i16 = 0x02;
pub const POSIX_SPAWN_SETSIGDEF: i16 = 0x04;
pub const POSIX_SPAWN_SETSIGMASK: i16 = 0x08;
pub const POSIX_SPAWN_SETSCHEDPARAM: i16 = 0x10;
pub const POSIX_SPAWN_SETSCHEDULER: i16 = 0x20;
pub const POSIX_SPAWN_USEVFORK: i16 = 0x40;
pub const POSIX_SPAWN_SETSID: i16 = 0x80;
const FLAGS_ALL: i16 = 0xff;

/// Jedna akce souboru.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Close(c_int),
    Dup2(c_int, c_int),
    /// fd, cesta, priznaky `open`, prava noveho souboru.
    Open(c_int, Vec<u8>, c_int, u32),
    Chdir(Vec<u8>),
    Fchdir(c_int),
    CloseFrom(c_int),
}

/// `posix_spawn_file_actions_t` (80 B jako glibc); seznam akci je
/// v alokaci za ukazatelem.
#[repr(C)]
pub struct posix_spawn_file_actions_t {
    list: *mut Vec<Action>,
    _pad: [u8; 72],
}

/// `posix_spawnattr_t` (336 B jako glibc).
#[repr(C)]
pub struct posix_spawnattr_t {
    flags: i16,
    pgroup: c_int,
    sigdefault: Set,
    sigmask: Set,
    priority: c_int,
    policy: c_int,
    _pad: [c_int; 16],
}

/// Vysledek akci souboru: proudy 0-2, zdedene fd a pracovni adresar
/// potomka.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub io: [ChildIo; 3],
    /// Fd nad 2 vzestupne: (cislo v potomkovi, zdroj, z akce souboru).
    pub fds: Vec<(c_int, ChildIo, bool)>,
    pub cwd: Vec<u8>,
    /// Soubory k otevreni pred spustenim (`ChildIo::Open(index)`):
    /// absolutni cesta, priznaky, prava.
    pub opens: Vec<(Vec<u8>, c_int, u32)>,
}

/// `base` + `p` (absolutni `p` beze zmeny).
pub fn join(base: &[u8], p: &[u8]) -> Vec<u8> {
    if p.first() == Some(&b'/') {
        return p.to_vec();
    }
    let mut out = base.to_vec();
    if out.last() != Some(&b'/') {
        out.push(b'/');
    }
    out.extend_from_slice(p);
    out
}

/// Cislo v tabulce potomka behem `plan`.
#[derive(Clone, Copy)]
struct Entry {
    src: ChildIo,
    cloexec: bool,
    /// Z akce souboru (ne jen zdedene).
    explicit: bool,
}

/// Vyhodnoti akce souboru (poradi jako v potomkovi). `cwd` = adresar
/// rodice, `parent` = otevrena cisla rodice s `FD_CLOEXEC`, `dir_of` =
/// cesta adresare za cislem fd rodice.
pub fn plan(
    actions: &[Action],
    cwd: Vec<u8>,
    parent: &[(c_int, bool)],
    dir_of: impl Fn(c_int) -> Result<Vec<u8>, c_int>,
) -> Result<Plan, c_int> {
    use std::collections::BTreeMap;
    let mut t: BTreeMap<c_int, Entry> =
        parent.iter().map(|&(fd, cloexec)| (fd, Entry { src: ChildIo::Parent(fd), cloexec, explicit: false })).collect();
    let mut cwd = cwd;
    let mut opens = Vec::new();
    for a in actions {
        match a {
            Action::Close(fd) => {
                t.remove(fd);
            }
            Action::Dup2(from, to) => {
                let Some(e) = t.get(from).copied() else { return Err(errno::EBADF) };
                t.insert(*to, Entry { src: e.src, cloexec: false, explicit: true });
            }
            Action::Open(fd, path, flags, mode) => {
                let src = if path.as_slice() == b"/dev/null" {
                    ChildIo::Null
                } else {
                    // Relativni cesta plati vuci adresari potomka v tomto kroku.
                    opens.push((join(&cwd, path), *flags, *mode));
                    ChildIo::Open(opens.len() - 1)
                };
                t.insert(*fd, Entry { src, cloexec: flags & crate::unistd::O_CLOEXEC != 0, explicit: true });
            }
            Action::Chdir(p) => cwd = join(&cwd, p),
            Action::Fchdir(fd) => cwd = dir_of(*fd)?,
            Action::CloseFrom(from) => t.retain(|fd, _| fd < from),
        }
    }
    t.retain(|_, e| !e.cloexec);
    let get = |i: c_int| t.get(&i).map_or(ChildIo::Closed, |e| e.src);
    let io = [get(0), get(1), get(2)];
    let fds = t.iter().filter(|(fd, _)| **fd > 2).map(|(fd, e)| (*fd, e.src, e.explicit)).collect();
    Ok(Plan { io, fds, cwd, opens })
}

/// Program pro launchd. `exists` = je to bezny soubor?
pub fn resolve_program(file: &[u8], cwd: &[u8], path_var: Option<&[u8]>, exists: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    if file.contains(&b'/') {
        return join(cwd, file);
    }
    match path_var {
        // posix_spawnp: adresare z PATH (prazdna polozka = adresar potomka).
        Some(pv) => {
            for dir in pv.split(|&c| c == b':') {
                let d = if dir.is_empty() { cwd } else { dir };
                let cand = join(&join(cwd, d), file);
                if exists(&cand) {
                    return cand;
                }
            }
        }
        None => {
            let cand = join(cwd, file);
            if exists(&cand) {
                return cand;
            }
        }
    }
    file.to_vec()
}

/// Pole retezcu C zakoncene NULL.
///
/// # Safety
///
/// `p` je NULL nebo pole platnych retezcu zakoncene NULL.
unsafe fn strings(p: *const *const c_char) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    if p.is_null() {
        return out;
    }
    let mut i = 0;
    // SAFETY: kontrakt C.
    unsafe {
        while !(*p.add(i)).is_null() {
            out.push(c_bytes(*p.add(i)).to_vec());
            i += 1;
        }
    }
    out
}

/// Kontrola atributu v dobe spusteni.
fn check_attr(a: &posix_spawnattr_t) -> Result<(), c_int> {
    // Do existujici skupiny procesu se pridat nejde (skupiny nejsou;
    // potomek je vzdy v novem jobu, coz odpovida pgroup 0 a SETSID).
    if a.flags & POSIX_SPAWN_SETPGROUP != 0 && a.pgroup != 0 {
        return Err(errno::ENOTSUP);
    }
    // Realny cas aplikace nedostanou (jako Linux bez opravneni).
    if a.flags & POSIX_SPAWN_SETSCHEDULER != 0 && a.policy != crate::pthread::SCHED_OTHER {
        return Err(errno::EPERM);
    }
    if a.flags & POSIX_SPAWN_SETSCHEDPARAM != 0 && a.priority != 0 {
        return Err(errno::EPERM);
    }
    Ok(())
}

fn close_all(fds: &[c_int]) {
    for &fd in fds {
        let _ = sys::close(fd);
    }
}

/// Spolecny zaklad `posix_spawn` a `posix_spawnp`; vraci kod chyby.
///
/// # Safety
///
/// Ukazatele podle smlouvy `posix_spawn`.
unsafe fn spawn(
    pid: *mut c_int,
    file: *const c_char,
    fa: *const posix_spawn_file_actions_t,
    attr: *const posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
    search: bool,
) -> c_int {
    if file.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    if let Some(a) = unsafe { attr.as_ref() } {
        if let Err(e) = check_attr(a) {
            return e;
        }
    }
    // SAFETY: kontrakt C (seznam z posix_spawn_file_actions_init).
    let actions: &[Action] = match unsafe { fa.as_ref() } {
        Some(f) if !f.list.is_null() => unsafe { &*f.list },
        _ => &[],
    };
    let plan = match plan(actions, sys::getcwd(), &sys::fd_list(), sys::fd_path) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match sys::stat(&plan.cwd) {
        Ok(s) if s.kind == Kind::Dir => {}
        Ok(_) => return errno::ENOTDIR,
        Err(e) => return e,
    }
    let path_var = search.then(|| std::env::var_os("PATH").map(|p| p.into_encoded_bytes()).unwrap_or_else(|| b"/system/bin".to_vec()));
    // SAFETY: retezec C.
    let name = unsafe { c_bytes(file) };
    let exists = |p: &[u8]| matches!(sys::stat(p), Ok(s) if s.kind == Kind::File);
    let program = resolve_program(name, &plan.cwd, path_var.as_deref(), exists);
    let Ok(program) = String::from_utf8(program) else { return errno::EINVAL };
    // `.` a `..` vyresit jako `open` (fsd ani launchd je v ceste neberou;
    // Wine spousti `.../lib/wine/../../bin/wineserver`).
    let program = if program.starts_with('/') {
        match sys::abs_path(program.as_bytes()).map(String::from_utf8) {
            Ok(Ok(p)) => p,
            Ok(Err(_)) => return errno::EINVAL,
            Err(e) => return e,
        }
    } else {
        program
    };
    // SAFETY: kontrakt C.
    let mut args = unsafe { strings(argv) };
    if args.is_empty() {
        args.push(name.to_vec());
    }
    let env = if envp.is_null() {
        std::env::vars_os()
            .map(|(k, v)| {
                let mut e = k.into_encoded_bytes();
                e.push(b'=');
                e.extend_from_slice(v.as_encoded_bytes());
                e
            })
            .collect()
    } else {
        // SAFETY: kontrakt C.
        unsafe { strings(envp) }
    };
    // Soubory z `addopen` otevre rodic; potomek dostane proud z fsd (0-2)
    // nebo zdedeny soubor (nad 2).
    let mut temp = Vec::new();
    let mut io = plan.io;
    let mut fds = plan.fds;
    for slot in io.iter_mut().chain(fds.iter_mut().map(|(_, s, _)| s)) {
        if let ChildIo::Open(k) = *slot {
            let (path, flags, _mode) = &plan.opens[k];
            let Some(o) = crate::unistd::open_flags(*flags) else {
                close_all(&temp);
                return errno::EINVAL;
            };
            match sys::open(path, &o) {
                Ok(fd) => {
                    temp.push(fd);
                    *slot = ChildIo::Parent(fd);
                }
                Err(e) => {
                    close_all(&temp);
                    return e;
                }
            }
        }
    }
    let req = SpawnReq { program, args, env, cwd: plan.cwd, io, fds };
    let spawned = sys::spawn_child(req);
    close_all(&temp);
    match spawned {
        Ok(c) => {
            let id = wait::register(c);
            // SAFETY: kontrakt C.
            if let Some(p) = unsafe { pid.as_mut() } {
                *p = id;
            }
            0
        }
        Err(e) => e,
    }
}

/// # Safety
///
/// Ukazatele podle smlouvy POSIX.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn(
    pid: *mut c_int,
    path: *const c_char,
    fa: *const posix_spawn_file_actions_t,
    attr: *const posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { spawn(pid, path, fa, attr, argv, envp, false) }
}

/// # Safety
///
/// Ukazatele podle smlouvy POSIX.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnp(
    pid: *mut c_int,
    file: *const c_char,
    fa: *const posix_spawn_file_actions_t,
    attr: *const posix_spawnattr_t,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { spawn(pid, file, fa, attr, argv, envp, true) }
}

// --- akce souboru ------------------------------------------------------------

/// # Safety
///
/// `fa` ukazuje na pamet pro `posix_spawn_file_actions_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_init(fa: *mut posix_spawn_file_actions_t) -> c_int {
    if fa.is_null() {
        return errno::EINVAL;
    }
    let list = Box::into_raw(Box::new(Vec::new()));
    // SAFETY: kontrakt C.
    unsafe { fa.write(posix_spawn_file_actions_t { list, _pad: [0; 72] }) };
    0
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_destroy(fa: *mut posix_spawn_file_actions_t) -> c_int {
    // SAFETY: kontrakt C.
    let Some(f) = (unsafe { fa.as_mut() }) else { return errno::EINVAL };
    if !f.list.is_null() {
        // SAFETY: alokace z posix_spawn_file_actions_init.
        drop(unsafe { Box::from_raw(f.list) });
        f.list = core::ptr::null_mut();
    }
    0
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
unsafe fn push(fa: *mut posix_spawn_file_actions_t, a: Action) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { fa.as_mut() } {
        Some(f) if !f.list.is_null() => {
            // SAFETY: alokace z posix_spawn_file_actions_init.
            unsafe { (*f.list).push(a) };
            0
        }
        _ => errno::EINVAL,
    }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addclose(fa: *mut posix_spawn_file_actions_t, fd: c_int) -> c_int {
    if fd < 0 {
        return errno::EBADF;
    }
    // SAFETY: kontrakt C.
    unsafe { push(fa, Action::Close(fd)) }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_adddup2(fa: *mut posix_spawn_file_actions_t, fd: c_int, newfd: c_int) -> c_int {
    if fd < 0 || newfd < 0 {
        return errno::EBADF;
    }
    // SAFETY: kontrakt C.
    unsafe { push(fa, Action::Dup2(fd, newfd)) }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci, `path` retezec C.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addopen(
    fa: *mut posix_spawn_file_actions_t,
    fd: c_int,
    path: *const c_char,
    oflag: c_int,
    mode: u32,
) -> c_int {
    if fd < 0 {
        return errno::EBADF;
    }
    if path.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe { push(fa, Action::Open(fd, c_bytes(path).to_vec(), oflag, mode)) }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci, `path` retezec C.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addchdir_np(fa: *mut posix_spawn_file_actions_t, path: *const c_char) -> c_int {
    if path.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe { push(fa, Action::Chdir(c_bytes(path).to_vec())) }
}

/// POSIX 2024 jmeno `addchdir_np`.
///
/// # Safety
///
/// `fa` je inicializovany seznam akci, `path` retezec C.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addchdir(fa: *mut posix_spawn_file_actions_t, path: *const c_char) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { posix_spawn_file_actions_addchdir_np(fa, path) }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addfchdir_np(fa: *mut posix_spawn_file_actions_t, fd: c_int) -> c_int {
    if fd < 0 {
        return errno::EBADF;
    }
    // SAFETY: kontrakt C.
    unsafe { push(fa, Action::Fchdir(fd)) }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addfchdir(fa: *mut posix_spawn_file_actions_t, fd: c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { posix_spawn_file_actions_addfchdir_np(fa, fd) }
}

/// # Safety
///
/// `fa` je inicializovany seznam akci.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawn_file_actions_addclosefrom_np(fa: *mut posix_spawn_file_actions_t, from: c_int) -> c_int {
    if from < 0 {
        return errno::EBADF;
    }
    // SAFETY: kontrakt C.
    unsafe { push(fa, Action::CloseFrom(from)) }
}

// --- atributy ------------------------------------------------------------------

/// # Safety
///
/// `a` ukazuje na pamet pro `posix_spawnattr_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_init(a: *mut posix_spawnattr_t) -> c_int {
    if a.is_null() {
        return errno::EINVAL;
    }
    let v = posix_spawnattr_t {
        flags: 0,
        pgroup: 0,
        sigdefault: [0; 16],
        sigmask: [0; 16],
        priority: 0,
        policy: crate::pthread::SCHED_OTHER,
        _pad: [0; 16],
    };
    // SAFETY: kontrakt C.
    unsafe { a.write(v) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn posix_spawnattr_destroy(_a: *mut posix_spawnattr_t) -> c_int {
    0
}

/// Pristup k atributu; NULL = `EINVAL`.
///
/// # Safety
///
/// `a` je NULL nebo inicializovany atribut.
unsafe fn with_attr(a: *mut posix_spawnattr_t, f: impl FnOnce(&mut posix_spawnattr_t) -> c_int) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { a.as_mut() } {
        Some(a) => f(a),
        None => errno::EINVAL,
    }
}

/// Hodnotu do vystupu; NULL = `EINVAL`.
fn put<T>(out: *mut T, v: T) -> c_int {
    if out.is_null() {
        return errno::EINVAL;
    }
    // SAFETY: platny ukazatel (kontrakt C).
    unsafe { out.write(v) };
    0
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_setflags(a: *mut posix_spawnattr_t, flags: i16) -> c_int {
    if flags & !FLAGS_ALL != 0 {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe {
        with_attr(a, |a| {
            a.flags = flags;
            0
        })
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `out` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_getflags(a: *const posix_spawnattr_t, out: *mut i16) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { with_attr(a.cast_mut(), |a| put(out, a.flags)) }
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_setpgroup(a: *mut posix_spawnattr_t, pgroup: c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe {
        with_attr(a, |a| {
            a.pgroup = pgroup;
            0
        })
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `out` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_getpgroup(a: *const posix_spawnattr_t, out: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { with_attr(a.cast_mut(), |a| put(out, a.pgroup)) }
}

/// Maska signalu potomka: potomek zacina vzdy s prazdnou maskou a vychozimi
/// obsluhami (novy proces), maska se jen ulozi.
///
/// # Safety
///
/// `a` je inicializovany atribut, `s` platna mnozina.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_setsigmask(a: *mut posix_spawnattr_t, s: *const Set) -> c_int {
    // SAFETY: kontrakt C.
    let Some(s) = (unsafe { s.as_ref() }) else { return errno::EINVAL };
    // SAFETY: kontrakt C.
    unsafe {
        with_attr(a, |a| {
            a.sigmask = *s;
            0
        })
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `out` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_getsigmask(a: *const posix_spawnattr_t, out: *mut Set) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { with_attr(a.cast_mut(), |a| put(out, a.sigmask)) }
}

/// # Safety
///
/// `a` je inicializovany atribut, `s` platna mnozina.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_setsigdefault(a: *mut posix_spawnattr_t, s: *const Set) -> c_int {
    // SAFETY: kontrakt C.
    let Some(s) = (unsafe { s.as_ref() }) else { return errno::EINVAL };
    // SAFETY: kontrakt C.
    unsafe {
        with_attr(a, |a| {
            a.sigdefault = *s;
            0
        })
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `out` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_getsigdefault(a: *const posix_spawnattr_t, out: *mut Set) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { with_attr(a.cast_mut(), |a| put(out, a.sigdefault)) }
}

/// # Safety
///
/// `a` je inicializovany atribut.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_setschedpolicy(a: *mut posix_spawnattr_t, policy: c_int) -> c_int {
    if !matches!(policy, 0..=2) {
        return errno::EINVAL;
    }
    // SAFETY: kontrakt C.
    unsafe {
        with_attr(a, |a| {
            a.policy = policy;
            0
        })
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `out` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_getschedpolicy(a: *const posix_spawnattr_t, out: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { with_attr(a.cast_mut(), |a| put(out, a.policy)) }
}

/// # Safety
///
/// `a` je inicializovany atribut, `p` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_setschedparam(a: *mut posix_spawnattr_t, p: *const crate::pthread::sched_param) -> c_int {
    // SAFETY: kontrakt C.
    let Some(p) = (unsafe { p.as_ref() }) else { return errno::EINVAL };
    // SAFETY: kontrakt C.
    unsafe {
        with_attr(a, |a| {
            a.priority = p.sched_priority;
            0
        })
    }
}

/// # Safety
///
/// `a` je inicializovany atribut, `p` platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn posix_spawnattr_getschedparam(a: *const posix_spawnattr_t, p: *mut crate::pthread::sched_param) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { with_attr(a.cast_mut(), |a| put(p, crate::pthread::sched_param { sched_priority: a.priority })) }
}
