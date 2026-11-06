//! Start ostatnich CPU.

mod clock_sync;

use core::sync::atomic::{AtomicBool, Ordering};

use boot_info::{BootInfo, MemoryKind};

use crate::acpi::Platform;
use crate::arch::{self, Trampoline};
use crate::mm::KernelStack;
use crate::{log, sched};

static AP_STARTED: AtomicBool = AtomicBool::new(false);

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

/// Nastartuje vsechna CPU z ACPI krome aktualniho. Vraci pocet bezicich CPU.
pub fn start_secondary_cpus(platform: &Platform, boot_info: &BootInfo) -> usize {
    let bsp = arch::cpu_hw_id();
    let Some([code, pml4]) = low_pages::<2>(boot_info) else {
        log::warn!("chybi volna pamet pod 1 MiB, bezim jen na jednom CPU");
        return 1;
    };
    // SAFETY: obe stranky jsou volna RAM pod 1 MiB, kterou alokator nespravuje.
    let trampoline = unsafe { Trampoline::new(code, pml4) };

    let mut index = 1;
    for cpu in platform.cpus.iter().filter(|c| c.apic_id != bsp) {
        let Some(stack) = KernelStack::new() else {
            log::error!("nedostatek pameti pro zasobnik CPU {}", cpu.apic_id);
            break;
        };
        AP_STARTED.store(false, Ordering::Release);
        clock_sync::reset();
        let ok = trampoline.start(cpu.apic_id, stack.top(), ap_entry, index as u64, || {
            AP_STARTED.load(Ordering::Acquire)
        });
        // Zasobnik patri novemu CPU navzdy - i kdyz se neohlasilo vcas, muze
        // jeste nabehnout a zasobnik pouzit, takze ho nesmime uvolnit.
        core::mem::forget(stack);
        if !ok {
            // CPU muze nabehnout pozdeji: nechame mu jeho index i trampolinu
            // (vcetne identickeho mapovani) a dalsi CPU uz nestartujeme, aby
            // se data trampoliny neprepsala pod rukama.
            log::error!(
                "CPU s APIC ID {} se neohlasilo, dalsi CPU nestartuji",
                cpu.apic_id
            );
            return index;
        }
        clock_sync::measure_and_fix(cpu.apic_id);
        index += 1;
    }
    trampoline.finish();
    index
}

extern "sysv64" fn ap_entry(index: u64) -> ! {
    arch::clear_cpu_local();
    // SAFETY: prvni kod na novem CPU, preruseni jsou zakazana.
    unsafe { arch::init_ap_cpu() };
    sched::init_cpu(index as usize, arch::cpu_hw_id());
    AP_STARTED.store(true, Ordering::Release);
    clock_sync::answer();
    log::debug!("CPU {index} (APIC ID {}) bezi", arch::cpu_hw_id());
    sched::idle_loop()
}
