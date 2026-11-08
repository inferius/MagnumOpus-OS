//! Vlakna, odlozena prace a cas.
//!
//! `AcpiOsExecute` spousti obsluhy mimo volajici vlakno (oznameni `Notify`,
//! metody GPE). Dve fronty, kazda s vlastnim vlaknem: oznameni zvlast od
//! GPE a ostatniho - obsluha oznameni smi cekat na dokonceni prace GPE
//! (stejne rozdeleni jako Linux, jinak hrozi uviznuti).
//!
//! ID vlakna: libos nema TLS, ID se proto pozna podle zasobniku. Kazde
//! vlakno, ktere vola ACPICA, se zaregistruje ([`register_thread`]) s adresou
//! vrcholu sveho zasobniku; ID je poradi registrace (od 1).

use alloc::collections::VecDeque;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::time::Duration;

use libos::{Event, Signals, time, wait_one};
use spin::Mutex;

use super::{AE_BAD_PARAMETER, AE_OK, Status};
use acpica::ffi::OsdExecCallback;

/// Vrcholy zasobniku registrovanych vlaken (index + 1 = ID).
const MAX_THREADS: usize = 16;
static STACK_TOPS: [AtomicU64; MAX_THREADS] = [const { AtomicU64::new(0) }; MAX_THREADS];
static THREADS: AtomicU32 = AtomicU32::new(0);
/// Zasobnik vlakna i programu ma 256 KiB (`libos::thread::STACK_SIZE`).
const STACK: u64 = libos::thread::STACK_SIZE;

fn stack_pointer() -> u64 {
    let x = 0u8;
    &x as *const u8 as u64
}

/// Zaregistruje volajici vlakno (volat hned na zacatku vlakna, mimo hluboke
/// vnoreni).
pub fn register_thread() {
    let i = THREADS.fetch_add(1, Ordering::AcqRel) as usize;
    if i < MAX_THREADS {
        STACK_TOPS[i].store(stack_pointer() + 512, Ordering::Release);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetThreadId() -> u64 {
    let sp = stack_pointer();
    let n = (THREADS.load(Ordering::Acquire) as usize).min(MAX_THREADS);
    for (i, top) in STACK_TOPS[..n].iter().enumerate() {
        let top = top.load(Ordering::Acquire);
        if sp <= top && sp + STACK > top {
            return i as u64 + 1;
        }
    }
    // Neregistrovane vlakno (nemelo by nastat): spolecne ID mimo rozsah.
    (MAX_THREADS + 1) as u64
}

struct Job {
    f: OsdExecCallback,
    ctx: usize,
}

struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    /// Signalizovany, kdyz fronta neni prazdna.
    ready: Event,
    /// Prace ve fronte nebo prave bezici.
    pending: AtomicU32,
}

static QUEUES: spin::Once<[Queue; 2]> = spin::Once::new();

fn worker(q: &'static Queue) {
    register_thread();
    loop {
        let job = {
            let mut jobs = q.jobs.lock();
            let j = jobs.pop_front();
            if jobs.is_empty() {
                let _ = q.ready.reset();
            }
            j
        };
        match job {
            Some(j) => {
                // SAFETY: funkce a kontext od ACPICA (AcpiOsExecute).
                unsafe { (j.f)(j.ctx as *mut c_void) };
                q.pending.fetch_sub(1, Ordering::AcqRel);
            }
            None => {
                let _ = wait_one(q.ready.handle(), Signals::SIGNALED, None);
            }
        }
    }
}

/// Spusti vlakna front (pred inicializaci ACPICA).
pub fn start_workers() -> libos::Result<()> {
    let mk = || -> libos::Result<Queue> {
        Ok(Queue { jobs: Mutex::new(VecDeque::new()), ready: Event::create()?, pending: AtomicU32::new(0) })
    };
    let queues = [mk()?, mk()?];
    let q = QUEUES.call_once(|| queues);
    for (i, name) in ["acpid-notify", "acpid-gpe"].into_iter().enumerate() {
        let q: &'static Queue = &q[i];
        // JoinHandle se zahodi: vlakna bezi do konce procesu.
        core::mem::forget(libos::thread::spawn(name, move || worker(q))?);
    }
    Ok(())
}

/// Typ `OSL_NOTIFY_HANDLER` z `acpiosxf.h`.
const OSL_NOTIFY_HANDLER: u32 = 1;

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsExecute(kind: u32, f: Option<OsdExecCallback>, ctx: *mut c_void) -> Status {
    let (Some(f), Some(qs)) = (f, QUEUES.get()) else {
        return AE_BAD_PARAMETER;
    };
    let q = &qs[if kind == OSL_NOTIFY_HANDLER { 0 } else { 1 }];
    q.pending.fetch_add(1, Ordering::AcqRel);
    q.jobs.lock().push_back(Job { f, ctx: ctx as usize });
    let _ = q.ready.signal();
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWaitEventsComplete() {
    let Some(qs) = QUEUES.get() else { return };
    while qs.iter().any(|q| q.pending.load(Ordering::Acquire) != 0) {
        time::sleep(Duration::from_millis(1));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsSleep(ms: u64) {
    time::sleep(Duration::from_millis(ms));
}

/// Kratke cekani (AML `Stall`, nejvys desitky mikrosekund).
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsStall(us: u32) {
    let end = time::now() + us as u64 * 1000;
    while time::now() < end {
        core::hint::spin_loop();
    }
}

/// Cas v jednotkach 100 ns (AML `Timer`).
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetTimer() -> u64 {
    time::now() / 100
}
