//! Kernel.
//!
//! Start: overeni dat od loaderu, konzole, zakladni struktury CPU (GDT,
//! IDT), sprava pameti (fyzicky alokator, vlastni tabulky stranek, heap),
//! prepnuti na zasobnik s ochrannou strankou, self-testy.

#![no_std]
#![no_main]

extern crate alloc;

mod arch;
mod console;
mod framebuffer;
mod mm;
mod selftest;

use boot_info::{BOOT_INFO_MAGIC, BOOT_INFO_VERSION, BootInfo};
use core::fmt::Write;
use core::panic::PanicInfo;

/// Vstupni bod, na ktery skace loader (viz `ENTRY` v linker.ld).
///
/// Loader predava v `rdi` virtualni adresu `BootInfo` v prime mape fyzicke
/// pameti, preruseni jsou zakazana.
#[unsafe(no_mangle)]
extern "sysv64" fn _start(boot_info: &'static BootInfo) -> ! {
    if boot_info.magic != BOOT_INFO_MAGIC || boot_info.version != BOOT_INFO_VERSION {
        let _ = writeln!(arch::debug_serial(), "kernel: neplatny BootInfo od loaderu");
        arch::halt_forever();
    }

    // SAFETY: framebuffer popsal loader a namapoval ho do prime mapy;
    // kernel ho nepouziva nijak jinak nez pres konzoli.
    unsafe { console::init(&boot_info.framebuffer) };
    kprintln!("os kernel {} ({})", env!("CARGO_PKG_VERSION"), arch_name());

    // SAFETY: jednou, na bootovacim CPU, preruseni jsou zakazana.
    unsafe { arch::init_boot_cpu() };
    kprintln!("cpu: GDT, TSS a IDT nahrane");

    // SAFETY: jednou, po init_boot_cpu; BootInfo je overeny.
    let stack_top = unsafe { mm::init(boot_info) };
    let (free, total) = mm::phys::stats();
    kprintln!(
        "pamet: vlastni tabulky stranek aktivni, volno {} / {} MiB",
        free * 4096 / (1024 * 1024),
        total * 4096 / (1024 * 1024)
    );

    // SAFETY: zasobnik prave alokovany v mm::init, namapovany, nepouzivany.
    unsafe { arch::switch_stack(stack_top, kernel_main, boot_info as *const _ as u64) }
}

/// Pokracovani startu na zasobniku jadra s ochrannou strankou.
extern "sysv64" fn kernel_main(_boot_info: u64) -> ! {
    kprintln!("self-testy:");
    selftest::run();
    kprintln!("Milnik 2 hotov, zastavuji CPU.");

    if cfg!(feature = "qemu-test") {
        arch::qemu_exit(true);
    }
    arch::halt_forever()
}

const fn arch_name() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else {
        "neznama architektura"
    }
}

/// Konec pri neopravitelne chybe: v testech ukonci QEMU s neuspechem,
/// jinak zastavi CPU.
pub fn fatal_stop() -> ! {
    if cfg!(feature = "qemu-test") {
        arch::qemu_exit(false);
    }
    arch::halt_forever()
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    console::panic_print(format_args!("\nKERNEL PANIC: {info}\n"));
    arch::current_stack_trace(|addr| console::panic_print(format_args!("    {addr:#018x}\n")));
    fatal_stop()
}
