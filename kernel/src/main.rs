//! Kernel.
//!
//! Start na bootovacim CPU: overeni dat od loaderu, konzole, zakladni
//! struktury CPU (GDT, IDT), sprava pameti, ACPI, casovace, prepnuti na
//! zasobnik s ochrannou strankou, planovac, start ostatnich CPU a vlakno
//! `init` se self-testy.

#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod arch;
mod console;
mod error;
mod fatal;
mod framebuffer;
mod log;
mod mm;
mod object;
mod sched;
mod smp;
mod sync;
mod tests;

use boot_info::{BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo};
use core::fmt::Write;

use acpi::Platform;

pub use fatal::fatal_stop;

/// Informace o platforme, zjistene jednou pri startu.
static PLATFORM: spin::Once<Platform> = spin::Once::new();

/// Vstupni bod, na ktery skace loader (viz `ENTRY` v linker.ld).
///
/// Loader predava v `rdi` virtualni adresu `BootInfo` v prime mape fyzicke
/// pameti, preruseni jsou zakazana.
#[unsafe(no_mangle)]
extern "sysv64" fn _start(boot_info: &'static BootInfo) -> ! {
    arch::clear_cpu_local();
    if boot_info.magic != BOOT_INFO_MAGIC || boot_info.version != BOOT_INFO_VERSION {
        let _ = writeln!(arch::debug_serial(), "kernel: neplatny BootInfo od loaderu");
        arch::halt_forever();
    }

    // SAFETY: framebuffer popsal loader a namapoval ho do prime mapy;
    // kernel ho nepouziva nijak jinak nez pres konzoli.
    unsafe { console::init(&boot_info.framebuffer) };
    log::info!("os kernel {} ({})", env!("CARGO_PKG_VERSION"), arch_name());

    // SAFETY: jednou, na bootovacim CPU, preruseni jsou zakazana.
    unsafe { arch::init_boot_cpu() };

    // SAFETY: jednou, po init_boot_cpu; BootInfo je overeny.
    unsafe { mm::init(boot_info) };

    let platform = PLATFORM.call_once(|| match acpi::parse(boot_info.acpi_rsdp) {
        Ok(p) => p,
        Err(e) => panic!("cteni ACPI selhalo: {e}"),
    });
    log::info!(
        "ACPI: {} CPU, {} IOAPIC, HPET {}",
        platform.cpus.len(),
        platform.ioapics.len(),
        if platform.hpet_phys.is_some() {
            "ano"
        } else {
            "ne"
        }
    );
    // SAFETY: jednou, po inicializaci pameti, preruseni zakazana.
    let timers = unsafe { arch::init_platform(platform) };
    log::info!(
        "casovace: TSC {} MHz{}, APIC timer {} kHz, {}",
        timers.tsc_hz / 1_000_000,
        if timers.tsc_invariant {
            " (invariantni)"
        } else {
            " (CPU nehlasi invariantni TSC)"
        },
        timers.apic_timer_hz / 1000,
        if timers.x2apic { "x2APIC" } else { "xAPIC" },
    );

    // ACPI tabulky jsou prectene, GDT a IDT vlastni: pamet firmwaru je volna.
    mm::reclaim_boot_memory(boot_info);
    let (free, total) = mm::phys::stats();
    log::info!(
        "pamet: volno {} / {} MiB",
        free * 4096 / (1024 * 1024),
        total * 4096 / (1024 * 1024)
    );

    let stack = mm::KernelStack::new().expect("nedostatek pameti pro zasobnik");
    let top = stack.top();
    // Zasobnik bude navzdy patrit idle vlaknu bootovaciho CPU.
    core::mem::forget(stack);
    // SAFETY: zasobnik je prave alokovany, namapovany a nepouzivany.
    unsafe { arch::switch_stack(top, kernel_main, boot_info as *const _ as u64) }
}

/// Pokracovani startu na zasobniku jadra s ochrannou strankou.
extern "sysv64" fn kernel_main(boot_info: u64) -> ! {
    // SAFETY: ukazatel predany z `_start`, BootInfo zije navzdy.
    let boot_info = unsafe { &*(boot_info as *const BootInfo) };
    let platform = PLATFORM.get().unwrap();

    sched::init_cpu(0, arch::cpu_hw_id());
    let cpus = smp::start_secondary_cpus(platform, boot_info);
    log::info!("bezi {cpus} z {} CPU", platform.cpus.len());

    sched::spawn("init", sched::Qos::Normal, init_main);
    sched::idle_loop()
}

/// Prvni vlakno jadra. Pozdeji spusti prvni proces v userspace.
fn init_main() {
    let ok = tests::run_all();
    if ok {
        log::info!("self-testy prosly, jadro bezi");
    } else {
        log::error!("self-testy selhaly");
    }
    if cfg!(feature = "qemu-test") {
        arch::qemu_exit(ok);
    }
}

const fn arch_name() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "neznama architektura"
    }
}
