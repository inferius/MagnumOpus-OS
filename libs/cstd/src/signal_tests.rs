//! Testy signalu na hostiteli: raise, maska a cekajici signaly,
//! SA_RESETHAND, casovac (vlakno signalu), mnoziny.

use core::ffi::c_int;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::signal::*;

static GOT_USR1: AtomicI32 = AtomicI32::new(0);
static GOT_USR2: AtomicI32 = AtomicI32::new(0);
static GOT_ALRM: AtomicI32 = AtomicI32::new(0);

extern "C" fn on_usr1(s: c_int) {
    GOT_USR1.fetch_add(s, Ordering::SeqCst);
}

extern "C" fn on_usr2(s: c_int) {
    GOT_USR2.fetch_add(s, Ordering::SeqCst);
}

extern "C" fn on_alrm(_s: c_int) {
    GOT_ALRM.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn raise_runs_handler_and_mask_defers() {
    assert_eq!(signal(SIGUSR1, on_usr1 as extern "C" fn(c_int) as usize), SIG_DFL);
    assert_eq!(raise(SIGUSR1), 0);
    assert_eq!(GOT_USR1.load(Ordering::SeqCst), SIGUSR1);
    // Blokovany signal ceka, odblokovani ho doruci.
    let mut set = [0u64; 16];
    let mut pend = [0u64; 16];
    // SAFETY: platne ukazatele.
    unsafe {
        sigemptyset(&mut set);
        sigaddset(&mut set, SIGUSR1);
        assert_eq!(sigismember(&set, SIGUSR1), 1);
        sigprocmask(SIG_BLOCK, &set, core::ptr::null_mut());
        raise(SIGUSR1);
        sigpending(&mut pend);
        assert_eq!(sigismember(&pend, SIGUSR1), 1);
        assert_eq!(GOT_USR1.load(Ordering::SeqCst), SIGUSR1, "jeste nedoruceno");
        sigprocmask(SIG_UNBLOCK, &set, core::ptr::null_mut());
    }
    assert_eq!(GOT_USR1.load(Ordering::SeqCst), 2 * SIGUSR1);
    assert_eq!(kill(getpid(), SIGUSR1), 0);
    assert_eq!(GOT_USR1.load(Ordering::SeqCst), 3 * SIGUSR1);
    assert!(kill(getpid() + 1, SIGUSR1) < 0, "cizi proces neni");
}

#[test]
fn resethand_and_ignore() {
    let act = sigaction { sa_handler: on_usr2 as extern "C" fn(c_int) as usize, sa_mask: [0; 16], sa_flags: SA_RESETHAND, sa_restorer: 0 };
    // SAFETY: platne ukazatele.
    unsafe { assert_eq!(sigaction(SIGUSR2, &act, core::ptr::null_mut()), 0) };
    raise(SIGUSR2);
    assert_eq!(GOT_USR2.load(Ordering::SeqCst), SIGUSR2);
    // Po SA_RESETHAND zpet SIG_DFL; ignorovat, at test nekonci proces.
    assert_eq!(signal(SIGUSR2, SIG_IGN), SIG_DFL);
    raise(SIGUSR2);
    assert_eq!(GOT_USR2.load(Ordering::SeqCst), SIGUSR2);
    assert_eq!(signal(SIGKILL, SIG_IGN), SIG_ERR, "SIGKILL nejde zmenit");
}

#[test]
fn timer_delivers_in_signal_thread() {
    signal(SIGALRM, on_alrm as extern "C" fn(c_int) as usize);
    let t = itimerval { it_interval: [0, 0], it_value: [0, 20_000] };
    // SAFETY: platne ukazatele.
    unsafe { assert_eq!(setitimer(ITIMER_REAL, &t, core::ptr::null_mut()), 0) };
    let start = std::time::Instant::now();
    while GOT_ALRM.load(Ordering::SeqCst) == 0 && start.elapsed().as_secs() < 5 {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(GOT_ALRM.load(Ordering::SeqCst), 1);
    assert!(start.elapsed().as_millis() >= 15);
    // alarm(0) zrusi, vraci zbytek (zadny).
    assert_eq!(alarm(0), 0);
}
