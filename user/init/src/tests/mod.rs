//! Testy rozhrani jadra z userspace.
//!
//! Na rozdil od testu jadra test nepanikuje, ale vraci `Result<(), String>`
//! s popisem chyby, aby jeden neuspech nezastavil ostatni testy.

mod basic;
mod drivers;
mod events;
mod firewall;
mod fs;
mod fs_data;
mod fs_hist;
mod input;
mod ipc;
mod jobs;
mod memory;
mod net;
mod process;
mod shell;
mod supervisor;
mod time;
mod usb;

use alloc::string::String;

use libos::{Startup, println};

pub type TestResult = Result<(), String>;
type Test = (&'static str, fn(&Startup) -> TestResult);

const SUITES: &[(&str, &[Test])] = &[
    ("zaklad", basic::TESTS),
    ("pamet", memory::TESTS),
    ("ipc", ipc::TESTS),
    ("udalosti a nahoda", events::TESTS),
    ("procesy", process::TESTS),
    ("joby", jobs::TESTS),
    ("ovladace", drivers::TESTS),
    ("souborovy system", fs::TESTS),
    ("vstup", input::TESTS),
    ("usb", usb::TESTS),
    ("historie FS", fs_hist::TESTS),
    ("data FS", fs_data::TESTS),
    ("cas", time::TESTS),
    ("firewall", firewall::TESTS),
    ("sit", net::TESTS),
    ("supervisor", supervisor::TESTS),
    ("shell", shell::TESTS),
];

/// Zarizeni spustena pri startu (pro testy ovladacu a souboroveho systemu).
fn devices() -> Option<&'static crate::system::Devices> {
    crate::system::devices()
}

/// Sluzba site spustena pri startu (pro testy site).
fn net_service() -> Option<&'static crate::system::NetService> {
    crate::system::NET.get()
}

/// Spusti vsechny testy, vraci pocet selhanych.
pub fn run_all(startup: &Startup) -> usize {
    let mut failed = 0;
    let mut passed = 0;
    for (suite, tests) in SUITES {
        for (name, test) in *tests {
            match test(startup) {
                Ok(()) => {
                    println!("[ok] {suite}: {name}");
                    passed += 1;
                }
                Err(e) => {
                    println!("[SELHAL] {suite}: {name}: {e}");
                    failed += 1;
                }
            }
        }
    }
    println!("{passed} testu userspace proslo, {failed} selhalo");
    failed
}

/// Overi rovnost, jinak vrati chybu s popisem.
#[macro_export]
macro_rules! check_eq {
    ($left:expr, $right:expr, $what:expr) => {{
        let (l, r) = (&$left, &$right);
        if l != r {
            return Err(alloc::format!("{}: ocekavano {:?}, je {:?}", $what, r, l));
        }
    }};
}

/// Overi podminku, jinak vrati chybu s popisem.
#[macro_export]
macro_rules! check {
    ($cond:expr, $what:expr) => {{
        if !$cond {
            return Err(alloc::string::String::from($what));
        }
    }};
}

/// Prevod chyby libos na popis testu.
pub fn fail(what: &str, e: libos::Error) -> String {
    alloc::format!("{what}: {e}")
}
