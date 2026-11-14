//! Doruceni vyjimek primo ve vlaknu (`EXCEPTION_HANDLER`): obsluha opravi
//! vypadek, podvrzeny ramec, NEXT na kanal procesu, preteceni zasobniku
//! (s alternativnim zasobnikem i bez nej), limit vnoreni.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use libos::abi::exception::{action, frame, kind, stage};
use libos::abi::process::EXIT_FAULT_BASE;
use libos::abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE, region};
use libos::exception::{
    Frame, SignalClass, alloc_alt_stack, clear_handler, frame_report, handler_return, receive, set_alt_stack,
    set_handler, signal_class,
};
use libos::{Error, Startup, time, vm};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("ve vlaknu: obsluha opravi vypadek a RESUME", fix_and_resume),
    ("ve vlaknu: podvrzeny ramec -> INVALID_ARGS, pak RESUME", forged_frame),
    ("ve vlaknu: NEXT pokracuje na kanal procesu", next_to_channel),
    ("ve vlaknu: preteceni zasobniku na alternativnim zasobniku", overflow_alt_stack),
    ("ve vlaknu: preteceni bez alternativniho zasobniku konci cisty", overflow_clean_end),
    ("ve vlaknu: vnoreni nad limit jde na kanal procesu", nesting_limit),
    ("ve vlaknu: chybne argumenty a RETURN bez obsluhy", bad_args),
    ("ve vlaknu: typovy ramec (breakpoint, alternativni zasobnik)", frame_api),
];

const RW: u32 = MAP_READ | MAP_WRITE;

fn deadline(ms: u64) -> Option<u64> {
    Some(time::deadline_after(Duration::from_millis(ms)))
}

fn page() -> u64 {
    vm::page_size()
}

/// Rezim obsluhy pro jednotlive testy.
static MODE: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
static RESULT: AtomicU64 = AtomicU64::new(0);
static FORGED: AtomicU64 = AtomicU64::new(0);

const M_FIX: u64 = 1;
const M_FORGE: u64 = 2;
const M_NEXT: u64 = 3;
const M_OVERFLOW: u64 = 4;
const M_NEST: u64 = 5;
const M_FRAME: u64 = 6;

/// Alternativni zasobnik testu `frame_api` (zacatek, delka).
static ALT: AtomicU64 = AtomicU64::new(0);
const ALT_LEN: u64 = 64 * 1024;

/// Offset PC v obecnych registrech ramce.
const PC_OFF: usize = frame::GENERAL + if cfg!(target_arch = "x86_64") { 16 * 8 } else { 32 * 8 };

extern "C" fn handler(f: *mut u8) -> ! {
    // SAFETY: `f` je ramec, ktery dalo jadro.
    let r = unsafe { frame_report(f) };
    let base = BASE.load(Ordering::SeqCst);
    let pg = page();
    match MODE.load(Ordering::SeqCst) {
        M_FIX | M_FORGE if r.kind == kind::PAGE_FAULT && r.stage == stage::IN_THREAD => {
            HITS.fetch_add(1, Ordering::SeqCst);
            if MODE.load(Ordering::SeqCst) == M_FORGE {
                // Podvrzene PC v jadre: navrat odmitnut, ramec se vrati.
                // SAFETY: PC v ramci (8 B na `PC_OFF`).
                let pc = unsafe { (f.add(PC_OFF) as *mut u64).read_unaligned() };
                unsafe { (f.add(PC_OFF) as *mut u64).write_unaligned(0xFFFF_8000_0000_0000) };
                // SAFETY: ramec obsluhy.
                let e = unsafe { handler_return(f, action::RESUME) };
                FORGED.store(e as u64, Ordering::SeqCst);
                unsafe { (f.add(PC_OFF) as *mut u64).write_unaligned(pc) };
            }
            let _ = vm::commit(base, pg, RW);
            // SAFETY: stranka je prave namapovana RW.
            unsafe { core::ptr::write_volatile(base as *mut u64, 99) };
        }
        M_NEXT => {
            HITS.fetch_add(1, Ordering::SeqCst);
            // SAFETY: ramec obsluhy.
            let e = unsafe { handler_return(f, action::NEXT) };
            RESULT.store(e as u64 | 0x1000, Ordering::SeqCst);
            libos::sys::thread_exit()
        }
        M_OVERFLOW => {
            HITS.fetch_add(1, Ordering::SeqCst);
            // Na alternativnim zasobniku: vlakno ukoncit (zasobnik je plny).
            libos::sys::thread_exit()
        }
        M_FRAME if r.kind == kind::BREAKPOINT => {
            // SAFETY: ramec od jadra.
            let mut fr = unsafe { Frame::from_raw(f) };
            let alt = ALT.load(Ordering::SeqCst);
            let on_alt = (alt..alt + ALT_LEN).contains(&(f as u64));
            let ok = fr.report().kind == kind::BREAKPOINT
                && signal_class(fr.report().kind) == SignalClass::Trap
                && fr.pc() == r.pc
                && fr.sp() != 0
                && fr.fp().len() == if cfg!(target_arch = "x86_64") { 512 } else { 528 }
                && on_alt;
            HITS.fetch_add(1, Ordering::SeqCst);
            RESULT.store(if ok { 77 } else { 1 }, Ordering::SeqCst);
            // aarch64: PC stoji na `brk`, posunout za nej (x86_64 uz je za `int3`).
            if cfg!(target_arch = "aarch64") {
                let pc = fr.pc();
                fr.set_pc(pc + 4);
            }
            let e = fr.resume();
            RESULT.store(e as u64 | 0x3000, Ordering::SeqCst);
            libos::sys::thread_exit()
        }
        M_NEST => {
            // Kazda obsluha sahne na dalsi stranku rezervace: vnoreni.
            let d = HITS.fetch_add(1, Ordering::SeqCst) + 1;
            if d <= u64::from(libos::abi::exception::handler::MAX_DEPTH) {
                // SAFETY: zamerny pristup do rezervace (dalsi vyjimka).
                let _ = unsafe { core::ptr::read_volatile((base + d * pg) as *const u64) };
            }
        }
        _ => {}
    }
    // SAFETY: ramec obsluhy.
    let e = unsafe { handler_return(f, action::RESUME) };
    RESULT.store(e as u64 | 0x2000, Ordering::SeqCst);
    libos::sys::thread_exit()
}

fn reset(mode: u64, pages: u64) -> Result<u64, alloc::string::String> {
    let base = vm::reserve(pages * page(), 0).map_err(|e| fail("rezervace", e))?;
    BASE.store(base, Ordering::SeqCst);
    HITS.store(0, Ordering::SeqCst);
    RESULT.store(0, Ordering::SeqCst);
    FORGED.store(0, Ordering::SeqCst);
    MODE.store(mode, Ordering::SeqCst);
    set_handler(handler).map_err(|e| fail("SET_ENTRY", e))?;
    Ok(base)
}

fn read_in_thread(base: u64) -> Result<libos::thread::JoinHandle, alloc::string::String> {
    RESULT.store(0, Ordering::SeqCst);
    libos::thread::spawn("cte", move || {
        // SAFETY: zamerny pristup do rezervace; vypadek resi obsluha.
        let v = unsafe { core::ptr::read_volatile(base as *const u64) };
        if RESULT.load(Ordering::SeqCst) == 0 {
            RESULT.store(v, Ordering::SeqCst);
        }
    })
    .map_err(|e| fail("vlakno", e))
}

fn finish(base: u64, pages: u64) {
    let _ = clear_handler();
    MODE.store(0, Ordering::SeqCst);
    let _ = vm::release(base, pages * page());
}

fn fix_and_resume(_: &Startup) -> TestResult {
    let base = reset(M_FIX, 1)?;
    let w = read_in_thread(base)?;
    let r = w.join(deadline(5000));
    finish(base, 1);
    r.map_err(|e| fail("vlakno", e))?;
    check_eq!(HITS.load(Ordering::SeqCst), 1, "obsluha zavolana jednou");
    check_eq!(RESULT.load(Ordering::SeqCst), 99, "vlakno precetlo opravenou stranku");
    Ok(())
}

fn forged_frame(_: &Startup) -> TestResult {
    let base = reset(M_FORGE, 1)?;
    let w = read_in_thread(base)?;
    let r = w.join(deadline(5000));
    finish(base, 1);
    r.map_err(|e| fail("vlakno", e))?;
    check_eq!(FORGED.load(Ordering::SeqCst), Error::InvalidArgs as u64, "podvrzene PC odmitnuto");
    check_eq!(RESULT.load(Ordering::SeqCst), 99, "po opravenem ramci pokracovalo");
    Ok(())
}

fn next_to_channel(s: &Startup) -> TestResult {
    let base = reset(M_NEXT, 1)?;
    let ch = s.process.exception_channel(false).map_err(|e| fail("kanal procesu", e))?;
    let w = read_in_thread(base)?;
    let e = receive(&ch, deadline(5000));
    let res = (|| {
        let e = e.map_err(|e| fail("zprava po NEXT", e))?;
        check_eq!((e.report.stage, e.report.kind, e.report.addr), (stage::PROCESS, kind::PAGE_FAULT, base), "kanal procesu po NEXT");
        vm::commit(base, page(), RW).map_err(|e| fail("commit", e))?;
        // SAFETY: stranka namapovana RW.
        unsafe { core::ptr::write_volatile(base as *mut u64, 55) };
        e.resume().map_err(|e| fail("RESUME", e))
    })();
    let j = w.join(deadline(5000));
    finish(base, 1);
    res?;
    j.map_err(|e| fail("vlakno", e))?;
    check_eq!(HITS.load(Ordering::SeqCst), 1, "obsluha ve vlaknu zavolana");
    check_eq!(RESULT.load(Ordering::SeqCst), 55, "vlakno pokracovalo po obsluze kanalu");
    Ok(())
}

/// Rekurze bez konce (preteceni zasobniku).
#[allow(unconditional_recursion, reason = "zamerne preteceni zasobniku (test)")]
fn recurse(d: u64) -> u64 {
    let pad = core::hint::black_box([d; 32]);
    recurse(d + 1) + pad[(d % 32) as usize]
}

fn overflow_alt_stack(_: &Startup) -> TestResult {
    let base = reset(M_OVERFLOW, 1)?;
    let alt = vm::alloc(64 * 1024, RW).map_err(|e| fail("alternativni zasobnik", e))?;
    let w = libos::thread::spawn("preteceni", move || {
        if set_alt_stack(alt, 64 * 1024).is_ok() {
            recurse(0);
        }
    })
    .map_err(|e| fail("vlakno", e))?;
    let r = w.join(deadline(10000));
    finish(base, 1);
    let _ = vm::release(alt, 64 * 1024);
    r.map_err(|e| fail("vlakno", e))?;
    check_eq!(HITS.load(Ordering::SeqCst), 1, "obsluha na alternativnim zasobniku zavolana");
    Ok(())
}

fn overflow_clean_end(s: &Startup) -> TestResult {
    let fs = libos::initfs::Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let (p, _t, ch) = libos::spawn::spawn_with_thread(&job, "preteceni-bez", image, Vec::new(), Default::default())
        .map_err(|e| fail("spusteni", e))?;
    // Obsluha = libovolna spustitelna adresa potomka (kod programu); ramec
    // se pri preteceni stejne nevejde.
    let mut at = 0x10000;
    let entry = loop {
        let q = p.query(at).map_err(|e| fail("dotaz", e))?;
        if q.kind == region::MAPPED && q.flags & MAP_EXECUTE != 0 {
            break q.start;
        }
        at = q.start + q.len;
        check!(at < 0x1_0000_0000, "potomek nema spustitelny kod");
    };
    libos::exception::set_handler_for(&p, entry).map_err(|e| fail("SET_ENTRY potomka", e))?;
    ch.write(b"overflow", Vec::new()).map_err(|e| fail("overflow", e))?;
    check_eq!(p.wait_exit(deadline(20000)), Ok(EXIT_FAULT_BASE - kind::PAGE_FAULT as i64), "cisty konec po preteceni");
    Ok(())
}

fn nesting_limit(s: &Startup) -> TestResult {
    let depth = u64::from(libos::abi::exception::handler::MAX_DEPTH);
    let pages = depth + 1;
    let base = reset(M_NEST, pages)?;
    let ch = s.process.exception_channel(false).map_err(|e| fail("kanal procesu", e))?;
    let w = read_in_thread(base)?;
    let res = (|| {
        let e = receive(&ch, deadline(5000)).map_err(|e| fail("vyjimka nad limit", e))?;
        check_eq!(e.report.stage, stage::PROCESS, "nad limit na kanal procesu");
        check_eq!(HITS.load(Ordering::SeqCst), depth, "pocet vnorenych obsluh");
        check_eq!(e.report.addr, base + depth * page(), "adresa vyjimky nad limit");
        vm::commit(base, pages * page(), RW).map_err(|e| fail("commit", e))?;
        e.resume().map_err(|e| fail("RESUME", e))
    })();
    let j = w.join(deadline(5000));
    finish(base, pages);
    res?;
    j.map_err(|e| fail("vlakno po rozmotani obsluh", e))?;
    Ok(())
}

static DATA: u64 = 0;

fn bad_args(_: &Startup) -> TestResult {
    // Vstup v nespustitelne pameti (W^X): odmitnuto.
    // Handle vlastniho procesu patri behovemu prostredi: nezavirat.
    let me = core::mem::ManuallyDrop::new(libos::Process::from_handle(libos::Handle::from_raw(libos::rt::self_process())));
    check_eq!(
        libos::exception::set_handler_for(&me, &DATA as *const u64 as u64).err(),
        Some(Error::InvalidArgs),
        "vstup v datech"
    );
    check_eq!(set_alt_stack(0x10000, 64).err(), Some(Error::InvalidArgs), "maly alternativni zasobnik");
    // SAFETY: bez obsluhy se nic neprovede, jen chyba.
    let e = unsafe { handler_return(core::ptr::null(), action::RESUME) };
    check_eq!(e, Error::BadState, "RETURN bez obsluhy");
    Ok(())
}

fn frame_api(_: &Startup) -> TestResult {
    let base = reset(M_FRAME, 1)?;
    let w = libos::thread::spawn("breakpoint", || {
        let Ok(alt) = alloc_alt_stack(ALT_LEN) else { return };
        ALT.store(alt, Ordering::SeqCst);
        // SAFETY: breakpoint; obsluha ho preskoci a vlakno pokracuje.
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("int3");
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("brk #0");
        }
        HITS.fetch_add(100, Ordering::SeqCst);
        let _ = set_alt_stack(0, 0);
        let _ = vm::release(alt, ALT_LEN);
    })
    .map_err(|e| fail("vlakno", e))?;
    let r = w.join(deadline(5000));
    finish(base, 1);
    r.map_err(|e| fail("vlakno", e))?;
    check_eq!(RESULT.load(Ordering::SeqCst), 77, "ramec: druh, trida, PC, SP, FPU, alternativni zasobnik");
    check_eq!(HITS.load(Ordering::SeqCst), 101, "obsluha jednou, vlakno pokracovalo za breakpointem");
    Ok(())
}
