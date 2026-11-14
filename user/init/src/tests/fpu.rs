//! FPU a SIMD v programech (cile `*-unknown-magnumopus`, hard-float):
//! nove vlakno zacina s cistym stavem, registry preziji prepnuti vlaken
//! a vypocty v plovouci carce v soubeznych vlaknech davaji presne vysledky.
//! FP cast ramce vyjimky testuje `exceptions_fast::frame_api`.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use libos::abi::syscall;
use libos::{Event, Signals, Startup, Thread, Vmo, time, wait_one};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("FPU: nove vlakno zacina s nulovym stavem", fresh_state),
    ("FPU: vsechny vektorove registry preziji prepnuti vlaken", registers_survive_switch),
    ("FPU: vypocty f64 v soubeznych vlaknech", float_math_in_threads),
    ("FPU: AVX - horni poloviny YMM preziji prepnuti vlaken", super::fpu_avx::ymm_survive_switch),
    ("FPU: AVX - THREAD_STATE cte a zapisuje YMM v obrazu XSAVE", super::fpu_avx::ymm_thread_state),
];

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(20)))
}

/// XMM0/V0 nove spusteneho vlakna (musi byt 0).
static SEEN: AtomicU64 = AtomicU64::new(u64::MAX);

extern "C" fn fresh_worker(event: u64, _: u64) -> ! {
    let v: u64;
    // SAFETY: prvni instrukce vlakna: jen presun z registru FPU.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!("movq {}, xmm0", out(reg) v, options(nomem, nostack))
    };
    // SAFETY: viz vyse.
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!("fmov {}, d0", out(reg) v, options(nomem, nostack))
    };
    SEEN.store(v, Ordering::SeqCst);
    let _ = libos::sys::object_signal(event as u32, 0, Signals::SIGNALED.0);
    libos::sys::thread_exit()
}

fn fresh_state(s: &Startup) -> TestResult {
    let e = Event::create().map_err(|e| fail("event", e))?;
    let stack = Vmo::create(64 * 1024).map_err(|e| fail("zasobnik", e))?;
    let flags = libos::abi::vm::MAP_READ | libos::abi::vm::MAP_WRITE;
    let base = s.process.map(&stack, 0, 64 * 1024, flags, 0).map_err(|e| fail("mapovani", e))?;
    let t = Thread::create(&s.process, "fpu-nove").map_err(|e| fail("vlakno", e))?;
    let entry = fresh_worker as *const () as u64;
    let top = base + 64 * 1024 - 8;
    let ev = e.handle().raw() as u64;
    let th = t.handle().raw() as u64;
    let r: u64;
    // Stav tohoto vlakna nesmi prosaknout do noveho: XMM0/V0 se nastavi
    // tesne pred THREAD_START v tomtez bloku (prekladac ho mezi tim nepouzije).
    // SAFETY: THREAD_START se stejnymi argumenty jako `sys::thread_start`;
    // syscall meni jen rax, rcx a r11; XMM0 je oznaceny jako zmeneny.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "movq xmm0, {p}",
            "syscall",
            p = in(reg) 0x1234_5678_9ABC_DEF0u64,
            inlateout("rax") syscall::THREAD_START => r,
            in("rdi") th, in("rsi") entry, in("rdx") top, in("r10") ev, in("r8") 0u64, in("r9") 0u64,
            lateout("rcx") _, lateout("r11") _, out("xmm0") _,
            options(nostack)
        );
    }
    // SAFETY: viz vyse (aarch64 meni jen x0).
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "fmov d0, {p}",
            "svc #0",
            p = in(reg) 0x1234_5678_9ABC_DEF0u64,
            in("x8") syscall::THREAD_START,
            inlateout("x0") th => r,
            in("x1") entry, in("x2") top, in("x3") ev, in("x4") 0u64, in("x5") 0u64,
            out("v0") _,
            options(nostack)
        );
    }
    check!((r as i64) >= 0, alloc::format!("THREAD_START: {}", r as i64));
    wait_one(e.handle(), Signals::SIGNALED, deadline()).map_err(|e| fail("cekani", e))?;
    t.wait_exit(deadline()).map_err(|e| fail("konec vlakna", e))?;
    check_eq!(SEEN.load(Ordering::SeqCst), 0, "XMM0/V0 noveho vlakna (unik z jineho vlakna)");
    Ok(())
}

/// Bajty vsech vektorovych registru (x86_64 XMM0-15 = 256 B, aarch64
/// V0-31 = 512 B).
const REGS: usize = if cfg!(target_arch = "x86_64") { 256 } else { 512 };

/// Nahraje vsechny vektorove registry z `a`, `n`krat zavola THREAD_YIELD
/// a ulozi registry do `b` - vse v jednom bloku asm, takze mezi tim
/// registry nepouzije nic jineho nez jadro (prepnuti na jina vlakna).
fn hold_regs_across_yields(a: &[u8; REGS], b: &mut [u8; REGS], n: u64) {
    // SAFETY: ctou se a pisou jen predane buffery (REGS bajtu); syscall
    // THREAD_YIELD bez argumentu meni jen rax, rcx a r11; vsechny
    // vektorove registry jsou oznacene jako zmenene.
    #[cfg(target_arch = "x86_64")]
    unsafe {
        core::arch::asm!(
            "movdqu xmm0, [rdi]", "movdqu xmm1, [rdi+16]", "movdqu xmm2, [rdi+32]", "movdqu xmm3, [rdi+48]",
            "movdqu xmm4, [rdi+64]", "movdqu xmm5, [rdi+80]", "movdqu xmm6, [rdi+96]", "movdqu xmm7, [rdi+112]",
            "movdqu xmm8, [rdi+128]", "movdqu xmm9, [rdi+144]", "movdqu xmm10, [rdi+160]", "movdqu xmm11, [rdi+176]",
            "movdqu xmm12, [rdi+192]", "movdqu xmm13, [rdi+208]", "movdqu xmm14, [rdi+224]", "movdqu xmm15, [rdi+240]",
            "2:",
            "mov eax, {y}",
            "syscall",
            "dec rdx",
            "jnz 2b",
            "movdqu [rsi], xmm0", "movdqu [rsi+16], xmm1", "movdqu [rsi+32], xmm2", "movdqu [rsi+48], xmm3",
            "movdqu [rsi+64], xmm4", "movdqu [rsi+80], xmm5", "movdqu [rsi+96], xmm6", "movdqu [rsi+112], xmm7",
            "movdqu [rsi+128], xmm8", "movdqu [rsi+144], xmm9", "movdqu [rsi+160], xmm10", "movdqu [rsi+176], xmm11",
            "movdqu [rsi+192], xmm12", "movdqu [rsi+208], xmm13", "movdqu [rsi+224], xmm14", "movdqu [rsi+240], xmm15",
            y = const syscall::THREAD_YIELD,
            in("rdi") a.as_ptr(), in("rsi") b.as_mut_ptr(), inout("rdx") n => _,
            out("rax") _, out("rcx") _, out("r11") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _, out("xmm4") _, out("xmm5") _, out("xmm6") _, out("xmm7") _,
            out("xmm8") _, out("xmm9") _, out("xmm10") _, out("xmm11") _, out("xmm12") _, out("xmm13") _, out("xmm14") _, out("xmm15") _,
            options(nostack)
        );
    }
    // SAFETY: viz vyse (aarch64: syscall meni jen x0).
    #[cfg(target_arch = "aarch64")]
    unsafe {
        core::arch::asm!(
            "mov x9, {a}",
            "ld1 {{v0.16b, v1.16b, v2.16b, v3.16b}}, [x9], #64",
            "ld1 {{v4.16b, v5.16b, v6.16b, v7.16b}}, [x9], #64",
            "ld1 {{v8.16b, v9.16b, v10.16b, v11.16b}}, [x9], #64",
            "ld1 {{v12.16b, v13.16b, v14.16b, v15.16b}}, [x9], #64",
            "ld1 {{v16.16b, v17.16b, v18.16b, v19.16b}}, [x9], #64",
            "ld1 {{v20.16b, v21.16b, v22.16b, v23.16b}}, [x9], #64",
            "ld1 {{v24.16b, v25.16b, v26.16b, v27.16b}}, [x9], #64",
            "ld1 {{v28.16b, v29.16b, v30.16b, v31.16b}}, [x9]",
            "2:",
            "mov x8, #{y}",
            "svc #0",
            "subs {n}, {n}, #1",
            "b.ne 2b",
            "mov x9, {b}",
            "st1 {{v0.16b, v1.16b, v2.16b, v3.16b}}, [x9], #64",
            "st1 {{v4.16b, v5.16b, v6.16b, v7.16b}}, [x9], #64",
            "st1 {{v8.16b, v9.16b, v10.16b, v11.16b}}, [x9], #64",
            "st1 {{v12.16b, v13.16b, v14.16b, v15.16b}}, [x9], #64",
            "st1 {{v16.16b, v17.16b, v18.16b, v19.16b}}, [x9], #64",
            "st1 {{v20.16b, v21.16b, v22.16b, v23.16b}}, [x9], #64",
            "st1 {{v24.16b, v25.16b, v26.16b, v27.16b}}, [x9], #64",
            "st1 {{v28.16b, v29.16b, v30.16b, v31.16b}}, [x9]",
            a = in(reg) a.as_ptr(),
            b = in(reg) b.as_mut_ptr(),
            n = inout(reg) n => _,
            y = const syscall::THREAD_YIELD,
            out("x0") _, out("x8") _, out("x9") _,
            out("v0") _, out("v1") _, out("v2") _, out("v3") _, out("v4") _, out("v5") _, out("v6") _, out("v7") _,
            out("v8") _, out("v9") _, out("v10") _, out("v11") _, out("v12") _, out("v13") _, out("v14") _, out("v15") _,
            out("v16") _, out("v17") _, out("v18") _, out("v19") _, out("v20") _, out("v21") _, out("v22") _, out("v23") _,
            out("v24") _, out("v25") _, out("v26") _, out("v27") _, out("v28") _, out("v29") _, out("v30") _, out("v31") _,
            options(nostack)
        );
    }
}

/// Vlaken je vic nez CPU v QEMU testu (4), takze se pri `THREAD_YIELD`
/// opravdu stridaji na jednom CPU a stehuji mezi CPU.
const THREADS: u64 = 6;
const YIELDS: u64 = 300;

fn registers_survive_switch(_: &Startup) -> TestResult {
    let bad = Arc::new(AtomicU64::new(0));
    let mut ws = Vec::new();
    for t in 0..THREADS {
        let bad = bad.clone();
        let w = libos::thread::spawn("fpu-registry", move || {
            let mut a = [0u8; REGS];
            for (i, x) in a.iter_mut().enumerate() {
                *x = (i as u64 * 7 + t * 31 + 1) as u8;
            }
            let mut b = [0u8; REGS];
            hold_regs_across_yields(&a, &mut b, YIELDS);
            if a != b {
                bad.fetch_add(1 << (t * 8), Ordering::SeqCst);
            }
        })
        .map_err(|e| fail("vlakno", e))?;
        ws.push(w);
    }
    for w in ws {
        w.join(deadline()).map_err(|e| fail("konec vlakna", e))?;
    }
    check_eq!(bad.load(Ordering::SeqCst), 0, "vektorove registry po prepnuti (bajt na vlakno)");
    Ok(())
}

/// Soucet 1/k pro k = 1..=n v f64 (stejne poradi operaci jako reference,
/// takze vysledek musi souhlasit bit po bitu).
fn harmonic(n: u32, yield_every: u32) -> f64 {
    let mut s = 0.0f64;
    for k in 1..=n {
        s += 1.0 / f64::from(k);
        if yield_every != 0 && k % yield_every == 0 {
            libos::sys::thread_yield();
        }
    }
    s
}

fn float_math_in_threads(_: &Startup) -> TestResult {
    const N: u32 = 20_000;
    // Reference bez prepinani; H(20000) ~= 10.480728.
    let reference = harmonic(N, 0);
    check!((reference - 10.480_728).abs() < 1e-5, alloc::format!("H({N}) = {reference}"));
    let bits = reference.to_bits();
    let bad = Arc::new(AtomicU64::new(0));
    let mut ws = Vec::new();
    for _ in 0..THREADS {
        let bad = bad.clone();
        let w = libos::thread::spawn("fpu-vypocet", move || {
            if harmonic(N, 500).to_bits() != bits {
                bad.fetch_add(1, Ordering::SeqCst);
            }
        })
        .map_err(|e| fail("vlakno", e))?;
        ws.push(w);
    }
    for w in ws {
        w.join(deadline()).map_err(|e| fail("konec vlakna", e))?;
    }
    check_eq!(bad.load(Ordering::SeqCst), 0, "vlakna se spatnym vysledkem");
    Ok(())
}
