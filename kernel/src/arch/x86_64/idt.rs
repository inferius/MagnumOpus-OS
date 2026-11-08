//! IDT, obsluha vyjimek CPU a preruseni.
//!
//! Kazdy z 256 vektoru ma maly vstupni stub, ktery sjednoti zasobnik
//! (doplni nulovy chybovy kod tam, kde ho CPU nevklada, a cislo vektoru)
//! a skoci do spolecne casti. Ta ulozi vsechny registry do [`TrapFrame`]
//! a zavola Rust funkci. Stejny ramec pozdeji poslouzi i pro syscally.

use core::arch::{asm, global_asm, naked_asm};
use core::mem::size_of;

use super::apic::{
    self, VECTOR_HALT, VECTOR_RESCHEDULE, VECTOR_SPURIOUS, VECTOR_TIMER, VECTOR_TLB_FLUSH,
};
use super::gdt::{IST_DOUBLE_FAULT, IST_MACHINE_CHECK, IST_NMI, KERNEL_CODE};
use super::user::from_user;
use super::{cpu, halt_forever};
use crate::fatal::{fatal_stop, report, report_stack_trace};
use crate::{log, trap};

/// Ulozeny stav CPU v okamziku vyjimky. Poradi odpovida `isr_common`.
#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    // Ulozeno procesorem:
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl IdtEntry {
    const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        type_attr: 0,
        offset_mid: 0,
        offset_high: 0,
        _reserved: 0,
    };

    fn interrupt_gate(handler: u64, ist: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector: KERNEL_CODE,
            ist,
            type_attr: 0x8E, // P, DPL 0, 64bitova interrupt gate (zakaze IF)
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::MISSING; 256];

/// Spolecna cast vstupu: ulozi registry, zavola `trap_dispatch`, obnovi je.
///
/// Zarovnani: CPU zarovna RSP na 16 a ulozi 5 slov, stub prida 2, tady 15
/// -> 22 slov = 176 B, takze pred `call` je RSP zarovnany na 16 podle ABI.
#[unsafe(naked)]
extern "sysv64" fn isr_common() {
    naked_asm!(
        // Preruseni z userspace (CPL 3 v ulozenem CS na [rsp + 24]):
        // prepnout GS na per-CPU data jadra.
        "test byte ptr [rsp + 24], 3",
        "jz 2f",
        "swapgs",
        "2:",
        "push rax", "push rbx", "push rcx", "push rdx", "push rsi", "push rdi", "push rbp",
        "push r8", "push r9", "push r10", "push r11", "push r12", "push r13", "push r14", "push r15",
        "mov rdi, rsp",
        "cld",
        "call {dispatch}",
        "pop r15", "pop r14", "pop r13", "pop r12", "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rbp", "pop rdi", "pop rsi", "pop rdx", "pop rcx", "pop rbx", "pop rax",
        // Navrat do userspace: vratit GS userspace.
        "test byte ptr [rsp + 24], 3",
        "jz 3f",
        "swapgs",
        "3:",
        "add rsp, 16", // vektor + chybovy kod
        "iretq",
        dispatch = sym trap_dispatch,
    )
}

/// Tabulka 256 stubu po 16 bajtech. CPU vklada chybovy kod jen u vektoru
/// 8, 10-14, 17, 21, 29 a 30; ostatnim stub doplni nulu.
const STUB_SIZE: u64 = 16;
global_asm!(
    ".pushsection .text.isr_stubs, \"ax\"",
    ".balign 16",
    ".global __isr_stubs",
    "__isr_stubs:",
    ".set vec, 0",
    ".rept 256",
    ".balign 16",
    ".if !(vec == 8 || (vec >= 10 && vec <= 14) || vec == 17 || vec == 21 || vec == 29 || vec == 30)",
    "push 0",
    ".endif",
    "push vec",
    "jmp {common}",
    ".set vec, vec + 1",
    ".endr",
    ".popsection",
    common = sym isr_common,
);

unsafe extern "C" {
    static __isr_stubs: u8;
}

const EXCEPTION_NAMES: [&str; 32] = [
    "Divide Error (#DE)",
    "Debug (#DB)",
    "Non-Maskable Interrupt (NMI)",
    "Breakpoint (#BP)",
    "Overflow (#OF)",
    "Bound Range Exceeded (#BR)",
    "Invalid Opcode (#UD)",
    "Device Not Available (#NM)",
    "Double Fault (#DF)",
    "Coprocessor Segment Overrun",
    "Invalid TSS (#TS)",
    "Segment Not Present (#NP)",
    "Stack-Segment Fault (#SS)",
    "General Protection (#GP)",
    "Page Fault (#PF)",
    "Reserved 15",
    "x87 Floating-Point (#MF)",
    "Alignment Check (#AC)",
    "Machine Check (#MC)",
    "SIMD Floating-Point (#XM)",
    "Virtualization (#VE)",
    "Control Protection (#CP)",
    "Reserved 22",
    "Reserved 23",
    "Reserved 24",
    "Reserved 25",
    "Reserved 26",
    "Reserved 27",
    "Hypervisor Injection (#HV)",
    "VMM Communication (#VC)",
    "Security (#SX)",
    "Reserved 31",
];

/// Rozsah vektoru pro preruseni zarizeni (nize jsou vyjimky a vektory
/// jadra `apic::VECTOR_*`, nahore spurious).
pub const DEVICE_VECTOR_FIRST: u8 = 48;
pub const DEVICE_VECTOR_LAST: u8 = 239;

const VEC_NMI: u64 = 2;
const VEC_BREAKPOINT: u64 = 3;
const VEC_DOUBLE_FAULT: u64 = 8;
const VEC_PAGE_FAULT: u64 = 14;
const VEC_MACHINE_CHECK: u64 = 18;

/// Naplni IDT (jedna tabulka pro vsechna CPU). Volat jednou na BSP.
///
/// # Safety
///
/// Volat jednou, pred `load` na jakemkoli CPU.
pub unsafe fn init() {
    let stubs = &raw const __isr_stubs as u64;
    // SAFETY: podle kontraktu k IDT nikdo jiny soucasne nepristupuje.
    unsafe {
        let idt = &raw mut IDT;
        for vector in 0..256u64 {
            let ist = match vector {
                VEC_DOUBLE_FAULT => IST_DOUBLE_FAULT,
                VEC_NMI => IST_NMI,
                VEC_MACHINE_CHECK => IST_MACHINE_CHECK,
                _ => 0,
            };
            (*idt)[vector as usize] = IdtEntry::interrupt_gate(stubs + vector * STUB_SIZE, ist);
        }
    }
}

/// Nahraje IDT na aktualnim CPU.
///
/// # Safety
///
/// Volat po `init` a po nahrani GDT/TSS tohoto CPU (IST).
pub unsafe fn load() {
    let ptr = DescriptorTablePointer {
        limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
        base: &raw const IDT as u64,
    };
    // SAFETY: IDT je staticka a naplnena v `init`.
    unsafe { asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags)) };
}

extern "sysv64" fn trap_dispatch(frame: &mut TrapFrame) {
    let vector = frame.vector;
    let user = from_user(frame);
    match vector {
        // Vyjimka v userspace (vcetne int3): ukonci proces, ne jadro.
        v if v < 32 && user => {
            let addr = if v == VEC_PAGE_FAULT {
                cpu::read_cr2()
            } else {
                0
            };
            trap::user_fault(v as u8, frame.error_code, frame.rip, addr, frame.rsp, frame.rbp)
        }
        VEC_BREAKPOINT => {
            log::info!("breakpoint na {:#x}", frame.rip);
        }
        v if v < 32 => fatal_exception(frame),
        v if v == VECTOR_TIMER as u64 => {
            apic::eoi();
            trap::timer_interrupt();
        }
        v if v == VECTOR_RESCHEDULE as u64 => {
            apic::eoi();
            trap::reschedule_ipi();
        }
        v if v == VECTOR_TLB_FLUSH as u64 => {
            apic::eoi();
            trap::tlb_flush_ipi();
        }
        v if v == VECTOR_HALT as u64 => halt_forever(),
        v if v == super::apic::VECTOR_IOMMU_FAULT as u64 => {
            apic::eoi();
            super::iommu::on_fault_interrupt();
        }
        v if v == VECTOR_SPURIOUS as u64 => {} // spurious se nepotvrzuje
        v if (DEVICE_VECTOR_FIRST as u64..=DEVICE_VECTOR_LAST as u64).contains(&v) => {
            apic::eoi();
            trap::device_irq(v as u32);
        }
        _ => {
            apic::eoi();
            log::warn!("necekane preruseni, vektor {vector}");
        }
    }
    if vector >= 32 {
        trap::after_interrupt();
    }
    if user {
        trap::return_to_user();
    }
}

fn fatal_exception(f: &TrapFrame) -> ! {
    let name = EXCEPTION_NAMES[f.vector as usize];
    crate::fatal::set_reason(match f.vector {
        VEC_PAGE_FAULT => "PAGE_FAULT_KERNEL",
        8 => "DOUBLE_FAULT",
        13 => "GENERAL_PROTECTION",
        6 => "INVALID_OPCODE",
        0 => "DIVIDE_ERROR",
        _ => "CPU_EXCEPTION",
    });
    report(format_args!(
        "KERNEL: neosetrena vyjimka {name} na cpu{}, vektor {}, chybovy kod {:#x}",
        trap::current_cpu_index(),
        f.vector,
        f.error_code
    ));
    if f.vector == VEC_PAGE_FAULT {
        let e = f.error_code;
        report(format_args!(
            "  adresa {:#x}: {}, {}, {}{}",
            cpu::read_cr2(),
            if e & 1 != 0 {
                "poruseni prav"
            } else {
                "stranka neni namapovana"
            },
            if e & 2 != 0 { "zapis" } else { "cteni" },
            if e & 4 != 0 { "user" } else { "kernel" },
            if e & 16 != 0 {
                ", nacteni instrukce"
            } else {
                ""
            },
        ));
    }
    report(format_args!(
        "  rip {:#018x}  cs {:#x}  rflags {:#x}",
        f.rip, f.cs, f.rflags
    ));
    report(format_args!("  rsp {:#018x}  ss {:#x}", f.rsp, f.ss));
    report(format_args!(
        "  rax {:#018x}  rbx {:#018x}  rcx {:#018x}",
        f.rax, f.rbx, f.rcx
    ));
    report(format_args!(
        "  rdx {:#018x}  rsi {:#018x}  rdi {:#018x}",
        f.rdx, f.rsi, f.rdi
    ));
    report(format_args!(
        "  rbp {:#018x}  r8  {:#018x}  r9  {:#018x}",
        f.rbp, f.r8, f.r9
    ));
    report(format_args!(
        "  r10 {:#018x}  r11 {:#018x}  r12 {:#018x}",
        f.r10, f.r11, f.r12
    ));
    report(format_args!(
        "  r13 {:#018x}  r14 {:#018x}  r15 {:#018x}",
        f.r13, f.r14, f.r15
    ));
    report_stack_trace(Some(f.rbp));
    fatal_stop()
}
