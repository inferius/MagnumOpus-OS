//! Prechody mezi jadrem a userspace (ring 3).
//!
//! - Vstup syscallu: instrukce `syscall` skoci na `syscall_entry`. Ta
//!   prepne GS na per-CPU data jadra (`swapgs`), prepne na zasobnik jadra
//!   aktualniho vlakna a ulozi registry do stejneho [`TrapFrame`] jako
//!   preruseni. Zpet se vraci pres `iretq` (pomalejsi nez `sysret`, ale bez
//!   jeho zaludnosti s nekanonickou navratovou adresou; optimalizace pozdeji).
//! - Prvni vstup vlakna do userspace: [`enter_user`].
//! - Per-CPU data pro tyto prechody ([`CpuLocal`]) lezi v `PerCpu` na
//!   pevnem offsetu, aby k nim asembler pristupoval pres `gs:[...]`.

use core::arch::{asm, naked_asm};

use super::cpu;
use super::gdt::{self, USER_CODE, USER_DATA};
use super::idt::TrapFrame;

/// Offset [`CpuLocal`] v generickych per-CPU datech (hned za ukazatelem na
/// sebe). Hlida ho staticka kontrola v `sched::percpu`.
pub const CPU_LOCAL_OFFSET: usize = 8;
const KERNEL_RSP: usize = CPU_LOCAL_OFFSET;
const USER_RSP: usize = CPU_LOCAL_OFFSET + 8;

/// Hodnota `TrapFrame::vector` pro syscall (neni to skutecny vektor).
pub const SYSCALL_VECTOR: u64 = 0x100;

/// Architekturne specificka per-CPU data.
#[repr(C)]
#[derive(Debug)]
pub struct CpuLocal {
    /// Vrchol zasobniku jadra aktualniho vlakna (pro vstup syscallu).
    kernel_rsp: u64,
    /// Odlozeny ukazatel zasobniku userspace pri vstupu syscallu.
    user_rsp: u64,
    /// Adresa TSS tohoto CPU (RSP0 pro preruseni z userspace).
    tss: u64,
}

impl crate::arch::api::CpuLocalData for CpuLocal {
    /// Data pro aktualni CPU (volat po nahrani GDT a TSS).
    fn for_this_cpu() -> Self {
        Self {
            kernel_rsp: 0,
            user_rsp: 0,
            tss: gdt::current_tss(),
        }
    }
}

const MSR_EFER: u32 = 0xC000_0080;
const MSR_STAR: u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_FMASK: u32 = 0xC000_0084;
const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;
const EFER_SCE: u64 = 1 << 0;

/// Priznaky RFLAGS, ktere `syscall` vynuluje: IF (preruseni zapne az jadro
/// po prepnuti zasobniku), DF, TF, NT, AC.
const SYSCALL_FMASK: u64 = (1 << 9) | (1 << 10) | (1 << 8) | (1 << 14) | (1 << 18);

/// Zapne instrukci `syscall` na aktualnim CPU.
///
/// # Safety
///
/// Volat jednou pri startu CPU, po nahrani GDT.
pub unsafe fn init_cpu() {
    // STAR: [47:32] = CS jadra pro syscall (SS = CS + 8),
    //       [63:48] = baze pro sysret (SS = baze + 8, CS = baze + 16);
    //       sysret zatim nepouzivame, ale hodnota odpovida GDT.
    let star = ((gdt::KERNEL_DATA as u64 | 3) << 48) | ((gdt::KERNEL_CODE as u64) << 32);
    // SAFETY: MSR syscallu existuji na kazdem x86_64 CPU; LSTAR ukazuje na
    // platny vstupni bod; KERNEL_GS_BASE = 0 je GS userspace pri prvnim
    // `swapgs` do userspace.
    unsafe {
        cpu::wrmsr(MSR_STAR, star);
        cpu::wrmsr(MSR_LSTAR, syscall_entry as *const () as u64);
        cpu::wrmsr(MSR_FMASK, SYSCALL_FMASK);
        cpu::wrmsr(MSR_KERNEL_GS_BASE, 0);
        cpu::wrmsr(MSR_EFER, cpu::rdmsr(MSR_EFER) | EFER_SCE);
    }
}

/// Nastavi zasobnik jadra, na ktery CPU prepne pri vstupu z userspace
/// (preruseni pres TSS.RSP0, syscall pres per-CPU data).
///
/// # Safety
///
/// Volat na vlastnim CPU se zakazanymi prerusenimi; `local` jsou data
/// tohoto CPU.
pub unsafe fn set_kernel_stack(local: &CpuLocal, top: u64) {
    // SAFETY: `tss` je adresa TSS tohoto CPU; RSP0 lezi na offsetu 4
    // (TSS je packed, proto zapis bez zarovnani). Per-CPU data patri
    // tomuto CPU a preruseni jsou zakazana.
    unsafe {
        ((local.tss + 4) as *mut u64).write_unaligned(top);
        let p = local as *const CpuLocal as *mut CpuLocal;
        (*p).kernel_rsp = top;
    }
}

/// Prvni vstup aktualniho vlakna do userspace. `arg0` a `arg1` dostane
/// program v `rdi` a `rsi`.
///
/// # Safety
///
/// Aktivni adresni prostor musi byt adresni prostor procesu vlakna
/// a zasobnik jadra pro navrat z userspace musi byt nastaveny
/// (`set_kernel_stack`).
pub unsafe fn enter_user(entry: u64, stack: u64, arg0: u64, arg1: u64) -> ! {
    // SAFETY: ramec pro iretq: SS, RSP, RFLAGS (IF=1), CS, RIP. Ostatni
    // registry se nuluji, aby do userspace neunikla data jadra. swapgs
    // nastavi GS userspace (KERNEL_GS_BASE = per-CPU data jadra).
    unsafe {
        asm!(
            "cli",
            "push {ss}",
            "push {stack}",
            "push 0x202",
            "push {cs}",
            "push {entry}",
            "xor eax, eax", "xor ebx, ebx", "xor ecx, ecx", "xor edx, edx",
            "xor ebp, ebp", "xor r8d, r8d", "xor r9d, r9d", "xor r10d, r10d",
            "xor r11d, r11d", "xor r12d, r12d", "xor r13d, r13d", "xor r14d, r14d",
            "xor r15d, r15d",
            "swapgs",
            "iretq",
            ss = const USER_DATA as u64,
            cs = const USER_CODE as u64,
            stack = in(reg) stack,
            entry = in(reg) entry,
            in("rdi") arg0,
            in("rsi") arg1,
            options(noreturn)
        )
    }
}

/// Vstupni bod instrukce `syscall`. Registry: rax = cislo, rdi, rsi, rdx,
/// r10, r8, r9 = argumenty, rcx = navratova adresa, r11 = RFLAGS.
#[unsafe(naked)]
extern "sysv64" fn syscall_entry() {
    naked_asm!(
        "swapgs",
        "mov gs:[{user_rsp}], rsp",
        "mov rsp, gs:[{kernel_rsp}]",
        // Ramec ve tvaru jako od preruseni (SS, RSP, RFLAGS, CS, RIP, kod, vektor).
        "push {ss}",
        "push qword ptr gs:[{user_rsp}]",
        "push r11",
        "push {cs}",
        "push rcx",
        "push 0",
        "push {vector}",
        "push rax", "push rbx", "push rcx", "push rdx", "push rsi", "push rdi", "push rbp",
        "push r8", "push r9", "push r10", "push r11", "push r12", "push r13", "push r14", "push r15",
        "mov rdi, rsp",
        "cld",
        "call {dispatch}",
        "cli",
        "pop r15", "pop r14", "pop r13", "pop r12", "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rbp", "pop rdi", "pop rsi", "pop rdx", "pop rcx", "pop rbx", "pop rax",
        "add rsp, 16",
        "swapgs",
        "iretq",
        user_rsp = const USER_RSP,
        kernel_rsp = const KERNEL_RSP,
        ss = const USER_DATA as u64,
        cs = const USER_CODE as u64,
        vector = const SYSCALL_VECTOR,
        dispatch = sym syscall_dispatch,
    )
}

extern "sysv64" fn syscall_dispatch(frame: &mut TrapFrame) {
    // Zasobnik je prepnuty, registry ulozene: preruseni uz smi prijit.
    super::interrupts::enable();
    let args = [
        frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
    ];
    frame.rax = crate::trap::syscall(frame.rax, args);
    super::interrupts::disable();
    crate::trap::return_to_user();
}

/// Vrati true, pokud ramec patri preruseni z userspace (CPL 3).
pub fn from_user(frame: &TrapFrame) -> bool {
    frame.cs & 3 == 3
}
