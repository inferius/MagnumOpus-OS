//! Kernel.
//!
//! Start na bootovacim CPU: overeni dat od loaderu, konzole, zakladni
//! struktury CPU (`arch::init_boot_cpu`), sprava pameti, ACPI, casovace, prepnuti na
//! zasobnik s ochrannou strankou, planovac, start ostatnich CPU a vlakno
//! `init` se self-testy.

#![no_std]
#![no_main]

extern crate alloc;

mod acpi;
mod arch;
mod clock;
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
mod syscall;
mod tests;
mod trap;
mod userboot;

use boot_info::{BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo};
use core::fmt::Write;

use acpi::Platform;

pub use fatal::fatal_stop;

/// Informace o platforme, zjistene jednou pri startu.
static PLATFORM: spin::Once<Platform> = spin::Once::new();

/// Vstupni bod, na ktery skace loader (viz `ENTRY` v linker.ld).
///
/// Loader predava virtualni adresu `BootInfo` v prime mape fyzicke pameti
/// jako prvni argument podle C ABI cile (x86_64: `rdi`, aarch64: `x0`),
/// preruseni jsou zakazana.
#[unsafe(no_mangle)]
extern "C" fn _start(boot_info: &'static BootInfo) -> ! {
    arch::clear_cpu_local();
    arch::init_early(boot_info);
    if boot_info.magic != BOOT_INFO_MAGIC || boot_info.version != BOOT_INFO_VERSION {
        let _ = writeln!(arch::debug_serial(), "kernel: neplatny BootInfo od loaderu");
        arch::halt_forever();
    }

    // SAFETY: framebuffer popsal loader a namapoval ho do prime mapy;
    // kernel ho nepouziva nijak jinak nez pres konzoli.
    unsafe { console::init(&boot_info.framebuffer) };
    log::info!("os kernel {} ({})", env!("CARGO_PKG_VERSION"), arch::NAME);

    // SAFETY: jednou, na bootovacim CPU, preruseni jsou zakazana.
    unsafe { arch::init_boot_cpu() };

    // SAFETY: jednou, po init_boot_cpu; BootInfo je overeny.
    unsafe { mm::init(boot_info) };

    let platform = PLATFORM.call_once(|| match acpi::parse(boot_info.acpi_rsdp) {
        Ok(p) => p,
        Err(e) => panic!("cteni ACPI selhalo: {e}"),
    });
    log::info!("ACPI: {} CPU, {}", platform.cpus.len(), platform.arch);
    // SAFETY: jednou, po inicializaci pameti, preruseni zakazana.
    let timers = unsafe { arch::init_platform(platform) };
    log::info!("casovace: {timers}");

    clock::init(boot_info, platform);

    // ACPI tabulky jsou prectene, tabulky CPU vlastni: pamet firmwaru je volna.
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
extern "C" fn kernel_main(boot_info: u64) -> ! {
    // SAFETY: ukazatel predany z `_start`, BootInfo zije navzdy.
    let boot_info = unsafe { &*(boot_info as *const BootInfo) };
    let platform = PLATFORM.get().unwrap();

    sched::init_cpu(0, arch::cpu_hw_id());
    let cpus = smp::start_secondary_cpus(platform, boot_info);
    log::info!("bezi {cpus} z {} CPU", platform.cpus.len());

    sched::spawn("boot", sched::Qos::Normal, move || boot_main(boot_info));
    sched::idle_loop()
}

/// Prvni vlakno jadra: self-testy jadra a spusteni procesu `init`.
fn boot_main(boot_info: &'static BootInfo) {
    // IOMMU pred spustenim ovladacu (od ted je DMA zarizeni bez domeny
    // zakazane). Bezi uz vsechna CPU, takze jde nasmerovat preruseni chyb.
    {
        let p = PLATFORM.get().unwrap();
        // SAFETY: jednou, pred spustenim procesu.
        unsafe { arch::iommu::init(p) };
    }
    let kernel_ok = tests::run_all();
    if kernel_ok {
        log::info!("self-testy jadra prosly");
    } else {
        log::error!("self-testy jadra selhaly");
    }

    let init_code = match userboot::start_init(boot_info, PLATFORM.get().unwrap()) {
        Ok(Some(init)) => {
            // Init bezi dal sam; tady jen cekame na jeho konec (v testech je
            // jeho navratovy kod vysledkem userspace testu).
            let _ = object::wait::wait_one(&*init, object::Signals::TERMINATED, None);
            init.exit_code()
        }
        Ok(None) => None,
        Err(e) => {
            log::error!("init se nepodarilo spustit: {e}");
            None
        }
    };
    match init_code {
        Some(0) => log::info!("init skoncil uspesne"),
        Some(code) => log::error!(
            "init skoncil s kodem {code} ({})",
            abi::process::describe_exit(code)
        ),
        None => log::error!("init nebezi"),
    }
    if cfg!(feature = "qemu-test") {
        arch::qemu_exit(kernel_ok && init_code == Some(0));
    }
}
