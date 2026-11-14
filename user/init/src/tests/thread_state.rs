//! Pozastaveni vlakna a jeho kontext (`THREAD_SUSPEND`, `THREAD_STATE`):
//! bezici i spici vlakno, presmerovani PC, FPU, neplatny kontext, kill
//! pozastaveneho vlakna.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use libos::abi::thread::kind;
use libos::abi::vm::USER_MAX;
use libos::{Error, Event, Signals, Startup, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("vlakno: pozastaveni bezici smycky a presmerovani PC", redirect_running),
    ("vlakno: spici vlakno je pozastavene hned, po probuzeni stoji", blocked_thread),
    ("vlakno: neplatny kontext -> INVALID_ARGS, jadro prezije", invalid_context),
    ("vlakno: kill pozastaveneho vlakna jineho procesu", kill_suspended),
];

fn deadline(ms: u64) -> Option<u64> {
    Some(time::deadline_after(Duration::from_millis(ms)))
}

#[cfg(target_arch = "x86_64")]
mod regs {
    use libos::abi::thread::X64General as General;
    pub const SIZE: usize = General::SIZE;
    pub fn pc_sp(b: &[u8]) -> (u64, u64) {
        let g = General::decode(b.try_into().unwrap());
        (g.rip, g.rsp)
    }
    /// Nove PC a SP; SP jako po instrukci `call` (rsp % 16 == 8).
    pub fn set_pc_sp(b: &mut [u8], pc: u64, sp: u64) {
        let mut g = General::decode((&*b).try_into().unwrap());
        g.rip = pc;
        g.rsp = (sp & !15) - 8;
        b.copy_from_slice(&g.encode());
    }
    pub fn set_flags(b: &mut [u8], f: u64) {
        let mut g = General::decode((&*b).try_into().unwrap());
        g.rflags = f;
        b.copy_from_slice(&g.encode());
    }
    pub const FP_SIZE: usize = libos::abi::thread::X64_FP_SIZE;
    /// Rezervovany bit MXCSR (FXRSTOR by v jadre vyvolal #GP).
    pub fn break_fp(b: &mut [u8]) {
        let at = libos::abi::thread::X64_FP_MXCSR;
        b[at + 3] = 0x80;
    }
}

#[cfg(target_arch = "aarch64")]
mod regs {
    use libos::abi::thread::A64General as General;
    pub const SIZE: usize = General::SIZE;
    pub fn pc_sp(b: &[u8]) -> (u64, u64) {
        let g = General::decode(b.try_into().unwrap());
        (g.pc, g.sp)
    }
    pub fn set_pc_sp(b: &mut [u8], pc: u64, sp: u64) {
        let mut g = General::decode((&*b).try_into().unwrap());
        g.pc = pc;
        g.sp = sp & !15;
        b.copy_from_slice(&g.encode());
    }
    pub fn set_flags(b: &mut [u8], f: u64) {
        let mut g = General::decode((&*b).try_into().unwrap());
        g.pstate = f;
        b.copy_from_slice(&g.encode());
    }
    pub const FP_SIZE: usize = libos::abi::thread::A64_FP_SIZE;
    /// Nedefinovany bit FPCR.
    pub fn break_fp(b: &mut [u8]) {
        b[512] |= 1;
    }
}

static SPIN: AtomicU64 = AtomicU64::new(0);
static REDIRECTED: AtomicU64 = AtomicU64::new(0);

/// Kam se presmeruje pozastavena smycka: oznaci se a vlakno skonci.
extern "C" fn redirected() -> ! {
    REDIRECTED.store(1, Ordering::SeqCst);
    libos::sys::thread_exit()
}

fn redirect_running(_: &Startup) -> TestResult {
    SPIN.store(0, Ordering::SeqCst);
    REDIRECTED.store(0, Ordering::SeqCst);
    let w = libos::thread::spawn("smycka", || loop {
        SPIN.fetch_add(1, Ordering::Relaxed);
    })
    .map_err(|e| fail("vlakno", e))?;
    let t = w.thread();
    while SPIN.load(Ordering::Relaxed) == 0 {
        libos::sys::thread_yield();
    }
    let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
    t.wait_suspended(deadline(5000)).map_err(|e| fail("SUSPENDED", e))?;
    let a = SPIN.load(Ordering::SeqCst);
    time::sleep(Duration::from_millis(30));
    check_eq!(SPIN.load(Ordering::SeqCst), a, "pozastavena smycka bezi dal");
    let mut g = vec![0u8; regs::SIZE];
    t.read_state(kind::GENERAL, &mut g).map_err(|e| fail("cteni registru", e))?;
    let (pc, sp) = regs::pc_sp(&g);
    check!(pc > 0x10000 && pc < USER_MAX && sp > 0x10000 && sp < USER_MAX, "PC a SP v pameti procesu");
    // FPU: cteni a zapis tehoz stavu.
    let mut fp = vec![0u8; regs::FP_SIZE];
    t.read_state(kind::FP, &mut fp).map_err(|e| fail("cteni FPU", e))?;
    t.write_state(kind::FP, &fp).map_err(|e| fail("zapis FPU", e))?;
    // Druhy token: obnoveni az po zavreni obou.
    let second = t.suspend().map_err(|e| fail("druhy token", e))?;
    regs::set_pc_sp(&mut g, redirected as *const () as u64, sp - 1024);
    t.write_state(kind::GENERAL, &g).map_err(|e| fail("zapis registru", e))?;
    let mut back = vec![0u8; regs::SIZE];
    t.read_state(kind::GENERAL, &mut back).map_err(|e| fail("cteni po zapisu", e))?;
    check_eq!(back, g, "cteni vidi cekajici zapis");
    drop(token);
    time::sleep(Duration::from_millis(30));
    check_eq!(REDIRECTED.load(Ordering::SeqCst), 0, "rozbehlo se s jednim zivym tokenem");
    drop(second);
    w.join(deadline(5000)).map_err(|e| fail("konec presmerovaneho vlakna", e))?;
    check_eq!(REDIRECTED.load(Ordering::SeqCst), 1, "vlakno pokracovalo na novem PC");
    Ok(())
}

static WOKE: AtomicU64 = AtomicU64::new(0);

fn blocked_thread(_: &Startup) -> TestResult {
    WOKE.store(0, Ordering::SeqCst);
    let ev = Event::create().map_err(|e| fail("event", e))?;
    let ev2 = Event::from_handle(ev.handle().duplicate(None).map_err(|e| fail("dup", e))?);
    let w = libos::thread::spawn("spi", move || {
        let _ = libos::wait_one(ev2.handle(), Signals::SIGNALED, None);
        WOKE.store(1, Ordering::SeqCst);
    })
    .map_err(|e| fail("vlakno", e))?;
    let t = w.thread();
    time::sleep(Duration::from_millis(50));
    let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
    // Spi v syscallu: pozastavene hned (bez cekani na konec cekani).
    t.wait_suspended(deadline(1000)).map_err(|e| fail("spici vlakno neni pozastavene hned", e))?;
    let mut g = vec![0u8; regs::SIZE];
    t.read_state(kind::GENERAL, &mut g).map_err(|e| fail("cteni registru spiciho", e))?;
    // Probuzeni: vlakno dokonci syscall, ale do userspace se nevrati.
    ev.signal().map_err(|e| fail("signal", e))?;
    time::sleep(Duration::from_millis(50));
    check_eq!(WOKE.load(Ordering::SeqCst), 0, "pozastavene vlakno se vratilo do userspace");
    t.read_state(kind::GENERAL, &mut g).map_err(|e| fail("cteni po probuzeni", e))?;
    drop(token);
    w.join(deadline(5000)).map_err(|e| fail("join", e))?;
    check_eq!(WOKE.load(Ordering::SeqCst), 1, "po obnoveni dobehlo");
    Ok(())
}

fn invalid_context(_: &Startup) -> TestResult {
    let ev = Event::create().map_err(|e| fail("event", e))?;
    let ev2 = Event::from_handle(ev.handle().duplicate(None).map_err(|e| fail("dup", e))?);
    let w = libos::thread::spawn("cil", move || {
        let _ = libos::wait_one(ev2.handle(), Signals::SIGNALED, None);
    })
    .map_err(|e| fail("vlakno", e))?;
    let t = w.thread();
    // At vlakno vstoupi do userspace a usne (pred prvnim vstupem kontext
    // jeste neni).
    time::sleep(Duration::from_millis(50));
    let mut g = vec![0u8; regs::SIZE];
    check_eq!(t.read_state(kind::GENERAL, &mut g).err(), Some(Error::BadState), "cteni bez pozastaveni");
    let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
    t.wait_suspended(deadline(1000)).map_err(|e| fail("SUSPENDED", e))?;
    t.read_state(kind::GENERAL, &mut g).map_err(|e| fail("cteni", e))?;
    let (pc, sp) = regs::pc_sp(&g);
    let good = g.clone();
    // PC a SP mimo userspace (nekanonicke na x86_64): zapis odmitnut.
    for (bad_pc, bad_sp, what) in [
        (USER_MAX, sp, "PC na USER_MAX"),
        (0x8000_0000_0000_0000, sp, "PC v jadre"),
        (pc, USER_MAX + 4096, "SP nad userspace"),
    ] {
        let mut b = good.clone();
        regs::set_pc_sp(&mut b, bad_pc, bad_sp + 8);
        check_eq!(t.write_state(kind::GENERAL, &b).err(), Some(Error::InvalidArgs), what);
    }
    // Privilegovane priznaky se orezou (zapis projde, vlakno pobezi dal).
    let mut flags = good.clone();
    regs::set_flags(&mut flags, u64::MAX);
    t.write_state(kind::GENERAL, &flags).map_err(|e| fail("priznaky", e))?;
    t.write_state(kind::GENERAL, &good).map_err(|e| fail("puvodni registry", e))?;
    let mut fp = vec![0u8; regs::FP_SIZE];
    t.read_state(kind::FP, &mut fp).map_err(|e| fail("cteni FPU", e))?;
    regs::break_fp(&mut fp);
    check_eq!(t.write_state(kind::FP, &fp).err(), Some(Error::InvalidArgs), "rezervovane bity FPU");
    // Spatna delka, druh a operace.
    check_eq!(t.read_state(kind::GENERAL, &mut g[..8]).err(), Some(Error::InvalidArgs), "kratky buffer");
    check_eq!(t.read_state(7, &mut g).err(), Some(Error::InvalidArgs), "neznamy druh");
    let mut z = vec![0u8; regs::SIZE];
    check_eq!(
        libos::sys::thread_state_read(t.handle().raw(), 9, &mut z).err(),
        Some(Error::InvalidArgs),
        "spatny druh pres syscall"
    );
    drop(token);
    ev.signal().map_err(|e| fail("signal", e))?;
    w.join(deadline(5000)).map_err(|e| fail("vlakno po zapisech dobehlo", e))?;
    Ok(())
}

fn kill_suspended(s: &Startup) -> TestResult {
    let fs = libos::initfs::Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let (p, t, ch) = libos::spawn::spawn_with_thread(&job, "pozastaveny", image, Vec::new(), Default::default())
        .map_err(|e| fail("spusteni", e))?;
    ch.write(b"sleep", Vec::new()).map_err(|e| fail("sleep", e))?;
    time::sleep(Duration::from_millis(50));
    let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
    t.wait_suspended(deadline(1000)).map_err(|e| fail("SUSPENDED", e))?;
    // Kill vyhraje nad pozastavenim (token zije dal).
    p.kill().map_err(|e| fail("kill", e))?;
    check!(p.wait_exit(deadline(5000)).is_ok(), "pozastaveny proces po kill neskoncil");
    drop(token);
    // Pozastavit skoncene vlakno nejde.
    check_eq!(t.suspend().err(), Some(Error::BadState), "pozastaveni skonceneho vlakna");
    Ok(())
}
