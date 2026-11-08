//! Udalosti tlacitek: pevna udalost (x86: bit PWRBTN_STS v PM1), zarizeni
//! `PNP0C0C`/`PNP0C0E` s oznamenim `Notify(..., 0x80)` a Generic Event
//! Device (`ACPI0013`, HW-reduced: ARM). GED ma v `_CRS` sva preruseni; pri
//! preruseni se vola jeho metoda `_EVT(cislo)`, ktera v AML provede
//! `Notify` na tlacitko.
//!
//! Obsluhy bezi ve vlaknech ACPICA (SCI, fronta oznameni, vlakno GED);
//! hlavnimu vlaknu predaji udalost pres pocitadla a event [`wake_event`].

use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use acpica::ffi::{self, Handle};
use acpica::ns;
use libos::Event;
use powerproto::Event as PowerEvent;

static WAKE: spin::Once<Event> = spin::Once::new();
static POWER: AtomicU32 = AtomicU32::new(0);
static SLEEP: AtomicU32 = AtomicU32::new(0);
/// Zarizeni tlacitek (handle jako cislo; `Handle` neni `Send`).
static BUTTONS: spin::Mutex<Vec<(usize, PowerEvent)>> = spin::Mutex::new(Vec::new());
/// Platforma hlasi tlacitko napajeni (pro `Status`).
pub static HAS_POWER_BUTTON: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

pub fn wake_event() -> Option<&'static Event> {
    WAKE.get()
}

fn raise(e: PowerEvent) {
    match e {
        PowerEvent::PowerButton => POWER.fetch_add(1, Ordering::AcqRel),
        PowerEvent::SleepButton => SLEEP.fetch_add(1, Ordering::AcqRel),
    };
    if let Some(w) = WAKE.get() {
        let _ = w.signal();
    }
}

/// Vyzvedne udalosti od posledniho volani (hlavni vlakno).
pub fn take() -> Vec<PowerEvent> {
    if let Some(w) = WAKE.get() {
        let _ = w.reset();
    }
    let mut v = Vec::new();
    for _ in 0..POWER.swap(0, Ordering::AcqRel).min(4) {
        v.push(PowerEvent::PowerButton);
    }
    for _ in 0..SLEEP.swap(0, Ordering::AcqRel).min(4) {
        v.push(PowerEvent::SleepButton);
    }
    v
}

unsafe extern "C" fn fixed_power(_ctx: *mut c_void) -> u32 {
    raise(PowerEvent::PowerButton);
    ffi::ACPI_INTERRUPT_HANDLED
}

unsafe extern "C" fn fixed_sleep(_ctx: *mut c_void) -> u32 {
    raise(PowerEvent::SleepButton);
    ffi::ACPI_INTERRUPT_HANDLED
}

/// Oznameni od AML (`Notify`) pro vsechna zarizeni.
unsafe extern "C" fn notify(device: Handle, value: u32, _ctx: *mut c_void) {
    // 0x80: "stisknuto" u tlacitek (ACPI 6.5, 5.6.6).
    let kind = BUTTONS.lock().iter().find(|(h, _)| *h == device as usize).map(|(_, k)| *k);
    match (kind, value) {
        (Some(k), 0x80) => raise(k),
        (Some(_), _) => {}
        (None, v) => libos::println!("acpid: oznameni {v:#x} pro {}", ns::path(device)),
    }
}

/// GED: preruseni -> `_EVT(cislo)`.
fn install_ged() {
    for dev in ns::devices(c"ACPI0013") {
        let irqs = match ns::interrupts(dev) {
            Ok(v) => v,
            Err(e) => {
                libos::println!("acpid: GED {}: _CRS nelze precist ({})", ns::path(dev), acpica::status_name(e));
                continue;
            }
        };
        for irq in irqs {
            let flags = if irq.edge != 0 { abi::syscall::line_flags::EDGE } else { 0 };
            let obj = match crate::osl::env().resource.interrupt_line_with(irq.number, flags) {
                Ok(o) => o,
                Err(e) => {
                    libos::println!("acpid: GED preruseni {} nelze ziskat: {e}", irq.number);
                    continue;
                }
            };
            let dev_n = dev as usize;
            let number = irq.number;
            let r = crate::osl::hw::spawn_interrupt_thread("acpid-ged", obj, move || {
                let arg = ffi::Object::integer(number as u64);
                if let Err(e) = ns::eval(dev_n as Handle, c"_EVT", &[arg]) {
                    libos::println!("acpid: GED _EVT({number}) selhalo: {}", acpica::status_name(e));
                }
            });
            match r {
                Ok(()) => libos::println!(
                    "acpid: GED {} na preruseni {} ({})",
                    ns::path(dev),
                    irq.number,
                    if irq.edge != 0 { "hrana" } else { "uroven" }
                ),
                Err(e) => libos::println!("acpid: vlakno GED nelze spustit: {e}"),
            }
        }
    }
}

/// Nainstaluje obsluhy tlacitek (po inicializaci ACPICA).
pub fn install(platform: &ffi::Platform) {
    match Event::create() {
        Ok(e) => {
            WAKE.call_once(|| e);
        }
        Err(e) => {
            libos::println!("acpid: event pro udalosti nelze vytvorit: {e}");
            return;
        }
    }
    {
        let mut b = BUTTONS.lock();
        for d in ns::devices(c"PNP0C0C") {
            b.push((d as usize, PowerEvent::PowerButton));
        }
        for d in ns::devices(c"PNP0C0E") {
            b.push((d as usize, PowerEvent::SleepButton));
        }
        if b.iter().any(|(_, k)| *k == PowerEvent::PowerButton) {
            HAS_POWER_BUTTON.store(true, Ordering::Release);
        }
    }
    // SAFETY: obsluhy jsou staticke funkce; kontext nepouzivaji.
    unsafe {
        let st = ffi::AcpiInstallNotifyHandler(ffi::ROOT, ffi::ACPI_ALL_NOTIFY, notify, core::ptr::null_mut());
        if st != ffi::AE_OK {
            libos::println!("acpid: obsluhu oznameni nelze nainstalovat: {}", acpica::status_name(st));
        }
        // Pevna tlacitka jen bez HW-reduced (tam pevne registry nejsou).
        if platform.hw_reduced == 0 {
            if ffi::AcpiInstallFixedEventHandler(ffi::ACPI_EVENT_POWER_BUTTON, fixed_power, core::ptr::null_mut()) == ffi::AE_OK {
                HAS_POWER_BUTTON.store(true, Ordering::Release);
            }
            let _ = ffi::AcpiInstallFixedEventHandler(ffi::ACPI_EVENT_SLEEP_BUTTON, fixed_sleep, core::ptr::null_mut());
        }
    }
    install_ged();
}
