//! GDT a TSS.
//!
//! V long mode slouzi segmentace jen k urceni urovne opravneni a TSS jen
//! k prepnuti zasobniku pri preruseni (RSP0 a IST). Poradi deskriptoru
//! odpovida pozadavkum instrukce SYSRET (user data tesne pred user code),
//! aby se pozdeji nemuselo menit.
//!
//! TODO(milnik 3): GDT a TSS pro kazde CPU zvlast.

use core::arch::asm;
use core::mem::size_of;

pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
#[allow(dead_code)]
pub const USER_DATA: u16 = 0x18 | 3;
#[allow(dead_code)]
pub const USER_CODE: u16 = 0x20 | 3;
const TSS_SELECTOR: u16 = 0x28;

/// Indexy IST (1-based, jak je ocekava IDT).
pub const IST_DOUBLE_FAULT: u8 = 1;
pub const IST_NMI: u8 = 2;
pub const IST_MACHINE_CHECK: u8 = 3;

const IST_STACK_SIZE: usize = 16 * 1024;

#[repr(C, packed(4))]
struct Tss {
    _reserved0: u32,
    rsp: [u64; 3],
    _reserved1: u64,
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}

#[repr(C, align(16))]
struct Stack([u8; IST_STACK_SIZE]);

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

static mut TSS: Tss = Tss {
    _reserved0: 0,
    rsp: [0; 3],
    _reserved1: 0,
    ist: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    // Za koncem TSS = zadna I/O permission bitmapa.
    iomap_base: size_of::<Tss>() as u16,
};

static mut GDT: [u64; 7] = [
    0,                     // null
    0x00AF_9A00_0000_FFFF, // kernel code: P, DPL 0, execute/read, L
    0x00CF_9200_0000_FFFF, // kernel data: P, DPL 0, read/write
    0x00CF_F200_0000_FFFF, // user data:   P, DPL 3, read/write
    0x00AF_FA00_0000_FFFF, // user code:   P, DPL 3, execute/read, L
    0,                     // TSS (dolni polovina), doplni se v `init`
    0,                     // TSS (horni polovina)
];

static mut IST_STACKS: [Stack; 3] = [const { Stack([0; IST_STACK_SIZE]) }; 3];

/// Nahraje GDT a TSS na aktualnim CPU.
///
/// # Safety
///
/// Volat jednou na bootovacim CPU, pred povolenim preruseni a pred
/// spustenim dalsich CPU (pracuje se statickymi tabulkami bez zamku).
pub unsafe fn init() {
    // SAFETY: podle kontraktu funkce k techto statickym promennym nikdo
    // jiny soucasne nepristupuje; pouzivaji se jen surove ukazatele.
    unsafe {
        let stacks = &raw mut IST_STACKS;
        let tss = &raw mut TSS;
        for (i, ist) in [IST_DOUBLE_FAULT, IST_NMI, IST_MACHINE_CHECK]
            .into_iter()
            .enumerate()
        {
            let top = (&raw mut (*stacks)[i]) as u64 + IST_STACK_SIZE as u64;
            (*tss).ist[ist as usize - 1] = top;
        }

        let base = tss as u64;
        let limit = (size_of::<Tss>() - 1) as u64;
        let gdt = &raw mut GDT;
        (*gdt)[5] = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x89 << 40) // P, typ 9 = dostupny 64bitovy TSS
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        (*gdt)[6] = base >> 32;

        let ptr = DescriptorTablePointer {
            limit: (size_of::<[u64; 7]>() - 1) as u16,
            base: gdt as u64,
        };

        // Nacteni GDT, prepnuti CS pres far return a nastaveni datovych
        // segmentu. FS/GS se nuluji, jejich bazi budou pozdeji urcovat MSR.
        asm!(
            "lgdt [{ptr}]",
            "push {code}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov ss, {data:x}",
            "xor {tmp:e}, {tmp:e}",
            "mov fs, {tmp:x}",
            "mov gs, {tmp:x}",
            "ltr {tss:x}",
            ptr = in(reg) &ptr,
            code = const KERNEL_CODE as u64,
            data = in(reg) KERNEL_DATA as u64,
            tss = in(reg) TSS_SELECTOR as u64,
            tmp = out(reg) _,
            options(preserves_flags),
        );
    }
}
