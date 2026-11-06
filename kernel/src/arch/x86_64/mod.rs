mod cpu;
mod gdt;
mod idt;
pub mod paging;
mod serial;

pub use serial::SerialPort;

use core::arch::asm;

/// Port prvni seriove linky (COM1). Na UEFI PC je to stale standardni adresa
/// a QEMU ji vystavuje na `-serial`.
const COM1: u16 = 0x3F8;

/// Vrati port pro ladici vystup kernelu.
///
/// Port I/O na COM1 je neskodny i kdyz tam zadna linka neni (zapisy se zahodi),
/// proto je bezpecne ho vytvaret opakovane, napr. v panic handleru.
pub fn debug_serial() -> SerialPort {
    // SAFETY: COM1 v kernelu nepouziva nic jineho nez ladici vystup.
    unsafe { SerialPort::new(COM1) }
}

/// Zakladni inicializace bootovaciho CPU: GDT, TSS, IDT, funkce pro
/// strankovani. Starsi radic preruseni 8259 se uplne zamaskuje, preruseni
/// budou chodit pres APIC (milnik 3).
///
/// # Safety
///
/// Volat jednou, co nejdrive po startu, se zakazanymi prerusenimi.
pub unsafe fn init_boot_cpu() {
    // SAFETY: predano volajicim (jednou, BSP, bez preruseni).
    unsafe {
        gdt::init();
        idt::init();
        paging::init_cpu();
        // 8259 PIC: zamaskovat vsechny linky na master i slave.
        cpu::outb(0x21, 0xFF);
        cpu::outb(0xA1, 0xFF);
    }
}

/// Zakaze preruseni a navzdy zastavi aktualni CPU.
pub fn halt_forever() -> ! {
    loop {
        // SAFETY: cli + hlt jen zastavi toto CPU, nemeni zadny stav pameti.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

/// Vyvola vyjimku breakpoint (test IDT).
pub fn breakpoint() {
    // SAFETY: int3 jen vyvola #BP, ktery obsluha vrati zpet sem.
    unsafe { asm!("int3", options(nomem, nostack)) };
}

/// Prepne na novy zasobnik a zavola `f(arg)`. Puvodni zasobnik se opusti.
///
/// # Safety
///
/// `top` musi byt vrchol namapovaneho, zapisovatelneho zasobniku
/// zarovnany na 16 bajtu, ktery nic jineho nepouziva.
pub unsafe fn switch_stack(top: u64, f: extern "sysv64" fn(u64) -> !, arg: u64) -> ! {
    // SAFETY: predano volajicim. `push 0` = falesna navratova adresa, takze
    // plati zarovnani podle ABI a vypis zasobniku skonci na nule.
    unsafe {
        asm!(
            "mov rsp, {top}",
            "xor ebp, ebp",
            "push 0",
            "jmp {f}",
            top = in(reg) top,
            f = in(reg) f,
            in("rdi") arg,
            options(noreturn)
        )
    }
}

/// Projde retez ramcu (kernel se preklada s frame pointery) od `rbp`
/// a pro kazdou navratovou adresu zavola `f`. Cte jen namapovanou pamet
/// jadra, aby pri poskozenem zasobniku nevznikla dalsi vyjimka.
pub fn stack_trace(mut rbp: u64, mut f: impl FnMut(u64)) {
    const MAX_DEPTH: usize = 32;
    for _ in 0..MAX_DEPTH {
        if rbp == 0
            || rbp % 8 != 0
            || rbp < 0xFFFF_8000_0000_0000
            || !paging::is_mapped(rbp)
            || !paging::is_mapped(rbp + 8)
        {
            return;
        }
        // SAFETY: obe slova ramce jsou zarovnana a namapovana (overeno vyse).
        let (next, ret) = unsafe { (*(rbp as *const u64), *((rbp + 8) as *const u64)) };
        if ret == 0 {
            return;
        }
        f(ret);
        rbp = next;
    }
}

/// Vypis zasobniku od mista volani.
pub fn current_stack_trace(f: impl FnMut(u64)) {
    stack_trace(cpu::read_rbp(), f);
}

/// Ukonci QEMU pres zarizeni `isa-debug-exit` (jen pro automaticke testy).
/// Bez tohoto zarizeni zapis nic neudela a funkce se vrati.
pub fn qemu_exit(success: bool) {
    // Hodnota v se v QEMU prevede na navratovy kod (v << 1) | 1,
    // tj. 0x10 -> 33 (uspech), 0x11 -> 35 (selhani). Musi odpovidat xtask.
    let value = if success { 0x10 } else { 0x11 };
    // SAFETY: port 0xF4 pouzivame vyhradne pro isa-debug-exit.
    unsafe { cpu::outb(0xF4, value) };
}
