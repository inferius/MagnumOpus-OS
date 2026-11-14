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
mod crashscreen;
mod error;
mod fatal;
mod framebuffer;
mod log;
mod mm;
mod object;
mod pstore;
mod random;
mod sched;
mod sysinfo;
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

/// Programy jsou prelozene pro zakladni uroven CPU (x86-64-v2, aarch64 FP
/// a NEON). Starsi CPU se zastavi hned se srozumitelnou hlaskou na obrazovce
/// i v logu - jinak by nektery program spadl na neplatne instrukci
/// nahodne za behu. Bez restartu (stejny stroj by se jen tocil dokola).
fn check_cpu_baseline() {
    let mut missing = [""; 8];
    let mut n = 0;
    let baseline = arch::missing_baseline(&mut |name| {
        if n < missing.len() {
            missing[n] = name;
            n += 1;
        }
    });
    if n == 0 {
        return;
    }
    // Halda jeste neni: seznam se sklada bez alokace.
    struct List<'a>(&'a [&'a str]);
    impl core::fmt::Display for List<'_> {
        fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
            for (i, s) in self.0.iter().enumerate() {
                write!(f, "{}{s}", if i > 0 { ", " } else { "" })?;
            }
            Ok(())
        }
    }
    log::error!("procesor nesplnuje pozadavky systemu ({baseline}): chybi {}", List(&missing[..n]));
    log::error!("system potrebuje procesor s {baseline} (x86_64 zhruba od roku 2009); viz docs/cs/pozadavky-hw.md");
    if cfg!(feature = "qemu-test") {
        arch::qemu_exit(false);
    }
    arch::halt_forever();
}

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
    unsafe {
        console::init(
            &boot_info.framebuffer,
            boot_info.boot_flags & boot_info::boot_flags::SPLASH != 0,
        )
    };
    log::set_defaults();
    log::info!(
        "os kernel {} ({}), sestaveni {}",
        env!("CARGO_PKG_VERSION"),
        arch::NAME,
        // Commit a datum doda xtask (`MO_BUILD`); `*` = neulozene zmeny.
        option_env!("MO_BUILD").unwrap_or("bez xtask")
    );

    check_cpu_baseline();

    // SAFETY: jednou, na bootovacim CPU, preruseni jsou zakazana.
    unsafe { arch::init_boot_cpu() };

    // SAFETY: jednou, po init_boot_cpu; BootInfo je overeny.
    unsafe { mm::init(boot_info) };
    pstore::init(boot_info);
    if boot_info.boot_flags & boot_info::boot_flags::RESCUE_LOGS != 0 {
        log::info!("rezim: vytahnout logy nainstalovaneho systemu (volba zavadece)");
        userboot::RESCUE_LOGS.store(true, core::sync::atomic::Ordering::Relaxed);
    }
    sysinfo::log_machine(boot_info);

    let platform = PLATFORM.call_once(|| match acpi::parse(boot_info.acpi_rsdp) {
        Ok(p) => p,
        Err(e) => panic!("cteni ACPI selhalo: {e}"),
    });
    log::info!("ACPI: {} CPU, {}", platform.cpus.len(), platform.arch);
    // SAFETY: jednou, po inicializaci pameti, preruseni zakazana.
    let timers = unsafe { arch::init_platform(platform) };
    log::info!("casovace: {timers}");

    clock::init(boot_info, platform);
    random::init(boot_info);

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
    sched::topo::set_platform(platform);
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
    // Startovni obraz nesmi zustat viset: dal uz ho nikdo neprevezme,
    // na obrazovku jde log s duvodem konce.
    if init_code != Some(0) || !cfg!(feature = "qemu-test") {
        console::leave_splash();
    }
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
