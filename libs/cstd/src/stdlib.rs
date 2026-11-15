//! `stdlib.h` (cast nad std): `exit`, `_Exit`, `atexit`, `getenv`, `abort`
//! a selhani `assert`. Pamet a prevody cisel jsou v cstd-core.

use std::collections::HashMap;
use std::ffi::CString;
use std::io::Write;
use std::sync::Mutex;

use crate::stdio::flush_all;
use crate::{c_bytes, c_char, c_int};

/// Ukonci proces: funkce `atexit` (v obracenem poradi), vyprazdneni proudu,
/// pak konec s kodem `code`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn exit(code: c_int) -> ! {
    // Handlery atexit a vyprazdneni proudu spusti osrt (at_exit) pri
    // `std::process::exit`; tady jen jistota pro proudy bez handleru.
    flush_all();
    std::process::exit(code)
}

/// Konec bez `atexit` a bez vyprazdneni proudu.
#[allow(non_snake_case)]
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn _Exit(code: c_int) -> ! {
    #[cfg(target_os = "magnumopus")]
    {
        std::os::magnumopus::rt::exit_now(code)
    }
    #[cfg(not(target_os = "magnumopus"))]
    std::process::exit(code)
}

/// `_exit` (unistd.h) = `_Exit`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn _exit(code: c_int) -> ! {
    _Exit(code)
}

/// Zaregistruje funkci pro konec procesu; 0 = OK, -1 = plno (limit 1024).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn atexit(f: extern "C" fn()) -> c_int {
    #[cfg(target_os = "magnumopus")]
    {
        crate::stdio::ensure_exit_hook();
        if std::os::magnumopus::rt::at_exit(f) { 0 } else { -1 }
    }
    #[cfg(not(target_os = "magnumopus"))]
    {
        let _ = f;
        -1
    }
}

/// Cache retezcu C pro `getenv`: vraceny ukazatel musi zustat platny i po
/// dalsich volanich (C ho nikdy neuvolni). Hodnota se drzi pro kazde jmeno;
/// zmena promenne vytvori novy retezec (stary zustane, maly unik je zamerny).
static ENV: Mutex<Option<HashMap<Vec<u8>, Vec<CString>>>> = Mutex::new(None);

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn getenv(name: *const c_char) -> *const c_char {
    if name.is_null() {
        return core::ptr::null();
    }
    // SAFETY: retezec C.
    let n = unsafe { c_bytes(name) };
    let Ok(key) = core::str::from_utf8(n) else { return core::ptr::null() };
    if key.is_empty() || key.contains('=') {
        return core::ptr::null();
    }
    let Some(val) = std::env::var_os(key) else { return core::ptr::null() };
    let Ok(c) = CString::new(val.into_encoded_bytes()) else { return core::ptr::null() };
    let mut guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let list = guard.get_or_insert_with(HashMap::new).entry(n.to_vec()).or_default();
    if let Some(last) = list.last()
        && *last == c
    {
        return last.as_ptr().cast();
    }
    list.push(c);
    list.last().map_or(core::ptr::null(), |c| c.as_ptr().cast())
}

/// Platne jmeno promenne (`setenv`, `unsetenv`).
fn env_name<'a>(name: *const c_char) -> Option<&'a str> {
    if name.is_null() {
        return None;
    }
    // SAFETY: retezec C.
    let n = core::str::from_utf8(unsafe { c_bytes(name) }).ok()?;
    (!n.is_empty() && !n.contains('=')).then_some(n)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setenv(name: *const c_char, value: *const c_char, overwrite: c_int) -> c_int {
    let (Some(n), false) = (env_name(name), value.is_null()) else {
        crate::errno::set(crate::errno::EINVAL);
        return -1;
    };
    if overwrite == 0 && std::env::var_os(n).is_some() {
        return 0;
    }
    // SAFETY: retezec C.
    let v = os_str(unsafe { c_bytes(value) });
    // SAFETY: prostredi procesu meni jen tato knihovna a std (osrt drzi
    // promenne pod zamkem, cteni z jinych vlaken je bezpecne).
    unsafe { std::env::set_var(n, v) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn unsetenv(name: *const c_char) -> c_int {
    let Some(n) = env_name(name) else {
        crate::errno::set(crate::errno::EINVAL);
        return -1;
    };
    // SAFETY: viz setenv.
    unsafe { std::env::remove_var(n) };
    0
}

/// Bajty hodnoty jako `OsStr` (na cili libovolne bajty, na hostiteli
/// jen UTF-8).
fn os_str(b: &[u8]) -> &std::ffi::OsStr {
    #[cfg(target_os = "magnumopus")]
    {
        std::os::magnumopus::ffi::OsStrExt::from_bytes(b)
    }
    #[cfg(not(target_os = "magnumopus"))]
    std::ffi::OsStr::new(core::str::from_utf8(b).unwrap_or(""))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn abort() -> ! {
    let _ = std::io::stderr().write_all(b"abort()\n");
    std::process::abort()
}

/// Selhani `assert` (volane z makra v `assert.h`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn __cstd_assert_fail(expr: *const c_char, file: *const c_char, line: c_int) -> ! {
    // SAFETY: retezce C z makra.
    let (e, f) = unsafe { (c_bytes(expr), c_bytes(file)) };
    flush_all();
    let mut err = std::io::stderr().lock();
    let _ = err.write_all(f);
    let _ = write!(err, ":{line}: assertion failed: ");
    let _ = err.write_all(e);
    let _ = err.write_all(b"\n");
    drop(err);
    std::process::abort()
}
