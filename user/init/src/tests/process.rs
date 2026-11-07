//! Procesy a vlakna: spusteni programu z initfs, IPC mezi procesy, kody.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use libos::initfs::Initfs;
use libos::{Channel, Error, Event, Signals, Startup, Thread, Vmo, spawn, time, wait_one};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("initfs obsahuje echo", initfs_has_echo),
    ("ping-pong s jinym procesem 200x", ping_pong),
    ("predani eventu jinemu procesu", pass_event),
    ("navratovy kod procesu", exit_code),
    ("PROCESS_KILL ukonci spici proces", kill),
    ("pad procesu -> kod vyjimky", crash),
    ("druhe vlakno v procesu", second_thread),
    (
        "vlakno mimo pamet procesu -> INVALID_ARGS",
        thread_bad_entry,
    ),
    ("stav SSE je oddeleny mezi vlakny", sse_isolated),
];

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(5)))
}

fn spawn_echo(s: &Startup) -> Result<(libos::Process, Channel), alloc::string::String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    spawn("echo", image, Vec::new()).map_err(|e| fail("spusteni echo", e))
}

fn initfs_has_echo(s: &Startup) -> TestResult {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    check!(fs.names().any(|n| n == "echo"), "echo chybi v initfs");
    check!(fs.find("init").is_some(), "init chybi v initfs");
    Ok(())
}

fn ping_pong(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    for i in 0..200u32 {
        let msg = alloc::format!("ping:{i}");
        ch.write(msg.as_bytes(), Vec::new())
            .map_err(|e| fail("zapis", e))?;
        let reply = ch.read(deadline()).map_err(|e| fail("odpoved", e))?;
        check_eq!(
            reply.data,
            alloc::format!("pong:{i}").into_bytes(),
            "odpoved"
        );
    }
    drop(ch);
    check_eq!(p.wait_exit(deadline()), Ok(0), "kod po zavreni kanalu");
    Ok(())
}

fn pass_event(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    let e = Event::create().map_err(|e| fail("event", e))?;
    let copy = e.handle().duplicate(None).map_err(|e| fail("dup", e))?;
    ch.write(b"signal", vec![copy])
        .map_err(|e| fail("zapis", e))?;
    let reply = ch.read(deadline()).map_err(|e| fail("odpoved", e))?;
    check_eq!(&reply.data[..], b"ok", "odpoved");
    let sig = wait_one(e.handle(), Signals::SIGNALED, deadline())
        .map_err(|e| fail("cekani na event", e))?;
    check!(
        sig.contains(Signals::SIGNALED),
        "event nebyl signalizovan jinym procesem"
    );
    drop(ch);
    p.wait_exit(deadline()).map_err(|e| fail("konec", e))?;
    Ok(())
}

fn exit_code(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    ch.write(b"exit:7", Vec::new())
        .map_err(|e| fail("zapis", e))?;
    check_eq!(p.wait_exit(deadline()), Ok(7), "navratovy kod");
    Ok(())
}

fn kill(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    ch.write(b"sleep", Vec::new())
        .map_err(|e| fail("zapis", e))?;
    time::sleep(Duration::from_millis(10));
    p.kill().map_err(|e| fail("kill", e))?;
    check_eq!(
        p.wait_exit(deadline()),
        Ok(libos::abi::process::EXIT_KILLED),
        "kod po kill"
    );
    Ok(())
}

fn crash(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    ch.write(b"crash", Vec::new())
        .map_err(|e| fail("zapis", e))?;
    // Page fault = vektor 14.
    check_eq!(
        p.wait_exit(deadline()),
        Ok(libos::abi::process::EXIT_FAULT_BASE - 14),
        "kod po padu"
    );
    Ok(())
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Vstup druheho vlakna: rdi = handle eventu, rsi = kolikrat pricist.
extern "C" fn worker(event: u64, n: u64) -> ! {
    for _ in 0..n {
        COUNTER.fetch_add(1, Ordering::SeqCst);
    }
    let _ = libos::sys::object_signal(event as u32, 0, Signals::SIGNALED.0);
    libos::sys::thread_exit()
}

fn second_thread(s: &Startup) -> TestResult {
    COUNTER.store(0, Ordering::SeqCst);
    let e = Event::create().map_err(|e| fail("event", e))?;
    let stack = Vmo::create(64 * 1024).map_err(|e| fail("zasobnik", e))?;
    let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
    let base = s
        .process
        .map(&stack, 0, 64 * 1024, flags, 0)
        .map_err(|e| fail("mapovani zasobniku", e))?;
    let t = Thread::create(&s.process, "worker").map_err(|e| fail("vlakno", e))?;
    // Zasobnik zarovnany na 16 B minus 8 = jako po instrukci call.
    t.start(
        worker as *const () as u64,
        base + 64 * 1024 - 8,
        e.handle().raw() as u64,
        1000,
    )
    .map_err(|e| fail("spusteni vlakna", e))?;
    wait_one(e.handle(), Signals::SIGNALED, deadline()).map_err(|e| fail("cekani na vlakno", e))?;
    t.wait_exit(deadline())
        .map_err(|e| fail("konec vlakna", e))?;
    check_eq!(
        COUNTER.load(Ordering::SeqCst),
        1000,
        "pocitadlo z druheho vlakna"
    );
    Ok(())
}

fn thread_bad_entry(s: &Startup) -> TestResult {
    use libos::abi::vm::USER_MAX;
    let t = Thread::create(&s.process, "spatne").map_err(|e| fail("vlakno", e))?;
    // Nekanonicka adresa, adresa v jadre a posledni (vyhrazena) stranka.
    for entry in [0x8000_0000_0000u64, 0xFFFF_FFFF_8000_0000, USER_MAX, 0] {
        check_eq!(
            t.start(entry, 0x10_0000, 0, 0),
            Err(Error::InvalidArgs),
            alloc::format!("start na {entry:#x}").as_str()
        );
    }
    check_eq!(
        t.start(0x40_0000, USER_MAX + 4096, 0, 0),
        Err(Error::InvalidArgs),
        "zasobnik za koncem pameti procesu"
    );
    Ok(())
}

/// XMM0 nove spusteneho vlakna (musi byt 0) a hodnota, kterou si zapsalo.
static SSE_SEEN: AtomicU64 = AtomicU64::new(u64::MAX);

/// Spodnich 64 bitu prvniho registru FPU/SIMD (x86_64 XMM0, aarch64 V0).
fn xmm0_read() -> u64 {
    let v: u64;
    // SAFETY: jen presun z registru FPU do obecneho registru; jadro FPU
    // pro programy zapnulo.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("movq {}, xmm0", out(reg) v, options(nomem, nostack))
    };
    // SAFETY: viz vyse.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(".arch_extension fp", "fmov {}, d0", out(reg) v, options(nomem, nostack))
    };
    v
}

fn xmm0_write(v: u64) {
    // SAFETY: XMM0 program (soft-float) jinak nepouziva.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("movq xmm0, {}", in(reg) v, options(nomem, nostack))
    };
    // SAFETY: V0 program (soft-float) jinak nepouziva.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(".arch_extension fp", "fmov d0, {}", in(reg) v, options(nomem, nostack))
    };
}

extern "C" fn sse_worker(event: u64, _: u64) -> ! {
    SSE_SEEN.store(xmm0_read(), Ordering::SeqCst);
    xmm0_write(0xBAD0_BAD0_BAD0_BAD0);
    let _ = libos::sys::object_signal(event as u32, 0, Signals::SIGNALED.0);
    libos::sys::thread_exit()
}

fn sse_isolated(s: &Startup) -> TestResult {
    let e = Event::create().map_err(|e| fail("event", e))?;
    let stack = Vmo::create(64 * 1024).map_err(|e| fail("zasobnik", e))?;
    let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
    let base = s
        .process
        .map(&stack, 0, 64 * 1024, flags, 0)
        .map_err(|e| fail("mapovani", e))?;
    let t = Thread::create(&s.process, "sse").map_err(|e| fail("vlakno", e))?;
    xmm0_write(0x1234_5678_9ABC_DEF0);
    t.start(
        sse_worker as *const () as u64,
        base + 64 * 1024 - 8,
        e.handle().raw() as u64,
        0,
    )
    .map_err(|e| fail("spusteni", e))?;
    wait_one(e.handle(), Signals::SIGNALED, deadline()).map_err(|e| fail("cekani", e))?;
    t.wait_exit(deadline())
        .map_err(|e| fail("konec vlakna", e))?;
    check_eq!(
        SSE_SEEN.load(Ordering::SeqCst),
        0,
        "XMM0 noveho vlakna (unik z jineho vlakna)"
    );
    check_eq!(
        xmm0_read(),
        0x1234_5678_9ABC_DEF0,
        "XMM0 po prepnuti vlaken"
    );
    Ok(())
}
