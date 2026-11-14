//! Doruceni vyjimek: kanaly vyjimek procesu, vlakna a debuggeru, poradi
//! obsluh, vyreseni (RESUME, NEXT, KILL), kill behem vyjimky, vlakno
//! obsluhy a vycet vlaken procesu.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use libos::abi::exception::{access, kind, stage};
use libos::abi::process::EXIT_FAULT_BASE;
use libos::exception::receive;
use libos::{Error, Startup, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("vyjimky: obsluha vyresi vypadek stranky (commit + RESUME)", resolve_page_fault),
    ("vyjimky: breakpoint, obsluha vlakna posune PC", breakpoint),
    ("vyjimky: poradi debugger, proces, debugger a konec procesu", order_and_kill),
    ("vyjimky: kill behem vyjimky, token pak BAD_STATE", kill_during),
    ("vyjimky: vlakno obsluhy preskoci kanal procesu", handler_thread),
    ("PROCESS_THREADS: vlakna jineho procesu", process_threads),
];

fn deadline(ms: u64) -> Option<u64> {
    Some(time::deadline_after(Duration::from_millis(ms)))
}

static VALUE: AtomicU64 = AtomicU64::new(0);
static AT: AtomicU64 = AtomicU64::new(0);

fn resolve_page_fault(s: &Startup) -> TestResult {
    let page = libos::vm::page_size();
    let base = libos::vm::reserve(page, 0).map_err(|e| fail("rezervace", e))?;
    AT.store(base, Ordering::SeqCst);
    VALUE.store(0, Ordering::SeqCst);
    let ch = s.process.exception_channel(false).map_err(|e| fail("kanal procesu", e))?;
    // Druhy kanal procesu nejde.
    check_eq!(s.process.exception_channel(false).err(), Some(Error::AlreadyExists), "druhy kanal procesu");
    let w = libos::thread::spawn("vypadek", || {
        // SAFETY: zamerny pristup do rezervace - vypadek vyresi obsluha
        // (namapuje stranku a nech vlakno pokracovat).
        let v = unsafe { core::ptr::read_volatile(AT.load(Ordering::SeqCst) as *const u64) };
        VALUE.store(v, Ordering::SeqCst);
    })
    .map_err(|e| fail("vlakno", e))?;
    let e = receive(&ch, deadline(5000)).map_err(|e| fail("zprava o vyjimce", e))?;
    check_eq!((e.report.kind, e.report.stage, e.report.access), (kind::PAGE_FAULT, stage::PROCESS, access::READ), "zprava");
    check_eq!(e.report.addr, base, "adresa vypadku");
    // Vlakno ve vyjimce: kontext jde cist.
    let mut g = vec![0u8; 512];
    let n = if cfg!(target_arch = "x86_64") { 160 } else { 280 };
    e.thread.read_state(libos::abi::thread::kind::GENERAL, &mut g[..n]).map_err(|e| fail("kontext ve vyjimce", e))?;
    libos::vm::commit(base, page, libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE).map_err(|e| fail("commit", e))?;
    // SAFETY: stranka je prave namapovana RW.
    unsafe { core::ptr::write_volatile(base as *mut u64, 77) };
    e.resume().map_err(|e| fail("RESUME", e))?;
    check_eq!(e.resume().err(), Some(Error::BadState), "druhe rozhodnuti");
    w.join(deadline(5000)).map_err(|e| fail("vlakno po RESUME", e))?;
    check_eq!(VALUE.load(Ordering::SeqCst), 77, "vlakno precetlo opravenou stranku");
    drop(ch);
    libos::vm::release(base, page).map_err(|e| fail("uvolneni", e))?;
    Ok(())
}

static PASSED: AtomicU64 = AtomicU64::new(0);

fn hit_breakpoint() {
    // SAFETY: zamerny breakpoint; obsluha vlakna ho preskoci.
    unsafe {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("int3");
        #[cfg(target_arch = "aarch64")]
        core::arch::asm!("brk #0");
    }
    PASSED.store(1, Ordering::SeqCst);
}

fn breakpoint(_: &Startup) -> TestResult {
    PASSED.store(0, Ordering::SeqCst);
    let gate = libos::Event::create().map_err(|e| fail("event", e))?;
    let g2 = libos::Event::from_handle(gate.handle().duplicate(None).map_err(|e| fail("dup", e))?);
    let w = libos::thread::spawn("breakpoint", move || {
        let _ = libos::wait_one(g2.handle(), libos::Signals::SIGNALED, None);
        hit_breakpoint();
    })
    .map_err(|e| fail("vlakno", e))?;
    let ch = w.thread().exception_channel().map_err(|e| fail("kanal vlakna", e))?;
    gate.signal().map_err(|e| fail("signal", e))?;
    let e = receive(&ch, deadline(5000)).map_err(|e| fail("zprava", e))?;
    check_eq!((e.report.kind, e.report.stage), (kind::BREAKPOINT, stage::THREAD), "breakpoint na kanalu vlakna");
    // aarch64: `brk` necha PC na instrukci, obsluha ho posune.
    #[cfg(target_arch = "aarch64")]
    {
        use libos::abi::thread::{A64General, kind as tk};
        let mut b = vec![0u8; A64General::SIZE];
        e.thread.read_state(tk::GENERAL, &mut b).map_err(|e| fail("cteni", e))?;
        let mut r = A64General::decode(b.as_slice().try_into().unwrap());
        check_eq!(r.pc, e.report.pc, "PC ve zprave");
        r.pc += 4;
        e.thread.write_state(tk::GENERAL, &r.encode()).map_err(|e| fail("zapis PC", e))?;
    }
    e.resume().map_err(|e| fail("RESUME", e))?;
    w.join(deadline(5000)).map_err(|e| fail("join", e))?;
    check_eq!(PASSED.load(Ordering::SeqCst), 1, "vlakno pokracovalo za breakpointem");
    Ok(())
}

fn echo(s: &Startup, name: &str) -> Result<(libos::Process, libos::Thread, libos::Channel), alloc::string::String> {
    let fs = libos::initfs::Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    libos::spawn::spawn_with_thread(&job, name, image, Vec::new(), Default::default()).map_err(|e| fail("spusteni", e))
}

fn order_and_kill(s: &Startup) -> TestResult {
    let (p, _t, ch) = echo(s, "vyjimka-poradi")?;
    let dbg = p.exception_channel(true).map_err(|e| fail("debugger", e))?;
    let own = p.exception_channel(false).map_err(|e| fail("kanal procesu", e))?;
    ch.write(b"crash", Vec::new()).map_err(|e| fail("crash", e))?;
    let first = receive(&dbg, deadline(5000)).map_err(|e| fail("prvni sance", e))?;
    check_eq!((first.report.stage, first.report.kind), (stage::DEBUGGER_FIRST, kind::PAGE_FAULT), "prvni sance");
    first.next().map_err(|e| fail("NEXT", e))?;
    let proc = receive(&own, deadline(5000)).map_err(|e| fail("kanal procesu", e))?;
    check_eq!(proc.report.stage, stage::PROCESS, "kanal procesu");
    drop(proc); // zavreni tokenu = NEXT
    let second = receive(&dbg, deadline(5000)).map_err(|e| fail("druha sance", e))?;
    check_eq!(second.report.stage, stage::DEBUGGER_SECOND, "druha sance");
    second.next().map_err(|e| fail("NEXT", e))?;
    check_eq!(p.wait_exit(deadline(5000)), Ok(EXIT_FAULT_BASE - kind::PAGE_FAULT as i64), "konec po neosetrene vyjimce");
    Ok(())
}

fn kill_during(s: &Startup) -> TestResult {
    let (p, _t, ch) = echo(s, "vyjimka-kill")?;
    let own = p.exception_channel(false).map_err(|e| fail("kanal", e))?;
    ch.write(b"crash", Vec::new()).map_err(|e| fail("crash", e))?;
    let e = receive(&own, deadline(5000)).map_err(|e| fail("zprava", e))?;
    p.kill().map_err(|e| fail("kill", e))?;
    check!(p.wait_exit(deadline(5000)).is_ok(), "proces ve vyjimce po kill neskoncil");
    check_eq!(e.resume().err(), Some(Error::BadState), "rozhodnuti po kill");
    Ok(())
}

fn handler_thread(s: &Startup) -> TestResult {
    let (p, t, ch) = echo(s, "vyjimka-obsluha")?;
    t.mark_exception_handler().map_err(|e| fail("vlakno obsluhy", e))?;
    let own = p.exception_channel(false).map_err(|e| fail("kanal", e))?;
    ch.write(b"crash", Vec::new()).map_err(|e| fail("crash", e))?;
    check_eq!(p.wait_exit(deadline(5000)), Ok(EXIT_FAULT_BASE - kind::PAGE_FAULT as i64), "konec bez obsluhy");
    check_eq!(receive(&own, deadline(100)).err().map(|_| ()), Some(()), "kanal procesu nic nedostal");
    Ok(())
}

fn process_threads(s: &Startup) -> TestResult {
    let (p, _t, ch) = echo(s, "vlakna")?;
    ch.write(b"sleep", Vec::new()).map_err(|e| fail("sleep", e))?;
    let threads = p.threads().map_err(|e| fail("PROCESS_THREADS", e))?;
    check_eq!(threads.len(), 1, "pocet vlaken");
    let token = threads[0].suspend().map_err(|e| fail("pozastaveni pres vycet", e))?;
    threads[0].wait_suspended(deadline(1000)).map_err(|e| fail("SUSPENDED", e))?;
    drop(token);
    p.kill().map_err(|e| fail("kill", e))?;
    p.wait_exit(deadline(5000)).map_err(|e| fail("konec", e))?;
    Ok(())
}
