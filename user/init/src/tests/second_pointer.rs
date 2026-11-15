//! Druhy ukazatel vlakna (`THREAD_SET_POINTER` s `thread_pointer::SECOND`):
//! na x86_64 baze GS programu pro kazde vlakno (TEB Windows vrstvy,
//! docs/cs/windows-vrstva.md), v kontextu `THREAD_STATE`; na aarch64
//! `NOT_SUPPORTED`.

use super::{Test, TestResult};

pub const TESTS: &[Test] = &[
    ("druhy ukazatel vlakna: GS pro kazde vlakno, kontext, chyby", second_pointer),
];

#[cfg(target_arch = "aarch64")]
fn second_pointer(_: &libos::Startup) -> TestResult {
    use crate::check_eq;
    // aarch64 druhy ukazatel nema (Windows ARM64 drzi TEB v x18).
    check_eq!(libos::sys::thread_set_second_pointer(0x1000), Err(libos::Error::NotSupported), "aarch64: SECOND");
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn second_pointer(s: &libos::Startup) -> TestResult {
    x64::run(s)
}

#[cfg(target_arch = "x86_64")]
mod x64 {
    use alloc::vec;
    use core::sync::atomic::{AtomicU64, Ordering};
    use core::time::Duration;

    use libos::abi::thread::X64General as General;
    use libos::abi::thread::kind;
    use libos::abi::vm::USER_MAX;
    use libos::{Error, Startup, time};

    use super::super::{TestResult, fail};
    use crate::{check, check_eq};

    /// Cil GS (prvni slovo cte `gs:[0]`).
    #[repr(C, align(64))]
    struct Block([u64; 8]);

    static A: Block = Block([0xA1A1_0000_0000_0001, 0, 0, 0, 0, 0, 0, 0]);
    static B: Block = Block([0xB2B2_0000_0000_0002, 0, 0, 0, 0, 0, 0, 0]);
    static C: Block = Block([0xC3C3_0000_0000_0003, 0, 0, 0, 0, 0, 0, 0]);

    /// Vlakno B: kolikrat cetlo svou hodnotu, chyba, posledni prectena.
    static B_OK: AtomicU64 = AtomicU64::new(0);
    static B_BAD: AtomicU64 = AtomicU64::new(0);
    static B_LAST: AtomicU64 = AtomicU64::new(0);
    static B_STOP: AtomicU64 = AtomicU64::new(0);

    fn read_gs0() -> u64 {
        let v: u64;
        // SAFETY: cteni slova z bazy GS, kterou vlakno samo nastavilo na
        // platny blok (static).
        unsafe { core::arch::asm!("mov {}, gs:[0]", out(reg) v, options(nostack, readonly, preserves_flags)) };
        v
    }

    fn addr(b: &'static Block) -> u64 {
        b as *const Block as u64
    }

    pub fn run(_: &Startup) -> TestResult {
        // Chyby: adresa mimo userspace.
        check_eq!(libos::sys::thread_set_second_pointer(USER_MAX), Err(Error::InvalidArgs), "GS mimo userspace");
        libos::sys::thread_set_second_pointer(addr(&A)).map_err(|e| fail("GS hlavniho vlakna", e))?;
        check_eq!(read_gs0(), A.0[0], "gs:[0] hlavniho vlakna");
        B_OK.store(0, Ordering::SeqCst);
        B_BAD.store(0, Ordering::SeqCst);
        B_STOP.store(0, Ordering::SeqCst);
        let w = libos::thread::spawn("gs-b", || {
            if libos::sys::thread_set_second_pointer(addr(&B)).is_err() {
                B_BAD.fetch_add(1, Ordering::SeqCst);
                return;
            }
            while B_STOP.load(Ordering::Relaxed) == 0 {
                let v = read_gs0();
                B_LAST.store(v, Ordering::Relaxed);
                if v == B.0[0] || v == C.0[0] {
                    B_OK.fetch_add(1, Ordering::Relaxed);
                } else {
                    B_BAD.fetch_add(1, Ordering::Relaxed);
                }
                libos::sys::thread_yield();
            }
        })
        .map_err(|e| fail("vlakno B", e))?;
        // Obe vlakna se stridaji na CPU: kazde vidi jen svuj GS.
        let end = time::deadline_after(Duration::from_secs(5));
        while B_OK.load(Ordering::Relaxed) < 2000 && time::now() < end {
            check_eq!(read_gs0(), A.0[0], "hlavni vlakno po prepnuti");
            libos::sys::thread_yield();
        }
        check!(B_OK.load(Ordering::SeqCst) >= 2000, "vlakno B neprecetlo svuj GS dost krat");
        check_eq!(B_BAD.load(Ordering::SeqCst), 0, "vlakno B videlo cizi GS");
        // Kontext: pozastavit B, precist GS, prepsat na C.
        let t = w.thread();
        let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
        t.wait_suspended(Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("SUSPENDED", e))?;
        let mut g = vec![0u8; General::SIZE];
        t.read_state(kind::GENERAL, &mut g).map_err(|e| fail("cteni kontextu", e))?;
        let mut regs = General::decode((&*g).try_into().unwrap());
        check_eq!(regs.gs_base, addr(&B), "gs_base v kontextu");
        regs.gs_base = USER_MAX;
        check_eq!(t.write_state(kind::GENERAL, &regs.encode()), Err(Error::InvalidArgs), "gs_base mimo userspace v kontextu");
        regs.gs_base = addr(&C);
        t.write_state(kind::GENERAL, &regs.encode()).map_err(|e| fail("zapis gs_base", e))?;
        drop(token);
        let end = time::deadline_after(Duration::from_secs(5));
        while B_LAST.load(Ordering::Relaxed) != C.0[0] && time::now() < end {
            libos::sys::thread_yield();
        }
        check_eq!(B_LAST.load(Ordering::SeqCst), C.0[0], "vlakno B po zapisu kontextu cte novy GS");
        B_STOP.store(1, Ordering::SeqCst);
        w.join(Some(time::deadline_after(Duration::from_secs(5)))).map_err(|e| fail("join B", e))?;
        check_eq!(B_BAD.load(Ordering::SeqCst), 0, "vlakno B videlo cizi GS");
        check_eq!(read_gs0(), A.0[0], "hlavni vlakno na konci");
        // Uklid: GS initu zpet na 0 (dalsi testy ho necekaji).
        libos::sys::thread_set_second_pointer(0).map_err(|e| fail("GS zpet na 0", e))?;
        Ok(())
    }
}
