//! Neopravitelne chyby: panika a fatalni vyjimky CPU.

use core::fmt;
use core::panic::PanicInfo;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{arch, log, sched};

static PANICKING: AtomicBool = AtomicBool::new(false);
static STOPPING: AtomicBool = AtomicBool::new(false);

/// Vypis pri fatalni chybe (bez cekani na zamky, zapise se i do bufferu logu).
pub fn report(args: fmt::Arguments) {
    log::emergency(args);
}

/// Vypis zasobniku od ukazatele ramce `fp` (`None` = od mista volani).
pub fn report_stack_trace(fp: Option<u64>) {
    let mut print = |addr: u64| report(format_args!("    {addr:#018x}"));
    match fp {
        Some(fp) => arch::stack_trace(fp, &mut print),
        None => arch::current_stack_trace(&mut print),
    }
}

/// Konec pri neopravitelne chybe: zastavi ostatni CPU, v testech ukonci
/// QEMU s neuspechem, jinak zastavi i aktualni CPU.
///
/// Pokud behem zastavovani vznikne dalsi chyba (napr. vyjimka v obsluze
/// jine vyjimky), uz se nic nezkousi a CPU se rovnou zastavi - jinak by se
/// chyby zacyklily a puvodni pricina by se ztratila.
pub fn fatal_stop() -> ! {
    arch::interrupts::disable();
    if STOPPING.swap(true, Ordering::SeqCst) {
        arch::halt_forever();
    }
    sched::halt_other_cpus();
    // Zaznam pady do RAM, ktera prezije teply restart, a obrazovka pady.
    let record = crate::pstore::on_fatal();
    if crate::pstore::is_test_panic() {
        // Test zaznamu pady: hned restart (QEMU bez -no-reboot nabehne
        // znovu se stejnou RAM).
        let _ = arch::system_power(false);
        arch::halt_forever();
    }
    if cfg!(feature = "qemu-test") {
        arch::qemu_exit(false);
    }
    crate::crashscreen::show(record);
    // Odpocet a restart: zaznam ceka v RAM na dalsi start.
    let until = arch::now_ns() + crate::crashscreen::RESTART_AFTER_NS;
    while arch::now_ns() < until {
        core::hint::spin_loop();
    }
    let _ = arch::system_power(false);
    arch::halt_forever()
}

/// Kod duvodu pady pro zaznam a obrazovku (napr. `PAGE_FAULT`).
pub fn set_reason(reason: &'static str) {
    crate::pstore::set_reason(reason);
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    arch::interrupts::disable();
    if PANICKING.swap(true, Ordering::SeqCst) {
        // Panika behem paniky (nebo na jinem CPU soucasne): jen zastavit.
        arch::halt_forever();
    }
    set_reason("KERNEL_PANIC");
    report(format_args!(
        "KERNEL PANIC na cpu{}: {info}",
        sched::try_current_cpu_index().unwrap_or(0)
    ));
    report_stack_trace(None);
    fatal_stop()
}
