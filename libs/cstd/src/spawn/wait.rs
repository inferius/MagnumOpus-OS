//! `sys/wait.h`: cekani na potomky z `posix_spawn` (`waitpid`, `wait`,
//! `wait3`, `wait4`, `waitid`) a `kill` na potomka.
//!
//! Cisla potomku (`pid_t`) prideluje cstd (tabulka v procesu, jine
//! procesy je neznaji). Skupiny procesu nejsou: `waitpid(0)` a
//! `waitpid(-pgid)` cekaji na kohokoli jako `-1`. Stav se kopiruje jako
//! Linux (`WEXITSTATUS` = spodnich 8 bitu kodu); konec, ktery zpusobil
//! system (`PROCESS_KILL`, vyjimka CPU), se hlasi jako signal. `SIGCHLD`
//! rodic nedostava; zastaveni (`WUNTRACED`) nevznika.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use crate::signal::{SIGABRT, SIGFPE, SIGHUP, SIGILL, SIGINT, SIGKILL, SIGQUIT, SIGSEGV, SIGTERM, SIGTRAP};
use crate::sys::{self, Child};
use crate::{c_int, errno};

pub const WNOHANG: c_int = 1;
pub const WUNTRACED: c_int = 2;
pub const WSTOPPED: c_int = 2;
pub const WEXITED: c_int = 4;
pub const WCONTINUED: c_int = 8;
pub const WNOWAIT: c_int = 0x0100_0000;

pub const P_ALL: c_int = 0;
pub const P_PID: c_int = 1;
pub const P_PGID: c_int = 2;

/// Kody konce od systemu (`abi::process`).
const EXIT_KILLED: i64 = -1_000_000;
const EXIT_PANIC: i64 = -1_000_001;
const EXIT_NO_THREADS: i64 = -1_000_002;
const EXIT_FAULT_BASE: i64 = -1_000_100;

/// Stav pro `waitpid` z kodu konce procesu. `sent` = signal z `kill`,
/// kterym rodic potomka ukoncil (jinak `PROCESS_KILL` = `SIGKILL`).
pub fn status_of(code: i64, sent: Option<c_int>) -> c_int {
    let signaled = |sig: c_int| sig & 0x7f;
    match code {
        EXIT_KILLED => signaled(sent.unwrap_or(SIGKILL)),
        EXIT_PANIC => signaled(SIGABRT),
        // Posledni vlakno skoncilo (pthread_exit v main): POSIX = exit(0).
        EXIT_NO_THREADS => 0,
        c if (EXIT_FAULT_BASE - 255..=EXIT_FAULT_BASE).contains(&c) => signaled(fault_signal((EXIT_FAULT_BASE - c) as u32)),
        c => ((c & 0xff) as c_int) << 8,
    }
}

/// Signal pro vyjimku CPU (cislo vektoru jako v jadre).
fn fault_signal(vector: u32) -> c_int {
    if cfg!(target_arch = "x86_64") {
        match vector {
            0 | 16 | 19 => SIGFPE,
            1 | 3 => SIGTRAP,
            6 => SIGILL,
            17 => crate::signal::SIGBUS,
            _ => SIGSEGV,
        }
    } else {
        SIGSEGV
    }
}

struct Entry {
    pid: c_int,
    child: Arc<Child>,
    /// Signal z `kill` (0 = zadny).
    sent: AtomicI32,
}

static CHILDREN: Mutex<Vec<Arc<Entry>>> = Mutex::new(Vec::new());
static NEXT_PID: AtomicI32 = AtomicI32::new(0);

fn table() -> std::sync::MutexGuard<'static, Vec<Arc<Entry>>> {
    CHILDREN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Zaradi potomka, vraci jeho cislo (od `getpid() + 1`, nikdy 0, 1 ani
/// cislo procesu).
pub fn register(c: Child) -> c_int {
    let me = crate::signal::getpid();
    let _ = NEXT_PID.compare_exchange(0, me.wrapping_add(1).max(2), Ordering::AcqRel, Ordering::Acquire);
    let pid = loop {
        let p = NEXT_PID.fetch_add(1, Ordering::AcqRel);
        if p > 1 && p != me {
            break p;
        }
        let _ = NEXT_PID.compare_exchange(p.wrapping_add(1), 2, Ordering::AcqRel, Ordering::Acquire);
    };
    table().push(Arc::new(Entry { pid, child: Arc::new(c), sent: AtomicI32::new(0) }));
    pid
}

/// Potomci, na ktere `pid` ceka (`-1`, `0`, `< -1` = vsichni).
fn matching(pid: c_int) -> Vec<Arc<Entry>> {
    table().iter().filter(|e| pid <= 0 || e.pid == pid).cloned().collect()
}

fn reap(pid: c_int) {
    table().retain(|e| e.pid != pid);
}

/// Jak dlouho cekat v jednom kole na vic potomku: novy potomek z jineho
/// vlakna se pak do cekani dostane nejpozdeji po tomto case.
const ROUND_NS: u64 = 50_000_000;

/// Jadro `waitpid`: (pid, kod konce, signal z kill), `None` = WNOHANG a nikdo
/// jeste neskoncil.
pub fn wait_for(pid: c_int, options: c_int) -> Result<Option<(c_int, i64, Option<c_int>)>, c_int> {
    if options & !(WNOHANG | WUNTRACED | WCONTINUED | WNOWAIT | WEXITED) != 0 {
        return Err(errno::EINVAL);
    }
    loop {
        let list = matching(pid);
        if list.is_empty() {
            return Err(errno::ECHILD);
        }
        for e in &list {
            if let Some(code) = e.child.try_exit()? {
                if options & WNOWAIT == 0 {
                    reap(e.pid);
                }
                let sent = e.sent.load(Ordering::Acquire);
                return Ok(Some((e.pid, code, (sent != 0).then_some(sent))));
            }
        }
        if options & WNOHANG != 0 {
            return Ok(None);
        }
        crate::pthread::testcancel();
        if list.len() == 1 && pid > 0 {
            list[0].child.wait_exit()?;
        } else {
            let refs: Vec<&Child> = list.iter().take(64).map(|e| &*e.child).collect();
            sys::wait_children(&refs, Some(ROUND_NS))?;
        }
    }
}

/// # Safety
///
/// `status` je NULL nebo platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int {
    match wait_for(pid, options) {
        Ok(Some((p, code, sent))) => {
            // SAFETY: kontrakt C.
            if let Some(s) = unsafe { status.as_mut() } {
                *s = status_of(code, sent);
            }
            p
        }
        Ok(None) => 0,
        Err(e) => errno::ret(Err(e)),
    }
}

/// # Safety
///
/// `status` je NULL nebo platny ukazatel.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wait(status: *mut c_int) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { waitpid(-1, status, 0) }
}

/// `wait4`: jako `waitpid`, `struct rusage` vynulovana (cas CPU jadro
/// zatim nedava).
///
/// # Safety
///
/// `status` a `ru` jsou NULL nebo platne ukazatele (`ru` = `struct rusage`).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wait4(pid: c_int, status: *mut c_int, options: c_int, ru: *mut u8) -> c_int {
    // SAFETY: kontrakt C.
    let r = unsafe { waitpid(pid, status, options) };
    if r > 0 && !ru.is_null() {
        // SAFETY: struct rusage ma 144 B (sys/resource.h).
        unsafe { core::ptr::write_bytes(ru, 0, 144) };
    }
    r
}

/// # Safety
///
/// Jako `wait4`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn wait3(status: *mut c_int, options: c_int, ru: *mut u8) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { wait4(-1, status, options, ru) }
}

/// `siginfo_t` pro `waitid` (128 B, pole jako Linux).
#[repr(C)]
pub struct WaitInfo {
    pub si_signo: c_int,
    pub si_errno: c_int,
    pub si_code: c_int,
    _pad: c_int,
    pub si_pid: c_int,
    pub si_uid: u32,
    pub si_status: c_int,
    _rest: [u8; 100],
}

pub const CLD_EXITED: c_int = 1;
pub const CLD_KILLED: c_int = 2;
pub const CLD_DUMPED: c_int = 3;

/// # Safety
///
/// `info` je NULL nebo platny `siginfo_t`.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn waitid(idtype: c_int, id: u32, info: *mut WaitInfo, options: c_int) -> c_int {
    let pid = match idtype {
        P_ALL | P_PGID => -1,
        P_PID => id as c_int,
        _ => return errno::ret(Err(errno::EINVAL)),
    };
    if options & (WEXITED | WSTOPPED | WCONTINUED) == 0 {
        return errno::ret(Err(errno::EINVAL));
    }
    match wait_for(pid, options & (WNOHANG | WNOWAIT)) {
        Ok(r) => {
            // SAFETY: kontrakt C.
            if let Some(i) = unsafe { info.as_mut() } {
                // SAFETY: siginfo_t ma 128 B, nuly = zadny potomek (WNOHANG).
                unsafe { core::ptr::write_bytes((i as *mut WaitInfo).cast::<u8>(), 0, size_of::<WaitInfo>()) };
                if let Some((p, code, sent)) = r {
                    let st = status_of(code, sent);
                    i.si_signo = crate::signal::SIGCHLD;
                    i.si_pid = p;
                    if st & 0x7f == 0 {
                        i.si_code = CLD_EXITED;
                        i.si_status = (st >> 8) & 0xff;
                    } else {
                        i.si_code = CLD_KILLED;
                        i.si_status = st & 0x7f;
                    }
                }
            }
            0
        }
        Err(e) => errno::ret(Err(e)),
    }
}

/// `kill` na potomka: signaly s vychozi akci konec (`SIGKILL`, `SIGTERM`,
/// `SIGINT`, `SIGHUP`, `SIGQUIT`) potomka ukonci hned (`PROCESS_KILL`, bez
/// jeho obsluhy signalu); `waitpid` pak hlasi odeslany signal. Signal 0 =
/// jen kontrola. Ostatni signaly (doruceni do obsluhy potomka) zatim
/// nejdou: `ENOTSUP`. `None` = `pid` neni potomek.
pub fn kill_child(pid: c_int, sig: c_int) -> Option<Result<(), c_int>> {
    let e = table().iter().find(|e| e.pid == pid).cloned()?;
    Some(match sig {
        0 => Ok(()),
        SIGKILL | SIGTERM | SIGINT | SIGHUP | SIGQUIT => {
            if e.child.try_exit().ok().flatten().is_some() {
                // Uz skoncil (zombie): POSIX kill uspeje, stav se nemeni.
                return Some(Ok(()));
            }
            let _ = e.sent.compare_exchange(0, sig, Ordering::AcqRel, Ordering::Acquire);
            e.child.kill()
        }
        _ => Err(errno::ENOTSUP),
    })
}

