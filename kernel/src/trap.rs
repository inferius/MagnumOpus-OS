//! Zpetny smer smlouvy architektury: co kod architektury vola v jadru.
//!
//! Obsluha preruseni, vyjimek a syscallu je v `arch/<cil>/` (tabulka
//! vektoru, ulozeni registru, potvrzeni preruseni v radici). Jakmile vi,
//! co se stalo, preda rizeni sem - a nikam jinam do jadra. Diky tomu ma
//! nova architektura presny seznam toho, co musi zavolat (a kdy), a zbytek
//! jadra se pri portu nemeni.
//!
//! Pravidla pro volajici (kod architektury):
//! - vsechny funkce se volaji se zakazanymi prerusenimi, krome `syscall`
//!   (ten si je povoli sam az po prepnuti na zasobnik jadra - viz popis),
//! - preruseni je pred volanim uz potvrzene v radici (EOI), aby dalsi
//!   preruseni mohlo prijit hned po navratu nebo po prepnuti vlakna,
//! - `after_interrupt` se vola po kazdem preruseni (ne vyjimce),
//!   `return_to_user` pred kazdym navratem do userspace.

use crate::{mm, object, random, sched, syscall};

/// Preruseni casovace naplanovaneho pres `arch::set_timer`.
#[inline]
pub fn timer_interrupt() {
    sched::on_timer_interrupt();
}

/// IPI "preplanuj" (`arch::send_reschedule`).
#[inline]
pub fn reschedule_ipi() {
    sched::on_reschedule_ipi();
}

/// IPI "vyprazdni TLB" (`arch::send_tlb_flush`).
#[inline]
pub fn tlb_flush_ipi() {
    mm::tlb::on_flush_ipi();
}

/// Preruseni zarizeni s cislem z rozsahu `DEVICE_IRQ_FIRST..=DEVICE_IRQ_LAST`.
#[inline]
pub fn device_irq(irq: u32) {
    // Casovani preruseni plni zasobnik entropie (bez zamku, levne).
    random::on_interrupt(irq);
    object::interrupt::dispatch(irq);
}

/// Konec obsluhy preruseni: preplanovani, pokud je potreba.
#[inline]
pub fn after_interrupt() {
    sched::preempt_if_needed();
}

/// Tesne pred navratem do userspace (konec syscallu nebo preruseni
/// z userspace): ukoncovane vlakno tady skonci.
#[inline]
pub fn return_to_user() {
    sched::on_return_to_user();
}

/// Syscall z userspace: cislo a 6 argumentu, vraci hodnotu pro program.
/// Volat uz na zasobniku jadra s povolenymi prerusenimi; po navratu
/// zakazat preruseni a zavolat `return_to_user`.
#[inline]
pub fn syscall(number: u64, args: [u64; 6]) -> u64 {
    syscall::dispatch(number, args)
}

/// Vyjimka CPU v userspace: ukonci proces. `code` je cislo chyby podle ABI
/// (`abi::process::EXIT_FAULT_BASE - code`); cisla odpovidaji vektorum
/// x86 (6 = neplatna instrukce, 13 = ochrana, 14 = vypadek stranky, ...)
/// a ostatni architektury na ne sve vyjimky mapuji. `pc` = adresa
/// instrukce, `addr` = adresa pristupu (u vypadku stranky, jinak 0).
#[inline]
pub fn user_fault(code: u8, error_code: u64, pc: u64, addr: u64) -> ! {
    sched::user_fault(code, error_code, pc, addr)
}

/// Index aktualniho CPU pro vypis fatalni chyby (0, pokud jeste neni
/// zaregistrovane).
#[inline]
pub fn current_cpu_index() -> usize {
    sched::try_current_cpu_index().unwrap_or(0)
}

/// Hardwarove ID CPU, na ktere se smeruji preruseni zarizeni (zatim vzdy
/// bootovaci CPU).
#[inline]
#[cfg_attr(
    not(target_arch = "x86_64"),
    allow(dead_code, reason = "aarch64 smeruje MSI pres kolekci ITS")
)]
pub fn irq_target_hw_id() -> u32 {
    sched::all_cpus().next().map_or(0, |c| c.hw_id)
}
