mod apic;
pub mod context;
mod cpu;
mod gdt;
mod hpet;
mod idt;
pub mod interrupts;
pub mod paging;
mod serial;
mod smp;
mod time;

pub use serial::SerialPort;
pub use smp::Trampoline;
pub use time::now_ns;

use core::arch::asm;

use crate::acpi::Platform;

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
/// chodi pres APIC.
///
/// # Safety
///
/// Volat jednou, co nejdrive po startu, se zakazanymi prerusenimi.
pub unsafe fn init_boot_cpu() {
    // SAFETY: predano volajicim (jednou, BSP, bez preruseni).
    unsafe {
        gdt::init_bsp();
        // Nacteni selektoru GS v gdt vynuluje jeho bazi: znovu nastavit
        // prazdna per-CPU data, jinak by je cetlo logovani z adresy 0.
        clear_cpu_local();
        idt::init();
        idt::load();
        paging::init_cpu();
        // 8259 PIC: zamaskovat vsechny linky na master i slave.
        cpu::outb(0x21, 0xFF);
        cpu::outb(0xA1, 0xFF);
    }
}

/// Informace o casovacich zjistene pri inicializaci platformy.
pub struct TimerInfo {
    pub tsc_hz: u64,
    pub tsc_invariant: bool,
    pub apic_timer_hz: u64,
    pub x2apic: bool,
}

/// Inicializace platformy na bootovacim CPU: HPET jako reference, kalibrace
/// TSC (hodiny) a APIC timeru, zapnuti lokalniho APIC.
///
/// # Safety
///
/// Volat jednou, po `init_boot_cpu` a inicializaci pameti.
pub unsafe fn init_platform(platform: &Platform) -> TimerInfo {
    let hpet_phys = platform.hpet_phys.expect("platforma nema HPET");
    // SAFETY: adresa HPET z ACPI.
    let hpet = unsafe { hpet::Hpet::new(hpet_phys) };
    const WINDOW_NS: u64 = 20_000_000;
    let tsc_hz = hpet.measure_hz(WINDOW_NS, time::rdtsc);
    time::init(tsc_hz);

    // SAFETY: jednou na BSP, IDT je nahrana, preruseni zakazana.
    unsafe { apic::init_cpu(true, platform.lapic_phys) };
    apic::calibrate_timer(|read| hpet.measure_hz(WINDOW_NS, read));

    TimerInfo {
        tsc_hz,
        tsc_invariant: time::is_invariant(),
        apic_timer_hz: apic::timer_hz(),
        x2apic: apic::is_x2apic(),
    }
}

/// Inicializace aplikacniho procesoru (vola se z jeho vstupniho bodu).
///
/// # Safety
///
/// Volat jednou na kazdem AP, hned po startu, se zakazanymi prerusenimi.
pub unsafe fn init_ap_cpu() {
    // SAFETY: predano volajicim.
    unsafe {
        gdt::init_ap();
        clear_cpu_local();
        idt::load();
        paging::init_cpu();
        apic::init_cpu(false, 0);
    }
}

/// Hodnota citace cyklu (TSC) aktualniho CPU.
pub fn cycle_counter() -> u64 {
    time::rdtsc()
}

/// Posune citac cyklu tohoto CPU; false, pokud to CPU neumi.
pub fn adjust_cycle_counter(delta: i64) -> bool {
    time::adjust(delta)
}

/// Identifikator aktualniho CPU v hardware (ID lokalniho APIC).
pub fn cpu_hw_id() -> u32 {
    apic::id()
}

const MSR_GS_BASE: u32 = 0xC000_0101;

/// Prazdna per-CPU data: gs:[0] = 0, tedy "CPU jeste neni zaregistrovane".
static EMPTY_CPU_LOCAL: u64 = 0;

/// Nastavi GS bazi na prazdna per-CPU data. Volat jako uplne prvni vec pri
/// startu kazdeho CPU, aby cteni per-CPU dat (napr. z logovani) bylo
/// bezpecne jeste pred registraci CPU.
pub fn clear_cpu_local() {
    // SAFETY: staticka promenna zije navzdy a jeji prvni slovo je 0.
    unsafe { cpu::wrmsr(MSR_GS_BASE, &raw const EMPTY_CPU_LOCAL as u64) };
}

/// Nastavi ukazatel na per-CPU data jadra (GS baze).
///
/// # Safety
///
/// `ptr` musi ukazovat na strukturu, jejiz prvni slovo obsahuje `ptr`
/// samotny, a musi zustat platna navzdy.
pub unsafe fn set_cpu_local(ptr: u64) {
    // SAFETY: predano volajicim.
    unsafe { cpu::wrmsr(MSR_GS_BASE, ptr) };
}

/// Ukazatel nastaveny `set_cpu_local` (cte se pres gs:[0], bez MSR).
pub fn cpu_local() -> u64 {
    let v;
    // SAFETY: GS baze ukazuje na per-CPU data, jejichz prvni slovo je
    // ukazatel na ne same (kontrakt `set_cpu_local`).
    unsafe { asm!("mov {}, gs:[0]", out(reg) v, options(readonly, nostack, preserves_flags)) };
    v
}

/// Naplanuje preruseni casovace na tomto CPU za `ns`; `None` ho zastavi.
pub fn set_timer(ns: Option<u64>) {
    apic::set_oneshot(ns);
}

/// Pozada CPU s danym hardwarovym ID o preplanovani.
pub fn send_reschedule(cpu_hw_id: u32) {
    apic::send_ipi(cpu_hw_id, apic::VECTOR_RESCHEDULE);
}

/// Zastavi CPU s danym hardwarovym ID (pri panice).
pub fn send_halt(cpu_hw_id: u32) {
    apic::send_ipi(cpu_hw_id, apic::VECTOR_HALT);
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
