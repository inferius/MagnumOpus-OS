pub mod paging;

use core::arch::asm;

/// Zapne bit NXE v EFER, pokud ho CPU podporuje. Vraci, zda je NX k dispozici.
pub fn enable_nx() -> bool {
    // CPUID 0x8000_0001, EDX bit 20 = NX.
    let ext = core::arch::x86_64::__cpuid(0x8000_0001);
    if ext.edx & (1 << 20) == 0 {
        return false;
    }
    const IA32_EFER: u32 = 0xC000_0080;
    // SAFETY: EFER existuje na kazdem x86_64 CPU a NX je podle CPUID
    // podporovany. Nastaveni NXE jen povoli interpretaci bitu 63 v polozkach
    // tabulek stranek; aktualni tabulky firmwaru jsou platne v obou rezimech
    // (pri vypnutem NXE musi mit bit 63 nulovy).
    unsafe {
        let (lo, hi): (u32, u32);
        asm!("rdmsr", in("ecx") IA32_EFER, out("eax") lo, out("edx") hi, options(nomem, nostack));
        asm!("wrmsr", in("ecx") IA32_EFER, in("eax") lo | (1 << 11), in("edx") hi, options(nomem, nostack));
    }
    true
}

/// Prepne na nove tabulky stranek, nastavi zasobnik a skoci do kernelu.
///
/// # Safety
///
/// - `pml4` musi byt fyzicka adresa platne PML4, ktera identicky mapuje
///   kod loaderu (tato funkce bezi i po prepnuti CR3), mapuje `stack_top`
///   a vstupni bod kernelu.
/// - `stack_top` musi byt zarovnany na 16 bajtu.
/// - Boot services uz musi byt ukoncene.
pub unsafe fn jump_to_kernel(pml4: u64, stack_top: u64, entry: u64, boot_info: u64) -> ! {
    // SAFETY: podminky viz dokumentace funkce. `push 0` vytvori falesnou
    // navratovou adresu, takze pri vstupu do kernelu plati System V ABI
    // (rsp + 8 zarovnane na 16) a backtrace skonci na nule.
    unsafe {
        asm!(
            "cli",
            "mov cr3, {pml4}",
            "mov rsp, {stack}",
            "xor ebp, ebp",
            "push 0",
            "jmp {entry}",
            pml4 = in(reg) pml4,
            stack = in(reg) stack_top,
            entry = in(reg) entry,
            in("rdi") boot_info,
            options(noreturn)
        )
    }
}
