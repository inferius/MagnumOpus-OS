//! Preruseni vlakna (`THREAD_INTERRUPT`): doruceni na obsluhu ve vlakne
//! (druh `INTERRUPT` s kodem), blokujici cekani vrati `INTERRUPTED`, kody
//! od nejnizsiho jeden po druhem, chybne argumenty a proces bez obsluhy.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use core::time::Duration;

use libos::abi::exception::{action, kind, stage};
use libos::exception::{clear_handler, frame_report, handler_return, set_handler};
use libos::{Error, Startup, sys, time};

use super::{Test, TestResult, fail};
use crate::check_eq;

pub const TESTS: &[Test] = &[
    ("preruseni: chybne argumenty a proces bez obsluhy", bad_args),
    ("preruseni: blokujici futex vrati INTERRUPTED", blocked_wait),
    ("preruseni: bezici vlakno, kody od nejnizsiho, stejne se slouci", order_and_merge),
];

/// Kody videne obsluhou (az 8), pocet doruceni a chyby ramce.
static SEEN: [AtomicU64; 8] = [const { AtomicU64::new(u64::MAX) }; 8];
static COUNT: AtomicU64 = AtomicU64::new(0);
static BAD: AtomicU64 = AtomicU64::new(0);
/// Futex, na kterem ceka testovane vlakno (nikdo ho nebudi).
static WORD: AtomicU32 = AtomicU32::new(0);
/// Vysledek vlakna: 1 = futex vratil INTERRUPTED, 2 = jina chyba/uspech.
static RESULT: AtomicU64 = AtomicU64::new(0);
static STOP: AtomicU64 = AtomicU64::new(0);

fn deadline(ms: u64) -> u64 {
    time::deadline_after(Duration::from_millis(ms))
}

extern "C" fn handler(f: *mut u8) -> ! {
    // SAFETY: `f` je ramec, ktery dalo jadro.
    let r = unsafe { frame_report(f) };
    if r.kind == kind::INTERRUPT && r.stage == stage::IN_THREAD {
        let i = COUNT.fetch_add(1, Ordering::SeqCst) as usize;
        if i < SEEN.len() {
            SEEN[i].store(r.code, Ordering::SeqCst);
        }
    } else {
        BAD.fetch_add(1, Ordering::SeqCst);
    }
    // SAFETY: ramec obsluhy.
    let e = unsafe { handler_return(f, action::RESUME) };
    BAD.store(e as u64 | 0x1000, Ordering::SeqCst);
    sys::thread_exit()
}

fn reset() -> TestResult {
    for s in &SEEN {
        s.store(u64::MAX, Ordering::SeqCst);
    }
    COUNT.store(0, Ordering::SeqCst);
    BAD.store(0, Ordering::SeqCst);
    RESULT.store(0, Ordering::SeqCst);
    STOP.store(0, Ordering::SeqCst);
    set_handler(handler).map_err(|e| fail("SET_ENTRY", e))
}

fn bad_args(_: &Startup) -> TestResult {
    let _ = clear_handler();
    let w = libos::thread::spawn("ceka", || {
        while STOP.load(Ordering::SeqCst) == 0 {
            let _ = sys::futex_wait(&WORD, 0, deadline(10));
        }
    })
    .map_err(|e| fail("vlakno", e))?;
    STOP.store(0, Ordering::SeqCst);
    let no_handler = w.thread().interrupt(1).err();
    reset()?;
    let big = w.thread().interrupt(64).err();
    let _ = clear_handler();
    STOP.store(1, Ordering::SeqCst);
    w.join(Some(deadline(5000))).map_err(|e| fail("vlakno", e))?;
    check_eq!(no_handler, Some(Error::BadState), "proces bez obsluhy ve vlakne");
    check_eq!(big, Some(Error::InvalidArgs), "kod 64");
    Ok(())
}

fn blocked_wait(_: &Startup) -> TestResult {
    reset()?;
    let w = libos::thread::spawn("futex", || {
        let r = sys::futex_wait(&WORD, 0, deadline(5000));
        RESULT.store(if r == Err(Error::Interrupted) { 1 } else { 2 }, Ordering::SeqCst);
    })
    .map_err(|e| fail("vlakno", e))?;
    // Vlakno usne ve futexu (nikdo ho nebudi, termin 5 s).
    time::sleep(Duration::from_millis(50));
    let sent = w.thread().interrupt(7);
    let r = w.join(Some(deadline(3000)));
    let _ = clear_handler();
    sent.map_err(|e| fail("THREAD_INTERRUPT", e))?;
    r.map_err(|e| fail("vlakno skoncilo driv nez termin futexu", e))?;
    check_eq!(RESULT.load(Ordering::SeqCst), 1, "futex vratil INTERRUPTED");
    check_eq!(COUNT.load(Ordering::SeqCst), 1, "obsluha jednou");
    check_eq!(SEEN[0].load(Ordering::SeqCst), 7, "kod v Report::code");
    check_eq!(BAD.load(Ordering::SeqCst), 0, "druh a stupen ramce");
    Ok(())
}

fn order_and_merge(_: &Startup) -> TestResult {
    reset()?;
    let w = libos::thread::spawn("bezi", || {
        // Bez syscallu: doruceni po preruseni z userspace (IPI, casovac).
        while COUNT.load(Ordering::SeqCst) < 2 && STOP.load(Ordering::SeqCst) == 0 {
            core::hint::spin_loop();
        }
    })
    .map_err(|e| fail("vlakno", e))?;
    let t = w.thread();
    let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
    let stopped = t.wait_suspended(Some(deadline(3000)));
    let sent = [t.interrupt(5), t.interrupt(3), t.interrupt(3)];
    drop(token);
    // Kontrola konce: bez doruceni by vlakno toclo dal, zastavit ho.
    let mut r = w.join(Some(deadline(3000)));
    if r.is_err() {
        STOP.store(1, Ordering::SeqCst);
        r = Err(Error::TimedOut);
    }
    let _ = clear_handler();
    stopped.map_err(|e| fail("pozastaveni", e))?;
    for s in sent {
        s.map_err(|e| fail("THREAD_INTERRUPT", e))?;
    }
    r.map_err(|e| fail("vlakno nedostalo preruseni", e))?;
    check_eq!(COUNT.load(Ordering::SeqCst), 2, "dve doruceni (3 se sloucila)");
    check_eq!(SEEN[0].load(Ordering::SeqCst), 3, "nejnizsi kod prvni");
    check_eq!(SEEN[1].load(Ordering::SeqCst), 5, "pak dalsi");
    check_eq!(BAD.load(Ordering::SeqCst), 0, "druh a stupen ramce");
    Ok(())
}
