//! GDT a TSS (pro kazde CPU zvlast).
//!
//! V long mode slouzi segmentace jen k urceni urovne opravneni a TSS jen
//! k prepnuti zasobniku pri preruseni (RSP0 a IST). Poradi deskriptoru
//! odpovida pozadavkum instrukce SYSRET (user data tesne pred user code),
//! aby se pozdeji nemuselo menit.

use alloc::alloc::{Layout, alloc_zeroed};
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

const GDT_TEMPLATE: [u64; 7] = [
    0,                     // null
    0x00AF_9A00_0000_FFFF, // kernel code: P, DPL 0, execute/read, L
    0x00CF_9200_0000_FFFF, // kernel data: P, DPL 0, read/write
    0x00CF_F200_0000_FFFF, // user data:   P, DPL 3, read/write
    0x00AF_FA00_0000_FFFF, // user code:   P, DPL 3, execute/read, L
    0,                     // TSS (dolni polovina), doplni se v `load`
    0,                     // TSS (horni polovina)
];

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

/// GDT, TSS a IST zasobniky jednoho CPU. Platne jsou i vynulovane
/// (vse potrebne doplni `load`), takze je lze alokovat `alloc_zeroed`
/// bez stavby velke struktury na zasobniku.
#[repr(C)]
pub struct CpuTables {
    gdt: [u64; 7],
    tss: Tss,
    ist_stacks: [Stack; 3],
}

#[repr(C, packed)]
struct DescriptorTablePointer {
    limit: u16,
    base: u64,
}

/// Tabulky bootovaciho CPU (pouzivaji se driv, nez existuje heap).
static mut BSP_TABLES: CpuTables = CpuTables {
    gdt: [0; 7],
    tss: Tss {
        _reserved0: 0,
        rsp: [0; 3],
        _reserved1: 0,
        ist: [0; 7],
        _reserved2: 0,
        _reserved3: 0,
        iomap_base: 0,
    },
    ist_stacks: [const { Stack([0; IST_STACK_SIZE]) }; 3],
};

/// Nahraje GDT a TSS bootovaciho CPU.
///
/// # Safety
///
/// Volat jednou, na bootovacim CPU, pred povolenim preruseni.
pub unsafe fn init_bsp() {
    // SAFETY: BSP_TABLES pouziva jen bootovaci CPU, inicializuje se jednou.
    unsafe { load(&raw mut BSP_TABLES) }
}

/// Alokuje a nahraje GDT a TSS pro aplikacni procesor.
///
/// # Safety
///
/// Volat jednou na kazdem AP, pred povolenim preruseni.
pub unsafe fn init_ap() {
    let layout = Layout::new::<CpuTables>();
    // SAFETY: layout ma nenulovou velikost; vynulovane CpuTables jsou platne.
    let tables = unsafe { alloc_zeroed(layout) } as *mut CpuTables;
    assert!(!tables.is_null(), "nedostatek pameti pro GDT/TSS");
    // SAFETY: tabulky patri jen tomuto CPU a zustanou alokovane navzdy.
    unsafe { load(tables) }
}

/// # Safety
///
/// `tables` musi zustat platne navzdy a nesmi je pouzivat jine CPU.
unsafe fn load(tables: *mut CpuTables) {
    // SAFETY: podle kontraktu mame k tabulkam vyhradni pristup.
    unsafe {
        let t = &mut *tables;
        t.gdt = GDT_TEMPLATE;
        // Za koncem TSS = zadna I/O permission bitmapa.
        t.tss.iomap_base = size_of::<Tss>() as u16;
        for (i, ist) in [IST_DOUBLE_FAULT, IST_NMI, IST_MACHINE_CHECK]
            .into_iter()
            .enumerate()
        {
            let top = (&raw mut t.ist_stacks[i]) as u64 + IST_STACK_SIZE as u64;
            t.tss.ist[ist as usize - 1] = top;
        }

        let base = &raw const t.tss as u64;
        let limit = (size_of::<Tss>() - 1) as u64;
        t.gdt[5] = (limit & 0xFFFF)
            | ((base & 0xFF_FFFF) << 16)
            | (0x89 << 40) // P, typ 9 = dostupny 64bitovy TSS
            | (((limit >> 16) & 0xF) << 48)
            | (((base >> 24) & 0xFF) << 56);
        t.gdt[6] = base >> 32;

        let ptr = DescriptorTablePointer {
            limit: (size_of::<[u64; 7]>() - 1) as u16,
            base: &raw const t.gdt as u64,
        };

        // Nacteni GDT, prepnuti CS pres far return a nastaveni datovych
        // segmentu. FS/GS selektory se nuluji; jejich baze nastavuji MSR
        // (GS = per-CPU data jadra, nastavuje se az po tomto volani).
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
