//! Start ostatnich CPU.
//!
//! Mechanismus startu (x86: INIT-SIPI s trampolinou, aarch64: PSCI) je
//! v `arch` (`api::Smp`); tady je poradi, zasobniky, cekani na ohlaseni
//! a srovnani citace cyklu.

mod clock_sync;

use core::sync::atomic::{AtomicBool, Ordering};

use boot_info::BootInfo;

use crate::acpi::Platform;
use crate::arch::{self, CpuStartResult, CpuStarterOps};
use crate::mm::KernelStack;
use crate::{log, sched};

static AP_STARTED: AtomicBool = AtomicBool::new(false);

/// Nastartuje vsechna CPU z ACPI krome aktualniho. Vraci pocet bezicich CPU.
pub fn start_secondary_cpus(platform: &Platform, boot_info: &BootInfo) -> usize {
    let bsp = arch::cpu_hw_id();
    // SAFETY: jednou, na bootovacim CPU, pamet je inicializovana.
    let Some(starter) = (unsafe { arch::prepare_cpu_start(boot_info) }) else {
        return 1;
    };

    let mut index = 1;
    for cpu in platform.cpus.iter().filter(|c| c.hw_id != bsp) {
        let Some(stack) = KernelStack::new() else {
            log::error!("nedostatek pameti pro zasobnik CPU {}", cpu.hw_id);
            break;
        };
        AP_STARTED.store(false, Ordering::Release);
        clock_sync::reset();
        let result = starter.start(cpu.hw_id, stack.top(), ap_entry, index as u64, || {
            AP_STARTED.load(Ordering::Acquire)
        });
        if result == CpuStartResult::Failed {
            // CPU zasobnik ani data startu nepouzije (duvod zalogovala
            // architektura): uvolnit a zkusit dalsi CPU.
            drop(stack);
            continue;
        }
        // Zasobnik patri novemu CPU navzdy - i kdyz se neohlasilo vcas, muze
        // jeste nabehnout a zasobnik pouzit, takze ho nesmime uvolnit.
        core::mem::forget(stack);
        if result == CpuStartResult::TimedOut {
            // CPU muze nabehnout pozdeji: nechame mu jeho index i pomocna
            // data startu (x86: trampolinu vcetne identickeho mapovani)
            // a dalsi CPU uz nestartujeme, aby se neprepsala pod rukama.
            log::error!(
                "CPU s {} {} se neohlasilo, dalsi CPU nestartuji",
                arch::HW_ID_NAME,
                cpu.hw_id
            );
            return index;
        }
        if !arch::CYCLE_COUNTER_SYNCED {
            clock_sync::measure_and_fix(cpu.hw_id);
        }
        index += 1;
    }
    starter.finish();
    index
}

extern "C" fn ap_entry(index: u64) -> ! {
    arch::clear_cpu_local();
    // SAFETY: prvni kod na novem CPU, preruseni jsou zakazana.
    unsafe { arch::init_ap_cpu() };
    sched::init_cpu(index as usize, arch::cpu_hw_id());
    AP_STARTED.store(true, Ordering::Release);
    if !arch::CYCLE_COUNTER_SYNCED {
        clock_sync::answer();
    }
    log::debug!(
        "CPU {index} ({} {}) bezi",
        arch::HW_ID_NAME,
        arch::cpu_hw_id()
    );
    sched::idle_loop()
}
