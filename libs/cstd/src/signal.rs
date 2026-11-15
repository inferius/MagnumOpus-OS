//! `signal.h` (rozhodnuti 6A): signaly bez podpory jadra.
//!
//! - **Synchronni** (`SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`, `SIGTRAP`):
//!   obsluha vyjimky ve vlakne (osrt::exception) zavola obsluhu signalu
//!   v tomtez vlakne; navrat z obsluhy = pokracovat (instrukce se zopakuje,
//!   napr. po `mprotect`), `SIG_DFL` = dalsi obsluha (konec procesu).
//! - **`raise`, `kill` na sebe**: obsluha bezi hned ve volajicim vlakne.
//! - **Asynchronni** (`alarm`, `setitimer`): obsluha bezi ve zvlastnim
//!   vlakne casovace (jako obsluha Ctrl+C konzole ve Windows); `EINTR`
//!   nevznika, `pause`/`sigsuspend` se probudi.
//! - Signaly od jinych procesu (terminal, rodic) zatim nejsou (blok
//!   parametru `SIGNAL` [navrzeno]); `SIGPIPE` se nevytvari (zapis vrati
//!   `EPIPE`).

use core::ffi::c_void;
use core::time::Duration;
use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::{c_char, c_int, errno};

pub const NSIG: usize = 65;
pub const SIGHUP: c_int = 1;
pub const SIGINT: c_int = 2;
pub const SIGQUIT: c_int = 3;
pub const SIGILL: c_int = 4;
pub const SIGTRAP: c_int = 5;
pub const SIGABRT: c_int = 6;
pub const SIGBUS: c_int = 7;
pub const SIGFPE: c_int = 8;
pub const SIGKILL: c_int = 9;
pub const SIGUSR1: c_int = 10;
pub const SIGSEGV: c_int = 11;
pub const SIGUSR2: c_int = 12;
pub const SIGPIPE: c_int = 13;
pub const SIGALRM: c_int = 14;
pub const SIGTERM: c_int = 15;
pub const SIGCHLD: c_int = 17;
pub const SIGCONT: c_int = 18;
pub const SIGSTOP: c_int = 19;
pub const SIGTSTP: c_int = 20;
pub const SIGTTIN: c_int = 21;
pub const SIGTTOU: c_int = 22;
pub const SIGURG: c_int = 23;
/// Casovace virtualniho a profilovaciho casu (`setitimer`) zatim nejsou;
/// cisla jsou kvuli maskam signalu v portech (SDL). Vychozi akce = konec.
pub const SIGVTALRM: c_int = 26;
pub const SIGPROF: c_int = 27;
pub const SIGWINCH: c_int = 28;

pub const SA_SIGINFO: c_int = 4;
pub const SA_ONSTACK: c_int = 0x0800_0000;
pub const SA_RESTART: c_int = 0x1000_0000;
pub const SA_NODEFER: c_int = 0x4000_0000;
pub const SA_RESETHAND: c_int = 0x8000_0000_u32 as c_int;
pub const SIG_BLOCK: c_int = 0;
pub const SIG_UNBLOCK: c_int = 1;
pub const SIG_SETMASK: c_int = 2;
pub const SI_USER: c_int = 0;
pub const SI_TIMER: c_int = -2;

/// `SIG_DFL` = 0, `SIG_IGN` = 1, `SIG_ERR` = -1.
pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;
pub const SIG_ERR: usize = usize::MAX;

/// `sigset_t` (1024 bitu jako glibc).
pub type Set = [u64; 16];

/// `siginfo_t` (128 B, prvni pole jako Linux; `si_addr` na offsetu 16).
#[repr(C)]
pub struct siginfo_t {
    pub si_signo: c_int,
    pub si_errno: c_int,
    pub si_code: c_int,
    _pad: c_int,
    pub si_addr: *mut c_void,
    _rest: [u64; 13],
}

/// `struct sigaction` (rozlozeni glibc x86_64, stejne na obou cilech).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct sigaction {
    pub sa_handler: usize,
    pub sa_mask: Set,
    pub sa_flags: c_int,
    pub sa_restorer: usize,
}

const EMPTY: sigaction = sigaction { sa_handler: SIG_DFL, sa_mask: [0; 16], sa_flags: 0, sa_restorer: 0 };

static ACTIONS: Mutex<[sigaction; NSIG]> = Mutex::new([EMPTY; NSIG]);
/// Cekajici signaly procesu (bity 1..64) - blokovane ve vsech vlaknech.
static PENDING: AtomicU64 = AtomicU64::new(0);
/// Citac doruceni (probouzi `pause`, `sigsuspend`).
static DELIVERED: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static MASK: Cell<u64> = const { Cell::new(0) };
}

fn bit(sig: c_int) -> u64 {
    1u64 << (sig - 1)
}

fn valid(sig: c_int) -> bool {
    (1..NSIG as c_int).contains(&sig)
}

/// Maska aktualniho vlakna (pro `sigsetjmp`).
pub fn current_mask() -> Set {
    let mut s = [0u64; 16];
    s[0] = MASK.with(Cell::get);
    s
}

/// Nastavi masku vlakna a doruci cekajici odblokovane signaly.
pub fn set_mask(s: Set) {
    MASK.with(|m| m.set(s[0] & !(bit(SIGKILL) | bit(SIGSTOP))));
    deliver_pending();
}

fn deliver_pending() {
    loop {
        let mask = MASK.with(Cell::get);
        let p = PENDING.load(Ordering::Acquire);
        let ready = p & !mask;
        if ready == 0 {
            return;
        }
        let sig = ready.trailing_zeros() as c_int + 1;
        if PENDING.compare_exchange(p, p & !bit(sig), Ordering::AcqRel, Ordering::Acquire).is_ok() {
            deliver(sig, SI_USER, core::ptr::null_mut());
        }
    }
}

/// Vychozi akce: konec procesu, nebo nic (`SIGCHLD`, `SIGCONT`, ...).
fn default_action(sig: c_int) {
    match sig {
        SIGCHLD | SIGCONT | SIGURG | SIGWINCH | SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => {}
        SIGABRT => crate::stdlib::abort(),
        _ => {
            // Konec "signalem": kod 128 + cislo (jako shell).
            crate::stdlib::_Exit(128 + sig)
        }
    }
}

/// Doruci signal v aktualnim vlakne (obsluha, ignorovani, vychozi akce).
pub fn deliver(sig: c_int, code: c_int, addr: *mut c_void) {
    let act = {
        let mut a = ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
        let act = a[sig as usize];
        if act.sa_flags & SA_RESETHAND != 0 && act.sa_handler > SIG_IGN {
            a[sig as usize] = EMPTY;
        }
        act
    };
    DELIVERED.fetch_add(1, Ordering::AcqRel);
    crate::sys::futex_wake(&DELIVERED, true);
    match act.sa_handler {
        SIG_IGN => {}
        SIG_DFL => default_action(sig),
        h => {
            // Behem obsluhy blokovat sa_mask a sam signal (bez SA_NODEFER).
            let old = MASK.with(Cell::get);
            let mut block = act.sa_mask[0];
            if act.sa_flags & SA_NODEFER == 0 {
                block |= bit(sig);
            }
            MASK.with(|m| m.set(old | block));
            if act.sa_flags & SA_SIGINFO != 0 {
                let mut info = siginfo_t { si_signo: sig, si_errno: 0, si_code: code, _pad: 0, si_addr: addr, _rest: [0; 13] };
                // Kontext (ucontext_t) zatim nevyplneny: nulovy buffer.
                let mut uc = [0u64; 128];
                // SAFETY: obsluha registrovana pres sigaction s SA_SIGINFO.
                let f: extern "C" fn(c_int, *mut siginfo_t, *mut c_void) = unsafe { core::mem::transmute(h) };
                f(sig, &mut info, uc.as_mut_ptr().cast());
            } else {
                // SAFETY: obsluha registrovana pres sigaction/signal.
                let f: extern "C" fn(c_int) = unsafe { core::mem::transmute(h) };
                f(sig);
            }
            MASK.with(|m| m.set(old));
        }
    }
}

/// Signal od programu (`raise`, `kill` na sebe, casovac): blokovany ceka.
fn send(sig: c_int, code: c_int) {
    if MASK.with(Cell::get) & bit(sig) != 0 && sig != SIGKILL && sig != SIGSTOP {
        PENDING.fetch_or(bit(sig), Ordering::AcqRel);
        return;
    }
    if sig == SIGKILL {
        crate::stdlib::_Exit(128 + SIGKILL);
    }
    deliver(sig, code, core::ptr::null_mut());
}

// --- synchronni signaly z vyjimek -------------------------------------------

#[cfg(target_os = "magnumopus")]
mod sync {
    use std::os::magnumopus::runtime::exception as ex;

    use super::*;

    extern "C" fn on_exception(frame: *mut u8) -> ! {
        // SAFETY: ramec od jadra.
        let f = unsafe { ex::Frame::from_raw(frame) };
        let r = f.report();
        let sig = match ex::signal_class_of(&r) {
            ex::SignalClass::Segv => SIGSEGV,
            ex::SignalClass::Bus => SIGBUS,
            ex::SignalClass::Fpe => SIGFPE,
            ex::SignalClass::Ill => SIGILL,
            ex::SignalClass::Trap => SIGTRAP,
        };
        let h = ACTIONS.lock().unwrap_or_else(|e| e.into_inner())[sig as usize].sa_handler;
        if h == SIG_DFL || h == SIG_IGN || MASK.with(Cell::get) & bit(sig) != 0 {
            // Bez obsluhy: dalsi v poradi (nakonec konec procesu s hlaskou).
            let _ = f.next();
            crate::stdlib::_Exit(128 + sig);
        }
        // si_code: SEGV_MAPERR = 1 (adresa), BUS_ADRALN = 1, FPE_INTDIV = 1.
        deliver(sig, 1, r.addr as *mut c_void);
        let _ = f.resume();
        crate::stdlib::_Exit(128 + sig)
    }

    /// Zaregistruje obsluhu vyjimek procesu (jednou).
    pub fn install() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = ex::set_handler(on_exception);
        });
    }

    pub fn alt_stack(base: u64, size: u64) -> bool {
        ex::set_alt_stack(base, size).is_ok()
    }

    pub fn min_alt() -> usize {
        ex::min_alt_stack()
    }
}

#[cfg(not(target_os = "magnumopus"))]
mod sync {
    pub fn install() {}
    pub fn alt_stack(_b: u64, _s: u64) -> bool {
        true
    }
    pub fn min_alt() -> usize {
        8192
    }
}

fn is_sync(sig: c_int) -> bool {
    matches!(sig, SIGSEGV | SIGBUS | SIGFPE | SIGILL | SIGTRAP)
}

// --- API C --------------------------------------------------------------------

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigaction(sig: c_int, act: *const sigaction, old: *mut sigaction) -> c_int {
    if !valid(sig) {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    let new = unsafe { act.as_ref() }.copied();
    if new.is_some() && (sig == SIGKILL || sig == SIGSTOP) {
        return errno::ret(Err(errno::EINVAL));
    }
    let prev = {
        let mut a = ACTIONS.lock().unwrap_or_else(|e| e.into_inner());
        let prev = a[sig as usize];
        if let Some(n) = new {
            a[sig as usize] = n;
        }
        prev
    };
    if let Some(n) = new
        && is_sync(sig)
        && n.sa_handler > SIG_IGN
    {
        sync::install();
    }
    if !old.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *old = prev };
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn signal(sig: c_int, handler: usize) -> usize {
    let act = sigaction { sa_handler: handler, sa_mask: [0; 16], sa_flags: SA_RESTART, sa_restorer: 0 };
    let mut old = EMPTY;
    // SAFETY: platne ukazatele.
    if unsafe { sigaction(sig, &act, &mut old) } != 0 {
        return SIG_ERR;
    }
    old.sa_handler
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn raise(sig: c_int) -> c_int {
    if !valid(sig) {
        return errno::ret(Err(errno::EINVAL));
    }
    send(sig, SI_USER);
    0
}

/// Cislo procesu: jadro ho programum nedava, proto nahodne kladne cislo
/// pevne po dobu behu (`osrt::process::self_id`, totez co
/// `std::process::id`): jmena docasnych souboru, `kill` na sebe, `l_pid`
/// zamku a `SO_PEERCRED` ho vidi jine procesy stejne.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getpid() -> c_int {
    std::process::id() as c_int
}

/// Rodic: neznamy, jako proces osirely (1).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn getppid() -> c_int {
    1
}

/// `kill`: vlastni proces (`pid` 0, -1 nebo `getpid()`) a potomci
/// z `posix_spawn` (`spawn::wait::kill_child`: jen ukonceni).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn kill(pid: c_int, sig: c_int) -> c_int {
    if sig != 0 && !valid(sig) {
        return errno::ret(Err(errno::EINVAL));
    }
    if pid > 0 && pid != getpid() {
        if let Some(r) = crate::spawn::wait::kill_child(pid, sig) {
            return errno::ret(r);
        }
    }
    if pid != 0 && pid != -1 && pid != getpid() {
        return errno::ret(Err(errno::ESRCH));
    }
    if sig != 0 {
        send(sig, SI_USER);
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn pthread_sigmask(how: c_int, set: *const Set, old: *mut Set) -> c_int {
    let cur = current_mask();
    if !old.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *old = cur };
    }
    // SAFETY: kontrakt C.
    let Some(s) = (unsafe { set.as_ref() }) else { return 0 };
    let new = match how {
        SIG_BLOCK => cur[0] | s[0],
        SIG_UNBLOCK => cur[0] & !s[0],
        SIG_SETMASK => s[0],
        _ => return errno::EINVAL,
    };
    let mut n = [0u64; 16];
    n[0] = new;
    set_mask(n);
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigprocmask(how: c_int, set: *const Set, old: *mut Set) -> c_int {
    // SAFETY: kontrakt C.
    match unsafe { pthread_sigmask(how, set, old) } {
        0 => 0,
        e => errno::ret(Err(e)),
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigpending(set: *mut Set) -> c_int {
    let mut s = [0u64; 16];
    s[0] = PENDING.load(Ordering::Acquire);
    // SAFETY: kontrakt C.
    unsafe { *set = s };
    0
}

/// Ceka na doruceni signalu (vraci vzdy -1 s `EINTR`).
fn wait_delivery(seen: u32) -> c_int {
    while DELIVERED.load(Ordering::Acquire) == seen {
        crate::sys::futex_wait(&DELIVERED, seen, None);
    }
    errno::ret(Err(errno::EINTR))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn pause() -> c_int {
    wait_delivery(DELIVERED.load(Ordering::Acquire))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigsuspend(set: *const Set) -> c_int {
    let seen = DELIVERED.load(Ordering::Acquire);
    let old = current_mask();
    // SAFETY: kontrakt C.
    if let Some(s) = unsafe { set.as_ref() } {
        set_mask(*s);
    }
    let r = if DELIVERED.load(Ordering::Acquire) != seen { errno::ret(Err(errno::EINTR)) } else { wait_delivery(seen) };
    set_mask(old);
    r
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigemptyset(s: *mut Set) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *s = [0; 16] };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigfillset(s: *mut Set) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { *s = [u64::MAX, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0] };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigaddset(s: *mut Set, sig: c_int) -> c_int {
    if !valid(sig) {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    unsafe { (*s)[0] |= bit(sig) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigdelset(s: *mut Set, sig: c_int) -> c_int {
    if !valid(sig) {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    unsafe { (*s)[0] &= !bit(sig) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigismember(s: *const Set, sig: c_int) -> c_int {
    if !valid(sig) {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    c_int::from(unsafe { (*s)[0] } & bit(sig) != 0)
}

/// `stack_t` pro `sigaltstack`.
#[repr(C)]
pub struct stack_t {
    pub ss_sp: *mut c_void,
    pub ss_flags: c_int,
    pub ss_size: usize,
}

pub const SS_DISABLE: c_int = 2;

thread_local! {
    static ALT: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sigaltstack(ss: *const stack_t, old: *mut stack_t) -> c_int {
    let (b, s) = ALT.with(Cell::get);
    if !old.is_null() {
        // SAFETY: kontrakt C.
        unsafe {
            *old = stack_t { ss_sp: b as *mut c_void, ss_flags: if s == 0 { SS_DISABLE } else { 0 }, ss_size: s };
        }
    }
    // SAFETY: kontrakt C.
    let Some(n) = (unsafe { ss.as_ref() }) else { return 0 };
    let (base, size) = if n.ss_flags & SS_DISABLE != 0 { (0, 0) } else { (n.ss_sp as usize, n.ss_size) };
    if size != 0 && size < sync::min_alt() {
        return errno::ret(Err(errno::ENOMEM));
    }
    if !sync::alt_stack(base as u64, size as u64) {
        return errno::ret(Err(errno::EINVAL));
    }
    ALT.with(|a| a.set((base, size)));
    0
}

// --- casovace (vlakno signalu) -------------------------------------------------

/// Stav casovace `ITIMER_REAL`: (termin v ns monotonniho casu, perioda ns);
/// 0 = vypnuto. Generace odlisi zruseny casovac.
static TIMER: Mutex<(u64, u64, u64)> = Mutex::new((0, 0, 0));
static TIMER_FUTEX: AtomicU32 = AtomicU32::new(0);

fn timer_thread() {
    loop {
        let (deadline, period, generation) = *TIMER.lock().unwrap_or_else(|e| e.into_inner());
        let seen = TIMER_FUTEX.load(Ordering::Acquire);
        if deadline == 0 {
            crate::sys::futex_wait(&TIMER_FUTEX, seen, None);
            continue;
        }
        let now = crate::sys::monotonic_ns();
        if now < deadline {
            crate::sys::futex_wait(&TIMER_FUTEX, seen, Some(Duration::from_nanos(deadline - now)));
            continue;
        }
        {
            let mut t = TIMER.lock().unwrap_or_else(|e| e.into_inner());
            if t.2 != generation {
                continue;
            }
            *t = if period == 0 { (0, 0, generation) } else { (deadline + period, period, generation) };
        }
        // Obsluha SIGALRM bezi v tomto vlakne (rozhodnuti 6A).
        send(SIGALRM, SI_TIMER);
    }
}

fn arm(deadline: u64, period: u64) -> (u64, u64) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("signaly".into()).stack_size(256 * 1024).spawn(timer_thread);
    });
    let mut t = TIMER.lock().unwrap_or_else(|e| e.into_inner());
    let now = crate::sys::monotonic_ns();
    let left = if t.0 == 0 { 0 } else { t.0.saturating_sub(now) };
    let old = (left, t.1);
    *t = (deadline, period, t.2.wrapping_add(1));
    drop(t);
    TIMER_FUTEX.fetch_add(1, Ordering::AcqRel);
    crate::sys::futex_wake(&TIMER_FUTEX, true);
    old
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn alarm(secs: u32) -> u32 {
    let deadline = if secs == 0 { 0 } else { crate::sys::monotonic_ns() + u64::from(secs) * 1_000_000_000 };
    let (left, _) = arm(deadline, 0);
    left.div_ceil(1_000_000_000) as u32
}

/// `struct itimerval` (dva `struct timeval`).
#[repr(C)]
pub struct itimerval {
    pub it_interval: [i64; 2],
    pub it_value: [i64; 2],
}

fn tv_ns(t: [i64; 2]) -> u64 {
    (t[0].max(0) as u64) * 1_000_000_000 + (t[1].max(0) as u64) * 1000
}

fn ns_tv(ns: u64) -> [i64; 2] {
    [(ns / 1_000_000_000) as i64, ((ns % 1_000_000_000) / 1000) as i64]
}

pub const ITIMER_REAL: c_int = 0;

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn setitimer(which: c_int, new: *const itimerval, old: *mut itimerval) -> c_int {
    if which != ITIMER_REAL {
        return errno::ret(Err(errno::EINVAL));
    }
    // SAFETY: kontrakt C.
    let Some(n) = (unsafe { new.as_ref() }) else { return errno::ret(Err(errno::EFAULT)) };
    let value = tv_ns(n.it_value);
    let deadline = if value == 0 { 0 } else { crate::sys::monotonic_ns() + value };
    let (left, period) = arm(deadline, tv_ns(n.it_interval));
    if !old.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *old = itimerval { it_interval: ns_tv(period), it_value: ns_tv(left) } };
    }
    0
}

// --- texty -------------------------------------------------------------------

pub fn name(sig: c_int) -> &'static [u8] {
    match sig {
        SIGHUP => b"Hangup\0",
        SIGINT => b"Interrupt\0",
        SIGQUIT => b"Quit\0",
        SIGILL => b"Illegal instruction\0",
        SIGTRAP => b"Trace/breakpoint trap\0",
        SIGABRT => b"Aborted\0",
        SIGBUS => b"Bus error\0",
        SIGFPE => b"Floating point exception\0",
        SIGKILL => b"Killed\0",
        SIGUSR1 => b"User defined signal 1\0",
        SIGSEGV => b"Segmentation fault\0",
        SIGUSR2 => b"User defined signal 2\0",
        SIGPIPE => b"Broken pipe\0",
        SIGALRM => b"Alarm clock\0",
        SIGTERM => b"Terminated\0",
        SIGCHLD => b"Child exited\0",
        _ => b"Unknown signal\0",
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn strsignal(sig: c_int) -> *const c_char {
    name(sig).as_ptr()
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn psignal(sig: c_int, s: *const c_char) {
    use std::io::Write;
    let mut e = std::io::stderr().lock();
    if !s.is_null() {
        // SAFETY: retezec C.
        let p = unsafe { crate::c_bytes(s) };
        if !p.is_empty() {
            let _ = e.write_all(p);
            let _ = e.write_all(b": ");
        }
    }
    let n = name(sig);
    let _ = e.write_all(&n[..n.len() - 1]);
    let _ = e.write_all(b"\n");
}
