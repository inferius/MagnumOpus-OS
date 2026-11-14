//! AVX s XSAVE (x86_64): horni poloviny YMM preziji prepnuti vlaken
//! a `THREAD_STATE` je cte i zapisuje v obrazu XSAVE. Na CPU bez AVX (nebo
//! na aarch64) se testy preskoci s poznamkou v logu.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use libos::{Startup, println, time};

use super::{TestResult, fail};
use crate::{check, check_eq};

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(20)))
}

/// Umi CPU AVX a zaplo ho jadro v XCR0 (OSXSAVE, AVX, XCR0 bity 1 a 2)?
#[cfg(target_arch = "x86_64")]
fn avx_enabled() -> bool {
    use core::arch::x86_64::__cpuid_count;
    let ecx = __cpuid_count(1, 0).ecx;
    if ecx & (1 << 27) == 0 || ecx & (1 << 28) == 0 {
        return false;
    }
    let (lo, _hi): (u32, u32);
    // SAFETY: XGETBV s ecx = 0 je s OSXSAVE povolene i v userspace.
    unsafe { core::arch::asm!("xgetbv", in("ecx") 0u32, out("eax") lo, out("edx") _hi, options(nomem, nostack)) };
    lo & 0b110 == 0b110
}

#[cfg(not(target_arch = "x86_64"))]
fn avx_enabled() -> bool {
    false
}

/// Offset casti AVX (horni poloviny YMM0-15, 256 B) v obrazu XSAVE.
#[cfg(target_arch = "x86_64")]
fn avx_offset() -> usize {
    core::arch::x86_64::__cpuid_count(0xD, 2).ebx as usize
}

/// YMM0-15 (512 B) z `a`, `n`krat THREAD_YIELD, YMM0-15 do `b` - vse
/// v jednom bloku asm (prekladac AVX nepouziva, cil ho nema).
#[cfg(target_arch = "x86_64")]
fn hold_ymm(a: &[u8; 512], b: &mut [u8; 512], n: u64) {
    // SAFETY: ctou se a pisou jen predane buffery (512 B); AVX je zapnute
    // (`avx_enabled`); syscall meni jen rax, rcx a r11; XMM0-15 (spodni
    // poloviny YMM) jsou oznacene jako zmenene, horni poloviny prekladac
    // nepouziva.
    unsafe {
        core::arch::asm!(
            "vmovdqu ymm0, [rdi]", "vmovdqu ymm1, [rdi+32]", "vmovdqu ymm2, [rdi+64]", "vmovdqu ymm3, [rdi+96]",
            "vmovdqu ymm4, [rdi+128]", "vmovdqu ymm5, [rdi+160]", "vmovdqu ymm6, [rdi+192]", "vmovdqu ymm7, [rdi+224]",
            "vmovdqu ymm8, [rdi+256]", "vmovdqu ymm9, [rdi+288]", "vmovdqu ymm10, [rdi+320]", "vmovdqu ymm11, [rdi+352]",
            "vmovdqu ymm12, [rdi+384]", "vmovdqu ymm13, [rdi+416]", "vmovdqu ymm14, [rdi+448]", "vmovdqu ymm15, [rdi+480]",
            "2:",
            "mov eax, {y}",
            "syscall",
            "dec rdx",
            "jnz 2b",
            "vmovdqu [rsi], ymm0", "vmovdqu [rsi+32], ymm1", "vmovdqu [rsi+64], ymm2", "vmovdqu [rsi+96], ymm3",
            "vmovdqu [rsi+128], ymm4", "vmovdqu [rsi+160], ymm5", "vmovdqu [rsi+192], ymm6", "vmovdqu [rsi+224], ymm7",
            "vmovdqu [rsi+256], ymm8", "vmovdqu [rsi+288], ymm9", "vmovdqu [rsi+320], ymm10", "vmovdqu [rsi+352], ymm11",
            "vmovdqu [rsi+384], ymm12", "vmovdqu [rsi+416], ymm13", "vmovdqu [rsi+448], ymm14", "vmovdqu [rsi+480], ymm15",
            "vzeroupper",
            y = const libos::abi::syscall::THREAD_YIELD,
            in("rdi") a.as_ptr(), in("rsi") b.as_mut_ptr(), inout("rdx") n => _,
            out("rax") _, out("rcx") _, out("r11") _,
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _, out("xmm4") _, out("xmm5") _, out("xmm6") _, out("xmm7") _,
            out("xmm8") _, out("xmm9") _, out("xmm10") _, out("xmm11") _, out("xmm12") _, out("xmm13") _, out("xmm14") _, out("xmm15") _,
            options(nostack)
        );
    }
}

pub fn ymm_survive_switch(_: &Startup) -> TestResult {
    if !avx_enabled() {
        println!("init: test FPU: AVX neni (CPU nebo architektura), preskoceno");
        return Ok(());
    }
    #[cfg(target_arch = "x86_64")]
    {
        let bad = Arc::new(AtomicU64::new(0));
        let mut ws = Vec::new();
        for t in 0..6u64 {
            let bad = bad.clone();
            let w = libos::thread::spawn("fpu-ymm", move || {
                let mut a = [0u8; 512];
                for (i, x) in a.iter_mut().enumerate() {
                    *x = (i as u64 * 13 + t * 37 + 5) as u8;
                }
                let mut b = [0u8; 512];
                hold_ymm(&a, &mut b, 300);
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
        check_eq!(bad.load(Ordering::SeqCst), 0, "YMM po prepnuti (bajt na vlakno)");
    }
    Ok(())
}

/// Vlakno testu `ymm_thread_state`: YMM0 z `IN`, toci THREAD_YIELD, dokud
/// `GO` neni 1, pak YMM0 do `OUT`.
#[cfg(target_arch = "x86_64")]
static GO: AtomicU64 = AtomicU64::new(0);
/// Vlakno ma YMM0 nahrane a toci smycku.
#[cfg(target_arch = "x86_64")]
static READY: AtomicU64 = AtomicU64::new(0);

pub fn ymm_thread_state(_: &Startup) -> TestResult {
    if !avx_enabled() {
        println!("init: test FPU: AVX neni (CPU nebo architektura), preskoceno");
        return Ok(());
    }
    #[cfg(target_arch = "x86_64")]
    {
        use libos::abi::thread::fp::{HEADER, Header, format};
        use libos::abi::thread::{X64_XSAVE_HEADER, kind};
        GO.store(0, Ordering::SeqCst);
        READY.store(0, Ordering::SeqCst);
        let seen = Arc::new(spin::Mutex::new([0u8; 32]));
        let out = seen.clone();
        let w = libos::thread::spawn("fpu-ymm-stav", move || {
            let input = [0xA5u8; 32];
            let mut o = [0u8; 32];
            // SAFETY: AVX je zapnute; cte/zapisuje jen `input`, `o`, `GO` a `READY`;
            // syscall meni rax, rcx, r11.
            unsafe {
                core::arch::asm!(
                    "vmovdqu ymm0, [{i}]",
                    "mov qword ptr [{rd}], 1",
                    "2:",
                    "mov eax, {y}",
                    "syscall",
                    "cmp qword ptr [{go}], 0",
                    "je 2b",
                    "vmovdqu [{o}], ymm0",
                    "vzeroupper",
                    i = in(reg) input.as_ptr(), o = in(reg) o.as_mut_ptr(), go = in(reg) GO.as_ptr(), rd = in(reg) READY.as_ptr(),
                    y = const libos::abi::syscall::THREAD_YIELD,
                    out("rax") _, out("rcx") _, out("r11") _, out("xmm0") _,
                    options(nostack)
                );
            }
            *out.lock() = o;
        })
        .map_err(|e| fail("vlakno", e))?;
        let t = w.thread();
        let end = time::deadline_after(Duration::from_secs(10));
        while READY.load(Ordering::SeqCst) == 0 && time::now() < end {
            libos::sys::thread_yield();
        }
        let r = (|| -> TestResult {
            let token = t.suspend().map_err(|e| fail("THREAD_SUSPEND", e))?;
            t.wait_suspended(deadline()).map_err(|e| fail("SUSPENDED", e))?;
            let mut fp = alloc::vec![0u8; libos::exception::fp_state_size()];
            t.read_state(kind::FP, &mut fp).map_err(|e| fail("cteni FPU", e))?;
            let h = Header::decode(&fp).ok_or("hlavicka FPU")?;
            check_eq!(h.format, format::X64_XSAVE, "format XSAVE");
            check!(h.features & 0b100 != 0, "XCR0 bez AVX");
            let data = &mut fp[HEADER..];
            let bv = u64::from_le_bytes(data[X64_XSAVE_HEADER..X64_XSAVE_HEADER + 8].try_into().unwrap());
            check!(bv & 0b100 != 0, "XSTATE_BV: cast AVX neni ulozena");
            let off = avx_offset();
            check_eq!(&data[off..off + 16], &[0xA5u8; 16][..], "horni polovina YMM0 v obrazu XSAVE");
            // Zapis: jina horni polovina YMM0, vlakno ji uvidi po obnoveni.
            data[off..off + 16].copy_from_slice(&[0x3Cu8; 16]);
            t.write_state(kind::FP, &fp).map_err(|e| fail("zapis FPU", e))?;
            drop(token);
            Ok(())
        })();
        GO.store(1, Ordering::SeqCst);
        w.join(deadline()).map_err(|e| fail("konec vlakna", e))?;
        r?;
        let o = *seen.lock();
        check_eq!(&o[..16], &[0xA5u8; 16][..], "spodni polovina YMM0 beze zmeny");
        check_eq!(&o[16..], &[0x3Cu8; 16][..], "horni polovina YMM0 po zapisu THREAD_STATE");
    }
    Ok(())
}
