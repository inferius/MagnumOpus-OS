//! Testy pthread na hostiteli (vlakna std, futex nahrazeny podminkou).

use core::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use crate::errno::{EBUSY, EDEADLK, EPERM, ETIMEDOUT};
use crate::pthread::*;
use crate::stat::timespec;

fn mutex(kind: i32) -> Box<pthread_mutex_t> {
    // SAFETY: nuly = staticky inicializator; pak init s typem.
    let mut m: Box<pthread_mutex_t> = Box::new(unsafe { core::mem::zeroed() });
    let mut a = pthread_mutexattr_t { kind: 0 };
    // SAFETY: platne ukazatele.
    unsafe {
        pthread_mutexattr_settype(&mut a, kind);
        assert_eq!(pthread_mutex_init(&mut *m, &a), 0);
    }
    m
}

#[test]
fn type_sizes_match_header() {
    assert_eq!(size_of::<pthread_mutex_t>(), 40);
    assert_eq!(size_of::<pthread_cond_t>(), 48);
    assert_eq!(size_of::<pthread_rwlock_t>(), 56);
    assert_eq!(size_of::<pthread_barrier_t>(), 32);
    assert_eq!(size_of::<pthread_attr_t>(), 64);
}

#[test]
fn mutex_kinds() {
    let mut r = mutex(PTHREAD_MUTEX_RECURSIVE);
    let mut e = mutex(PTHREAD_MUTEX_ERRORCHECK);
    // SAFETY: platne mutexy.
    unsafe {
        assert_eq!(pthread_mutex_lock(&mut *r), 0);
        assert_eq!(pthread_mutex_lock(&mut *r), 0, "rekurzivni podruhe");
        assert_eq!(pthread_mutex_unlock(&mut *r), 0);
        assert_eq!(pthread_mutex_unlock(&mut *r), 0);
        assert_eq!(pthread_mutex_unlock(&mut *r), EPERM, "uz neni vlastnik");
        assert_eq!(pthread_mutex_lock(&mut *e), 0);
        assert_eq!(pthread_mutex_lock(&mut *e), EDEADLK);
        assert_eq!(pthread_mutex_trylock(&mut *e), EDEADLK);
        assert_eq!(pthread_mutex_unlock(&mut *e), 0);
        assert_eq!(pthread_mutex_trylock(&mut *e), 0);
        assert_eq!(pthread_mutex_unlock(&mut *e), 0);
    }
}

struct Shared {
    m: pthread_mutex_t,
    c: pthread_cond_t,
    counter: AtomicUsize,
    ready: AtomicU32,
}

unsafe extern "C" fn worker(p: *mut c_void) -> *mut c_void {
    // SAFETY: test predava platny Shared.
    let s = unsafe { &mut *(p as *mut Shared) };
    for _ in 0..1000 {
        // SAFETY: platny mutex.
        unsafe {
            pthread_mutex_lock(&mut s.m);
            let v = s.counter.load(Ordering::Relaxed);
            s.counter.store(v + 1, Ordering::Relaxed);
            pthread_mutex_unlock(&mut s.m);
        }
    }
    // SAFETY: platny mutex a podminka.
    unsafe {
        pthread_mutex_lock(&mut s.m);
        s.ready.fetch_add(1, Ordering::Relaxed);
        pthread_cond_signal(&mut s.c);
        pthread_mutex_unlock(&mut s.m);
    }
    7 as *mut c_void
}

#[test]
fn threads_mutex_condition_join() {
    // SAFETY: nuly = staticke inicializatory.
    let mut s: Box<Shared> = Box::new(unsafe { core::mem::zeroed() });
    let mut ts = [core::ptr::null_mut(); 4];
    for t in &mut ts {
        // SAFETY: platne argumenty.
        assert_eq!(unsafe { pthread_create(t, core::ptr::null(), worker, (&mut *s as *mut Shared).cast()) }, 0);
    }
    // SAFETY: platny mutex a podminka.
    unsafe {
        pthread_mutex_lock(&mut s.m);
        while s.ready.load(Ordering::Relaxed) < 4 {
            pthread_cond_wait(&mut s.c, &mut s.m);
        }
        pthread_mutex_unlock(&mut s.m);
    }
    for t in ts {
        let mut r = core::ptr::null_mut();
        // SAFETY: vlakno z pthread_create.
        assert_eq!(unsafe { pthread_join(t, &mut r) }, 0);
        assert_eq!(r as usize, 7);
    }
    assert_eq!(s.counter.load(Ordering::Relaxed), 4000);
}

#[test]
fn timed_wait_expires() {
    // SAFETY: nuly = staticke inicializatory.
    let mut m: pthread_mutex_t = unsafe { core::mem::zeroed() };
    // SAFETY: nuly = staticke inicializatory.
    let mut c: pthread_cond_t = unsafe { core::mem::zeroed() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap();
    let abs = timespec { tv_sec: now.as_secs() as i64, tv_nsec: i64::from(now.subsec_nanos()) + 20_000_000 };
    let abs = timespec { tv_sec: abs.tv_sec + abs.tv_nsec / 1_000_000_000, tv_nsec: abs.tv_nsec % 1_000_000_000 };
    // SAFETY: platne objekty.
    unsafe {
        pthread_mutex_lock(&mut m);
        assert_eq!(pthread_cond_timedwait(&mut c, &mut m, &abs), ETIMEDOUT);
        assert_eq!(pthread_mutex_trylock(&mut m), EBUSY, "mutex je po navratu zamceny");
        pthread_mutex_unlock(&mut m);
    }
}

static ONCE_RUNS: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn once_fn() {
    ONCE_RUNS.fetch_add(1, Ordering::Relaxed);
}

#[test]
fn once_and_keys() {
    let o = AtomicU32::new(0);
    // SAFETY: platny once.
    unsafe {
        pthread_once(&o as *const _ as *mut _, once_fn);
        pthread_once(&o as *const _ as *mut _, once_fn);
    }
    assert_eq!(ONCE_RUNS.load(Ordering::Relaxed), 1);
    let mut k = 0u32;
    // SAFETY: platny ukazatel.
    assert_eq!(unsafe { pthread_key_create(&mut k, None) }, 0);
    assert!(pthread_getspecific(k).is_null());
    assert_eq!(pthread_setspecific(k, 42 as *const c_void), 0);
    assert_eq!(pthread_getspecific(k) as usize, 42);
    // Jine vlakno ma vlastni hodnotu.
    let other = std::thread::spawn(move || pthread_getspecific(k) as usize).join().unwrap();
    assert_eq!(other, 0);
    assert_eq!(pthread_key_delete(k), 0);
    assert!(pthread_getspecific(k).is_null(), "smazany klic");
}
