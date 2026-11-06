//! Kernel.
//!
//! Zatim (milnik 1) jen overi predana data od loaderu, vypise uvitani na
//! seriovou linku a do framebufferu a zastavi CPU.

#![no_std]
#![no_main]

mod arch;
mod console;
mod framebuffer;

use boot_info::{BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo, MemoryKind};
use core::fmt::Write;
use core::panic::PanicInfo;

/// Vstupni bod, na ktery skace loader (viz `ENTRY` v linker.ld).
///
/// Loader predava v `rdi` virtualni adresu `BootInfo` v prime mape fyzicke
/// pameti, preruseni jsou zakazana a zasobnik ma `KERNEL_STACK_SIZE` bajtu.
#[unsafe(no_mangle)]
extern "sysv64" fn _start(boot_info: &'static BootInfo) -> ! {
    if boot_info.magic != BOOT_INFO_MAGIC || boot_info.version != BOOT_INFO_VERSION {
        let _ = writeln!(arch::debug_serial(), "kernel: neplatny BootInfo od loaderu");
        arch::halt_forever();
    }

    // SAFETY: framebuffer popsal loader a namapoval ho do prime mapy;
    // kernel ho zatim nepouziva nijak jinak nez pres konzoli.
    unsafe { console::init(&boot_info.framebuffer) };

    kprintln!("os kernel {} ({})", env!("CARGO_PKG_VERSION"), arch_name());
    kprintln!("Ahoj, svete! Kernel bezi.");
    kprintln!("UTF-8 test: Příliš žluťoučký kůň úpěl ďábelské ódy.");
    kprintln!();

    let fb = &boot_info.framebuffer;
    if fb.is_present() {
        kprintln!(
            "framebuffer: {}x{}, stride {}, {:?}, fyz. {:#x}",
            fb.width,
            fb.height,
            fb.stride,
            fb.format,
            fb.phys_addr
        );
    } else {
        kprintln!("framebuffer: neni");
    }

    // SAFETY: BootInfo je overeny (magic, verze) a prima mapa je aktivni.
    let regions = unsafe { boot_info.memory_map() };
    let total = |kind: MemoryKind| -> u64 {
        regions
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.len)
            .sum()
    };
    const MIB: u64 = 1024 * 1024;
    kprintln!(
        "pamet: {} oblasti, volna {} MiB, k uvolneni {} MiB, kernel {} KiB",
        regions.len(),
        total(MemoryKind::Usable) / MIB,
        total(MemoryKind::Reclaimable) / MIB,
        total(MemoryKind::Kernel) / 1024
    );
    kprintln!("ACPI RSDP: {:#x}", boot_info.acpi_rsdp);
    kprintln!();
    kprintln!("Milnik 1 hotov, zastavuji CPU.");

    arch::halt_forever()
}

const fn arch_name() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "neznama architektura"
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    console::panic_print(format_args!("\nKERNEL PANIC: {info}\n"));
    arch::halt_forever()
}
