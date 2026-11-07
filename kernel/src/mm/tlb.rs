//! Vyprazdneni TLB na vsech CPU po zmene mapovani userspace (shootdown).
//!
//! Kazde CPU ma dve pocitadla: `tlb_requested` (kolik pozadavku dostalo)
//! a `tlb_done` (do ktereho pozadavku vcetne uz TLB vyprazdnilo). Zadatel si
//! vezme cislo pozadavku, posle IPI a ceka, az `tlb_done` cislo dozene.
//! Pocitadla misto priznaku: pozadavek, ktery prijde behem vyrizovani
//! jineho, se tak neztrati.
//!
//! Architektura, ktera umi zneplatnit TLB vsech CPU jedinou instrukci
//! (`arch::flush_tlb_broadcast`, napr. aarch64 `TLBI ... IS`), IPI vubec
//! nepotrebuje.
//!
//! Zadatel behem cekani vyrizuje i pozadavky na sebe, takze dve CPU, ktera
//! si posilaji shootdown soucasne, se nezablokuji. Volat bez drzeni zamku,
//! na ktere muze cekat jine CPU se zakazanymi prerusenimi.

use core::sync::atomic::Ordering;

use crate::arch;
use crate::sched::{self, all_cpus};

/// Vyprazdni TLB userspace na vsech CPU a pocka na dokonceni.
pub fn shootdown() {
    if arch::flush_tlb_broadcast() {
        return;
    }
    arch::flush_tlb_local();
    let Some(me) = sched::try_current_cpu_index() else {
        return; // pred startem planovace bezi jen jedno CPU
    };
    let mut tickets = alloc::vec::Vec::new();
    for cpu in all_cpus().filter(|c| c.index != me) {
        let ticket = cpu.tlb_requested.fetch_add(1, Ordering::AcqRel) + 1;
        arch::send_tlb_flush(cpu.hw_id);
        tickets.push((cpu, ticket));
    }
    for (cpu, ticket) in tickets {
        while cpu.tlb_done.load(Ordering::Acquire) < ticket {
            service_own();
            core::hint::spin_loop();
        }
    }
}

/// Obsluha IPI "vyprazdni TLB".
pub fn on_flush_ipi() {
    service_own();
}

fn service_own() {
    let Some(cpu) = sched::try_this_cpu() else {
        return;
    };
    let requested = cpu.tlb_requested.load(Ordering::Acquire);
    if cpu.tlb_done.load(Ordering::Acquire) < requested {
        arch::flush_tlb_local();
        cpu.tlb_done.fetch_max(requested, Ordering::AcqRel);
    }
}
