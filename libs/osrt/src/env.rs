//! Argumenty, promenne prostredi a pracovni adresar procesu.
//!
//! Pocatecni hodnoty jsou z bloku parametru (`abi::params`); zmeny
//! (`set_var`, `set_current_dir`) plati jen v tomto procesu. Pracovni
//! adresar si drzi runtime: sluzba FS zna jen absolutni cesty, relativni
//! cesty sklada [`resolve`].

use alloc::string::String;
use alloc::vec::Vec;

use libos::params::Params;
use libos::sync::Mutex;

use crate::start::Init;
use crate::{Error, Result};

static ARGS: Init<Vec<Vec<u8>>> = Init::new();
static VARS: Mutex<Vec<(Vec<u8>, Vec<u8>)>> = Mutex::new(Vec::new());
static CWD: Mutex<String> = Mutex::new(String::new());

/// Prazdne `environ` (jen ukoncujici NULL) pred startem.
static EMPTY_ENVIRON: [usize; 1] = [0];

/// `environ` pro programy v C (POSIX `extern char **environ`): pole
/// ukazatelu na "JMENO=HODNOTA\0" zakoncene NULL. Prestavi se pri startu
/// a pri kazde zmene promennych (`set_var`, `remove_var`) pod zamkem
/// `VARS`. Stara pole a retezce se neuvolnuji (ukazatele na ne mohl vzit
/// jiny kod; jako glibc): zmena prostredi stoji jednu kopii promennych.
#[unsafe(no_mangle)]
pub static mut environ: *const *const u8 = &raw const EMPTY_ENVIRON as *const *const u8;

/// Prestavi `environ` z promennych (volat s drzenym zamkem `VARS`).
fn rebuild_environ(vars: &[(Vec<u8>, Vec<u8>)]) {
    let mut ptrs: Vec<*const u8> = Vec::with_capacity(vars.len() + 1);
    for (k, v) in vars {
        let mut s = Vec::with_capacity(k.len() + v.len() + 2);
        s.extend_from_slice(k);
        s.push(b'=');
        s.extend_from_slice(v);
        s.push(0);
        ptrs.push(alloc::boxed::Box::leak(s.into_boxed_slice()).as_ptr());
    }
    ptrs.push(core::ptr::null());
    let arr = alloc::boxed::Box::leak(ptrs.into_boxed_slice()).as_ptr();
    // SAFETY: zapis ukazatele pod zamkem VARS; ctenari (C) vidi stare nebo
    // nove platne pole (obe zustavaji alokovana).
    unsafe { (&raw mut environ).write_volatile(arr) };
}

/// # Safety
///
/// Jen pri startu (viz `start::Init`).
pub(crate) unsafe fn init(params: Params) {
    let mut vars = Vec::new();
    for item in params.env {
        // Polozka bez `=` je cele jmeno s prazdnou hodnotou.
        let eq = item.iter().position(|&b| b == b'=').unwrap_or(item.len());
        let (k, v) = item.split_at(eq);
        let v = v.get(1..).unwrap_or(&[]);
        if !k.is_empty() {
            vars.retain(|(old, _): &(Vec<u8>, Vec<u8>)| old.as_slice() != k);
            vars.push((k.to_vec(), v.to_vec()));
        }
    }
    rebuild_environ(&vars);
    *VARS.lock() = vars;
    let cwd = String::from_utf8(params.cwd).ok().and_then(|c| normalize("/", &c).ok());
    *CWD.lock() = cwd.unwrap_or_else(|| String::from("/"));
    // SAFETY: predano volajicim.
    unsafe { ARGS.set(params.args) };
}

/// Argumenty procesu.
pub fn args() -> &'static [Vec<u8>] {
    ARGS.get().map_or(&[], Vec::as_slice)
}

pub fn var(key: &[u8]) -> Option<Vec<u8>> {
    VARS.lock().iter().find(|(k, _)| k.as_slice() == key).map(|(_, v)| v.clone())
}

pub fn set_var(key: &[u8], value: &[u8]) -> Result<()> {
    if key.is_empty() || key.contains(&b'=') || key.contains(&0) || value.contains(&0) {
        return Err(Error::InvalidArgs);
    }
    let mut vars = VARS.lock();
    match vars.iter_mut().find(|(k, _)| k.as_slice() == key) {
        Some((_, v)) => *v = value.to_vec(),
        None => vars.push((key.to_vec(), value.to_vec())),
    }
    rebuild_environ(&vars);
    Ok(())
}

pub fn remove_var(key: &[u8]) {
    let mut vars = VARS.lock();
    let before = vars.len();
    vars.retain(|(k, _)| k.as_slice() != key);
    if vars.len() != before {
        rebuild_environ(&vars);
    }
}

/// Kopie vsech promennych (v poradi pridani).
pub fn vars() -> Vec<(Vec<u8>, Vec<u8>)> {
    VARS.lock().clone()
}

/// Pracovni adresar (absolutni cesta).
pub fn current_dir() -> String {
    CWD.lock().clone()
}

/// Zmeni pracovni adresar; overi, ze cil je adresar.
pub fn set_current_dir(path: &[u8]) -> Result<()> {
    let abs = resolve(path)?;
    let st = crate::fs::stat(&abs)?;
    if st.kind != fsproto::kind::DIR && st.kind != fsproto::kind::DATASET {
        return Err(Error::NotADirectory);
    }
    *CWD.lock() = abs;
    Ok(())
}

/// Absolutni normalizovana cesta pro sluzbu FS (relativni vuci pracovnimu
/// adresari). Cesty v systemu jsou UTF-8.
pub fn resolve(path: &[u8]) -> Result<String> {
    let path = core::str::from_utf8(path).map_err(|_| Error::InvalidArgs)?;
    if path.is_empty() {
        return Err(Error::NotFound);
    }
    if path.starts_with('/') {
        normalize("/", path)
    } else {
        normalize(&CWD.lock(), path)
    }
}

/// Spoji `base` (absolutni) a `path`, vyresi `.` a `..` (nad korenem
/// zustava koren, jako v POSIX).
pub fn normalize(base: &str, path: &str) -> Result<String> {
    if path.contains('\0') {
        return Err(Error::InvalidArgs);
    }
    let mut parts: Vec<&str> = Vec::new();
    let full = if path.starts_with('/') { [path, ""] } else { [base, path] };
    for piece in full.iter().flat_map(|p| p.split('/')) {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => parts.push(name),
        }
    }
    let mut out = String::new();
    for p in &parts {
        out.push('/');
        out.push_str(p);
    }
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}
