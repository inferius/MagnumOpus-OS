//! Testy rozhrani jadra z userspace.
//!
//! Na rozdil od testu jadra test nepanikuje, ale vraci `Result<(), String>`
//! s popisem chyby, aby jeden neuspech nezastavil ostatni testy.

mod basic;
mod dns;
mod clock_sync;
mod drivers;
mod events;
mod firewall;
mod fs;
mod fs_data;
mod fs_hist;
mod fs_recovery;
mod gui;
mod input;
mod ipc;
mod jobs;
mod memory;
mod net;
mod process;
mod recovery;
mod recovery_kit;
mod console;
mod shell;
mod supervisor;
mod time;
mod usb;
mod web;

use alloc::string::String;
use alloc::vec::Vec;

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
    ("synchronizace casu", clock_sync::TESTS),
    ("firewall", firewall::TESTS),
    ("sit", net::TESTS),
    ("TLS a HTTP", web::TESTS),
    ("sifrovane DNS", dns::TESTS),
    ("supervisor", supervisor::TESTS),
    ("obnova sluzeb", recovery::TESTS),
    ("shell", shell::TESTS),
    // GUI posledni: kompozitor prevezme fokus vstupu.
    ("gui", gui::TESTS),
    // Posledni: zabije hlavni fsd (relace s listkem obnovi hlavni supervisor, ktery
    // v testovacim behu ridi jen tato sada).
    ("obnova fsd", fs_recovery::TESTS),
];

/// Zarizeni spustena pri startu (pro testy ovladacu a souboroveho systemu).
fn devices() -> Option<&'static crate::system::Devices> {
    crate::system::devices()
}

/// Sluzba site spustena pri startu (pro testy site).
fn net_service() -> Option<&'static crate::system::NetService> {
    crate::system::NET.get()
}

/// Vyber testu z initfs `test-filter` (`cargo xtask test --only ...
/// --repeat N`): `suites = sit,firewall` (jmena sad, prazdne = vse),
/// `repeat = N` (kolikrat vybrane sady pustit; hledani nestabilnich testu).
fn filter(startup: &Startup) -> (Vec<String>, usize) {
    let Some(text) = libos::initfs::Initfs::open(startup)
        .ok()
        .and_then(|fs| fs.find("test-filter").map(|d| String::from_utf8_lossy(d).into_owned()))
    else {
        return (Vec::new(), 1);
    };
    let mut suites = Vec::new();
    let mut repeat = 1;
    for line in text.lines() {
        match line.split_once('=') {
            Some((k, v)) if k.trim() == "suites" => {
                suites = v.split(',').map(|s| String::from(s.trim())).filter(|s| !s.is_empty()).collect();
            }
            Some((k, v)) if k.trim() == "repeat" => repeat = v.trim().parse().unwrap_or(1).clamp(1, 1000),
            _ => {}
        }
    }
    (suites, repeat)
}

/// Spusti vsechny testy, vraci pocet selhanych.
pub fn run_all(startup: &Startup) -> usize {
    let (only, repeat) = filter(startup);
    let mut failed = 0;
    let mut passed = 0;
    for round in 1..=repeat {
        if repeat > 1 {
            println!("--- kolo {round}/{repeat} ---");
        }
        for (suite, tests) in SUITES {
            if !only.is_empty() && !only.iter().any(|s| s == suite) {
                continue;
            }
            for (name, test) in *tests {
                let start = libos::time::now();
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
                let ms = (libos::time::now() - start) / 1_000_000;
                if ms > 20_000 {
                    println!("[pomaly] {suite}: {name}: {ms} ms");
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
