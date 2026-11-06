//! IDT a obsluha vyjimek CPU.
//!
//! Kazdy vektor ma maly vstupni stub (naked funkce), ktery sjednoti zasobnik
//! (doplni nulovy chybovy kod tam, kde ho CPU nevklada, a cislo vektoru)
//! a skoci do spolecne casti. Ta ulozi vsechny registry do [`TrapFrame`]
//! a zavola Rust funkci. Stejny ramec pozdeji poslouzi pro preruseni,
//! prepinani kontextu a syscally.

use core::arch::{asm, naked_asm};
use core::mem::size_of;

use super::gdt::{IST_DOUBLE_FAULT, IST_MACHINE_CHECK, IST_NMI, KERNEL_CODE};
use super::{cpu, stack_trace};
use crate::console;

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
        "push rax", "push rbx", "push rcx", "push rdx", "push rsi", "push rdi", "push rbp",
        "push r8", "push r9", "push r10", "push r11", "push r12", "push r13", "push r14", "push r15",
        "mov rdi, rsp",
        "cld",
        "call {dispatch}",
        "pop r15", "pop r14", "pop r13", "pop r12", "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rbp", "pop rdi", "pop rsi", "pop rdx", "pop rcx", "pop rbx", "pop rax",
        "add rsp, 16", // vektor + chybovy kod
        "iretq",
        dispatch = sym trap_dispatch,
    )
}

macro_rules! stubs {
    ($( $name:ident = $vec:literal $kind:ident ),* $(,)?) => {
        $( stubs!(@one $name, $vec, $kind); )*
        const STUBS: [extern "sysv64" fn(); 32] = [$($name),*];
    };
    (@one $name:ident, $vec:literal, noerr) => {
        #[unsafe(naked)]
        extern "sysv64" fn $name() {
            naked_asm!("push 0", "push {v}", "jmp {c}", v = const $vec, c = sym isr_common)
        }
    };
    (@one $name:ident, $vec:literal, err) => {
        #[unsafe(naked)]
        extern "sysv64" fn $name() {
            naked_asm!("push {v}", "jmp {c}", v = const $vec, c = sym isr_common)
        }
    };
}

// Vektory, pro ktere CPU vklada chybovy kod: 8, 10-14, 17, 21, 29, 30.
stubs! {
    isr0 = 0 noerr, isr1 = 1 noerr, isr2 = 2 noerr, isr3 = 3 noerr,
    isr4 = 4 noerr, isr5 = 5 noerr, isr6 = 6 noerr, isr7 = 7 noerr,
    isr8 = 8 err, isr9 = 9 noerr, isr10 = 10 err, isr11 = 11 err,
    isr12 = 12 err, isr13 = 13 err, isr14 = 14 err, isr15 = 15 noerr,
    isr16 = 16 noerr, isr17 = 17 err, isr18 = 18 noerr, isr19 = 19 noerr,
    isr20 = 20 noerr, isr21 = 21 err, isr22 = 22 noerr, isr23 = 23 noerr,
    isr24 = 24 noerr, isr25 = 25 noerr, isr26 = 26 noerr, isr27 = 27 noerr,
    isr28 = 28 noerr, isr29 = 29 err, isr30 = 30 err, isr31 = 31 noerr,
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

const VEC_NMI: u64 = 2;
const VEC_BREAKPOINT: u64 = 3;
const VEC_DOUBLE_FAULT: u64 = 8;
const VEC_PAGE_FAULT: u64 = 14;
const VEC_MACHINE_CHECK: u64 = 18;

/// Nahraje IDT s obsluhou vyjimek na aktualnim CPU.
///
/// # Safety
///
/// Volat po `gdt::init` (pouziva IST z TSS), na bootovacim CPU, se
/// zakazanymi prerusenimi a pred spustenim dalsich CPU.
pub unsafe fn init() {
    // SAFETY: podle kontraktu k IDT nikdo jiny soucasne nepristupuje.
    unsafe {
        let idt = &raw mut IDT;
        for (vector, stub) in STUBS.iter().enumerate() {
            let ist = match vector as u64 {
                VEC_DOUBLE_FAULT => IST_DOUBLE_FAULT,
                VEC_NMI => IST_NMI,
                VEC_MACHINE_CHECK => IST_MACHINE_CHECK,
                _ => 0,
            };
            (*idt)[vector] = IdtEntry::interrupt_gate(*stub as usize as u64, ist);
        }
        let ptr = DescriptorTablePointer {
            limit: (size_of::<[IdtEntry; 256]>() - 1) as u16,
            base: idt as u64,
        };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}

extern "sysv64" fn trap_dispatch(frame: &mut TrapFrame) {
    match frame.vector {
        VEC_BREAKPOINT => {
            crate::kprintln!("breakpoint na {:#x}", frame.rip);
        }
        v if v < 32 => fatal_exception(frame),
        _ => {}
    }
}

fn fatal_exception(f: &TrapFrame) -> ! {
    let name = EXCEPTION_NAMES[f.vector as usize];
    console::panic_print(format_args!(
        "\nKERNEL: neosetrena vyjimka {name}, vektor {}, chybovy kod {:#x}\n",
        f.vector, f.error_code
    ));
    if f.vector == VEC_PAGE_FAULT {
        let e = f.error_code;
        console::panic_print(format_args!(
            "  adresa {:#x}: {}, {}, {}{}\n",
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
    console::panic_print(format_args!(
        "  rip {:#018x}  cs {:#x}  rflags {:#x}\n  rsp {:#018x}  ss {:#x}\n\
         \x20 rax {:#018x}  rbx {:#018x}  rcx {:#018x}\n\
         \x20 rdx {:#018x}  rsi {:#018x}  rdi {:#018x}\n\
         \x20 rbp {:#018x}  r8  {:#018x}  r9  {:#018x}\n\
         \x20 r10 {:#018x}  r11 {:#018x}  r12 {:#018x}\n\
         \x20 r13 {:#018x}  r14 {:#018x}  r15 {:#018x}\n",
        f.rip,
        f.cs,
        f.rflags,
        f.rsp,
        f.ss,
        f.rax,
        f.rbx,
        f.rcx,
        f.rdx,
        f.rsi,
        f.rdi,
        f.rbp,
        f.r8,
        f.r9,
        f.r10,
        f.r11,
        f.r12,
        f.r13,
        f.r14,
        f.r15
    ));
    stack_trace(f.rbp, |addr| {
        console::panic_print(format_args!("    {addr:#018x}\n"))
    });
    crate::fatal_stop()
}
