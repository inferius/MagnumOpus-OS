//! Self-testy jadra, spoustene vlaknem `init` pri startu.
//!
//! Kazda oblast ma vlastni soubor s konstantou `TESTS`. Test pri selhani
//! panikuje (panika vypise zpravu, misto a zasobnik). S feature `qemu-test`
//! jadro po testech ukonci QEMU s vysledkem (pouziva `cargo xtask test`).

mod log;
mod mm;
mod mm_lazy;
mod mm_regions;
mod mm_cow;
mod object;
mod platform;
mod sched;
mod sched_qos;
mod userspace;

use alloc::sync::Arc;
use core::time::Duration;

use crate::log as klog;
use crate::object::Signals;
use crate::object::wait::{self, deadline_after};
use crate::sched::Thread;

/// Skupina testu: (jmeno, test).
type Suite = &'static [(&'static str, fn())];

const SUITES: &[(&str, Suite)] = &[
    ("pamet", mm::TESTS),
    ("pamet", mm_lazy::TESTS),
    ("pamet", mm_regions::TESTS),
    ("pamet", mm_cow::TESTS),
    ("platforma", crate::arch::TESTS),
    ("platforma", platform::TESTS),
    ("log", log::TESTS),
    ("planovac", sched::TESTS),
    ("planovac", sched_qos::TESTS),
    ("objekty", object::TESTS),
    ("userspace", userspace::TESTS),
];

/// Spusti vsechny testy. Vraci true, kdyz prosly (pri selhani se panikuje,
/// takze false zatim nenastava - navratova hodnota je pro budouci testy,
/// ktere selhani hlasi bez paniky).
pub fn run_all() -> bool {
    let mut count = 0;
    for (suite, tests) in SUITES {
        for (name, test) in *tests {
            klog::debug!("spoustim {suite}: {name}");
            test();
            klog::info!("[ok] {suite}: {name}");
            count += 1;
        }
    }
    klog::info!("{count} testu proslo");
    true
}

/// Pocka na konec vlakna, jinak panikuje s popisem.
fn join(t: &Arc<Thread>, timeout: Duration, what: &str) {
    if let Err(e) = wait::wait_one(&**t, Signals::TERMINATED, Some(deadline_after(timeout))) {
        crate::sched::dump_state(&[t.clone(), crate::sched::current()]);
        panic!("{what}: vlakno {} neskoncilo ({e})", t.name);
    }
}
