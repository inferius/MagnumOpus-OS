//! Procesovy fd (`<magnumopus/process.h>`, Windows vrstva W2): program
//! sveri svou pamet serveru sve aplikace (wineserver: `ReadProcessMemory`,
//! debugger). Bezpecnost jako `ptrace` stejneho uzivatele na Linuxu: fd
//! nese jen prava pameti (`READ_MEMORY`, `WRITE_MEMORY`), ne ukonceni ani
//! ladeni, ma vzdy `FD_CLOEXEC` a jinemu procesu se dostane jen vyslovne
//! (`SCM_RIGHTS`, `posix_spawn` jen po zruseni `FD_CLOEXEC`).

use crate::{c_int, errno, sys};

/// Cteni (`mo_process_memory`).
pub const MO_PROCESS_READ: c_int = 0;
/// Zapis (`mo_process_memory`).
pub const MO_PROCESS_WRITE: c_int = 1;

/// `mo_self_process_fd`: novy procesovy fd vlastniho procesu (s
/// `FD_CLOEXEC`); -1 a `errno` pri chybe.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_self_process_fd() -> c_int {
    errno::ret_val(sys::self_process_fd().map(|fd| fd as i64)) as c_int
}

/// `mo_self_control_fd`: ridici fd vlastniho procesu jen s pravem ukonceni
/// (server aplikace ho smi ukoncit, jako `kill` stejneho uzivatele);
/// `FD_CLOEXEC`. -1 a `errno` pri chybe.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_self_control_fd() -> c_int {
    errno::ret_val(sys::self_control_fd().map(|fd| fd as i64)) as c_int
}

/// `mo_self_thread_fd`: fd volajiciho vlakna jen s pravem preruseni (server
/// aplikace mu smi posilat signaly, jako `tgkill`); `FD_CLOEXEC`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_self_thread_fd() -> c_int {
    errno::ret_val(sys::self_thread_fd().map(|fd| fd as i64)) as c_int
}

/// `mo_process_kill`: ukonci proces za ridicim fd (kod `EXIT_KILLED`, jako
/// `SIGKILL`). 0 nebo -1 a `errno`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_process_kill(fd: c_int) -> c_int {
    errno::ret(sys::process_kill(fd))
}

/// `mo_thread_signal`: posle signal `sig` (1..63) vlaknu za fd vlakna
/// (preruseni vlakna, cstd ho doruci obsluze signalu). 0 nebo -1 a `errno`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn mo_thread_signal(fd: c_int, sig: c_int) -> c_int {
    errno::ret(sys::thread_signal(fd, sig))
}

/// `mo_process_memory`: cteni (`MO_PROCESS_READ`) z `addr` procesu za
/// `fd` do `buf`, nebo zapis (`MO_PROCESS_WRITE`) `buf` do `addr`. Vraci
/// prenesene bajty (mene = castecna kopie), -1 a `errno` pri chybe
/// (`EFAULT` adresa, `EPERM`/`EACCES` chybi pravo, `EINVAL` neni procesovy
/// fd nebo neznama operace).
///
/// # Safety
///
/// `buf` ukazuje na `len` bajtu (pro cteni zapisovatelnych).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mo_process_memory(fd: c_int, op: c_int, addr: u64, buf: *mut u8, len: usize) -> isize {
    let write = match op {
        MO_PROCESS_READ => false,
        MO_PROCESS_WRITE => true,
        _ => return errno::ret_val(Err::<i64, _>(errno::EINVAL)) as isize,
    };
    errno::ret_val(sys::process_memory(fd, write, addr, buf, len).map(|n| n as i64)) as isize
}
