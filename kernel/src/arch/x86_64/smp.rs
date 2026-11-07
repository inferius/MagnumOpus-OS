//! Start aplikacnich procesoru (AP).
//!
//! AP startuje v realnem modu na adrese pod 1 MiB (INIT-SIPI-SIPI).
//! Trampolina ho prevede pres chraneny rezim do long modu, nacte tabulky
//! stranek jadra, nastavi zasobnik a skoci do Rust kodu.
//!
//! Trampolina je pozicne nezavisla: bazi (fyzickou adresu stranky) si
//! spocita z CS a vsechna data cte relativne k ni. Absolutni adresy pro
//! dlouhe skoky a GDT doplni `Trampoline::new` pri kopirovani.

use core::arch::global_asm;

use boot_info::{BootInfo, MemoryKind};

use super::{apic, cpu, time};
use crate::arch::api::CpuStarter;
use crate::log;
use crate::mm::{self, PhysAddr};

global_asm!(
    r#"
    .pushsection .rodata.ap_trampoline, "a"
    .global __ap_tr_start
__ap_tr_start:
    .code16
    cli
    cld
    movw %cs, %ax
    movw %ax, %ds
    movzwl %ax, %ebx
    shll $4, %ebx                       # ebx = fyzicka baze trampoliny
    lgdtl (__ap_tr_gdt_ptr - __ap_tr_start)
    movl %cr0, %eax
    andl $0x9FFFFFFF, %eax              # CD = NW = 0 (po INIT je cache vypnuta)
    orl $0x21, %eax                     # PE | NE (vyjimky x87 jako #MF)
    movl %eax, %cr0
    ljmpl *(__ap_tr_far32 - __ap_tr_start)

    .code32
    .global __ap_tr_prot32
__ap_tr_prot32:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl %cr4, %eax
    orl $0xA0, %eax                     # PAE | PGE
    movl %eax, %cr4
    movl (__ap_tr_pml4 - __ap_tr_start)(%ebx), %eax
    movl %eax, %cr3                     # docasna PML4 pod 4 GiB
    movl $0xC0000080, %ecx              # EFER
    rdmsr
    orl $0x100, %eax                    # LME
    orl (__ap_tr_efer - __ap_tr_start)(%ebx), %eax
    wrmsr
    movl %cr0, %eax
    orl $0x80010001, %eax               # PG | WP | PE
    movl %eax, %cr0
    ljmpl *(__ap_tr_far64 - __ap_tr_start)(%ebx)

    .code64
    .global __ap_tr_long64
__ap_tr_long64:
    movl %ebx, %ebx                     # horni polovina rbx je nedefinovana
    movq (__ap_tr_cr3 - __ap_tr_start)(%rbx), %rax
    movq %rax, %cr3                     # skutecne tabulky jadra
    movq (__ap_tr_stack - __ap_tr_start)(%rbx), %rsp
    movq (__ap_tr_arg - __ap_tr_start)(%rbx), %rdi
    movq (__ap_tr_entry - __ap_tr_start)(%rbx), %rax
    xorl %ebp, %ebp
    pushq $0
    jmpq *%rax

    .balign 8
    .global __ap_tr_gdt
__ap_tr_gdt:
    .quad 0
    .quad 0x00CF9A000000FFFF            # 0x08: 32bitovy kod
    .quad 0x00CF92000000FFFF            # 0x10: data
    .quad 0x00AF9A000000FFFF            # 0x18: 64bitovy kod
    .global __ap_tr_gdt_ptr
__ap_tr_gdt_ptr:
    .word 31
    .long 0
    .balign 4
    .global __ap_tr_far32
__ap_tr_far32:
    .long 0
    .word 0x08
    .balign 4
    .global __ap_tr_far64
__ap_tr_far64:
    .long 0
    .word 0x18
    .balign 8
    .global __ap_tr_pml4
__ap_tr_pml4:  .quad 0
    .global __ap_tr_efer
__ap_tr_efer:  .quad 0
    .global __ap_tr_cr3
__ap_tr_cr3:   .quad 0
    .global __ap_tr_stack
__ap_tr_stack: .quad 0
    .global __ap_tr_arg
__ap_tr_arg:   .quad 0
    .global __ap_tr_entry
__ap_tr_entry: .quad 0
    .global __ap_tr_end
__ap_tr_end:
    .popsection
    "#,
    options(att_syntax)
);

unsafe extern "C" {
    static __ap_tr_start: u8;
    static __ap_tr_end: u8;
    static __ap_tr_gdt_ptr: u8;
    static __ap_tr_far32: u8;
    static __ap_tr_far64: u8;
    static __ap_tr_pml4: u8;
    static __ap_tr_efer: u8;
    static __ap_tr_cr3: u8;
    static __ap_tr_stack: u8;
    static __ap_tr_arg: u8;
    static __ap_tr_entry: u8;
}

/// Offset symbolu od zacatku trampoliny.
fn offset(sym: *const u8) -> u64 {
    sym as u64 - &raw const __ap_tr_start as u64
}

mod labels {
    unsafe extern "C" {
        pub static __ap_tr_prot32: u8;
        pub static __ap_tr_long64: u8;
        pub static __ap_tr_gdt: u8;
    }
}

/// Najde `N` volnych stranek pod 1 MiB (alokator tuto pamet nespravuje,
/// je vyhrazena pro start CPU). Stranku 0 vynechava.
fn low_pages<const N: usize>(boot_info: &BootInfo) -> Option<[u64; N]> {
    // SAFETY: BootInfo overeny pri startu.
    let regions = unsafe { boot_info.memory_map() };
    let mut pages = regions
        .iter()
        .filter(|r| matches!(r.kind, MemoryKind::Usable | MemoryKind::Reclaimable))
        .flat_map(|r| (r.start.max(0x1000)..r.end().min(0x10_0000)).step_by(4096));
    let mut out = [0; N];
    for slot in &mut out {
        *slot = pages.next()?;
    }
    Some(out)
}

/// Pripravi trampolinu pro start AP (`api::Smp::prepare_cpu_start`).
///
/// # Safety
///
/// Volat jednou, na BSP, po inicializaci pameti.
pub unsafe fn prepare(boot_info: &BootInfo) -> Option<Trampoline> {
    let Some([code, pml4]) = low_pages::<2>(boot_info) else {
        log::warn!("chybi volna pamet pod 1 MiB, bezim jen na jednom CPU");
        return None;
    };
    // SAFETY: obe stranky jsou volna RAM pod 1 MiB, kterou alokator nespravuje.
    Some(unsafe { Trampoline::new(code, pml4) })
}

/// Pripravena trampolina ve dvou strankach pod 1 MiB.
pub struct Trampoline {
    code: u64,
}

impl Trampoline {
    /// Zkopiruje trampolinu na `code` (stranka pod 1 MiB), na `pml4` vytvori
    /// kopii PML4 jadra (musi lezet pod 4 GiB, CR3 se nastavuje v 32bit
    /// rezimu) a docasne identicky namapuje stranku s kodem.
    ///
    /// # Safety
    ///
    /// Obe stranky musi byt volna RAM, kterou do `finish` nikdo nepouziva.
    unsafe fn new(code: u64, pml4: u64) -> Self {
        assert!(
            code < 0x10_0000 && code % 4096 == 0,
            "trampolina musi lezet pod 1 MiB"
        );
        assert!(pml4 < 0x1_0000_0000 && pml4 % 4096 == 0);

        let start = &raw const __ap_tr_start;
        let len = offset(&raw const __ap_tr_end) as usize;
        assert!(len <= 4096);
        let dst: *mut u8 = PhysAddr::new(code).to_virt().as_mut_ptr();
        // SAFETY: zdroj je trampolina v .rodata, cil cela volna stranka.
        unsafe { core::ptr::copy_nonoverlapping(start, dst, len) };

        let put32 = |sym: *const u8, value: u32| {
            // SAFETY: zapis do zkopirovane trampoliny (offset < len).
            unsafe { (dst.add(offset(sym) as usize) as *mut u32).write_unaligned(value) }
        };
        let put64 = |sym: *const u8, value: u64| {
            // SAFETY: viz vyse.
            unsafe { (dst.add(offset(sym) as usize) as *mut u64).write_unaligned(value) }
        };
        // SAFETY: jen adresy symbolu, zadne cteni.
        unsafe {
            put32(
                (&raw const __ap_tr_gdt_ptr).add(2),
                (code + offset(&raw const labels::__ap_tr_gdt)) as u32,
            );
            put32(
                &raw const __ap_tr_far32,
                (code + offset(&raw const labels::__ap_tr_prot32)) as u32,
            );
            put32(
                &raw const __ap_tr_far64,
                (code + offset(&raw const labels::__ap_tr_long64)) as u32,
            );
            let nx = if cpu::features().nx { cpu::EFER_NXE } else { 0 };
            put64(&raw const __ap_tr_efer, nx);
            put64(&raw const __ap_tr_cr3, mm::kernel_root().as_u64());
            put64(&raw const __ap_tr_pml4, pml4);
        }

        // Kod trampoliny musi byt po zapnuti strankovani identicky
        // namapovany; kopie PML4 to mapovani obsahuje taky.
        mm::map_identity_page(PhysAddr::new(code));
        let src: *const u64 = mm::kernel_root().to_virt().as_mut_ptr();
        let dst_pml4: *mut u64 = PhysAddr::new(pml4).to_virt().as_mut_ptr();
        // SAFETY: obe stranky jsou cele PML4 (512 polozek) v prime mape.
        unsafe { core::ptr::copy_nonoverlapping(src, dst_pml4, 512) };

        Self { code }
    }
}

impl CpuStarter for Trampoline {
    /// Nastartuje jedno AP: skoci do `entry(arg)` na zasobniku `stack_top`.
    /// `started` vraci true, jakmile AP ohlasi start. Vraci, zda AP nabehl.
    fn start(
        &self,
        apic_id: u32,
        stack_top: u64,
        entry: extern "C" fn(u64) -> !,
        arg: u64,
        started: impl Fn() -> bool,
    ) -> bool {
        let dst: *mut u8 = PhysAddr::new(self.code).to_virt().as_mut_ptr();
        let put64 = |sym: *const u8, value: u64| {
            // SAFETY: zapis do zkopirovane trampoliny.
            unsafe { (dst.add(offset(sym) as usize) as *mut u64).write_unaligned(value) }
        };
        put64(&raw const __ap_tr_stack, stack_top);
        put64(&raw const __ap_tr_arg, arg);
        put64(&raw const __ap_tr_entry, entry as usize as u64);

        let page = (self.code >> 12) as u8;
        apic::send_init(apic_id);
        time::busy_wait_ns(10_000_000);
        for _ in 0..2 {
            apic::send_startup(apic_id, page);
            for _ in 0..1000 {
                if started() {
                    return true;
                }
                time::busy_wait_ns(200_000);
            }
        }
        started()
    }

    /// Zrusi docasne identicke mapovani.
    fn finish(self) {
        mm::unmap_identity_page(PhysAddr::new(self.code));
    }
}
