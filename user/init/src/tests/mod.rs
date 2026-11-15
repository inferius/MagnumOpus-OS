//! Testy rozhrani jadra z userspace.
//!
//! Na rozdil od testu jadra test nepanikuje, ale vraci `Result<(), String>`
//! s popisem chyby, aby jeden neuspech nezastavil ostatni testy.

mod audio;
mod media;
mod media_ffmpeg;
mod audio_recovery;
mod accounts;
mod agent;
mod agent_shell;
mod basic;
pub mod diag;
mod dns;
mod clock_sync;
mod disks;
mod display;
mod drivers;
mod driverkit;
mod events;
mod dma;
mod fences;
mod firewall;
mod dynlink;
mod fpu;
mod fpu_avx;
mod fs;
mod fs_data;
mod fs_datasets;
mod gui_files;
mod fs_hist;
mod fs_perf;
mod fs_recovery;
mod gui;
mod sdl;
mod gui_gpu;
mod gui_video;
mod gui_player;
mod gui_recovery;
mod gui_palette;
mod gui_snapshot;
mod gui_term;
mod input;
mod input_recovery;
mod ipc;
mod jobs;
mod memory;
mod memory_lazy;
mod memory_regions;
mod memory_seal;
mod memory_cow;
mod memory_wx;
pub mod net;
mod net_ready;
mod net_v2;
mod nic;
pub mod power;
mod process;
mod thread_state;
mod exceptions;
mod exceptions_fast;
mod reboot;
mod recovery;
mod recovery_kit;
mod console;
mod shell;
mod cports;
mod cpp;
mod cstd;
mod std_app;
mod std_net;
mod std_proc;
mod std_ports;
mod supervisor;
mod tasks;
mod time;
mod usb;
mod web;
mod vulkan;

use alloc::string::String;
use alloc::vec::Vec;

use libos::{Startup, println};

pub type TestResult = Result<(), String>;
type Test = (&'static str, fn(&Startup) -> TestResult);

const SUITES: &[(&str, &[Test])] = &[
    ("zaklad", basic::TESTS),
    ("pamet", memory::TESTS),
    ("pamet", memory_lazy::TESTS),
    ("pamet", memory_regions::TESTS),
    ("pamet", memory_seal::TESTS),
    ("pamet", memory_cow::TESTS),
    ("pamet", memory_wx::TESTS),
    ("ipc", ipc::TESTS),
    ("udalosti a nahoda", events::TESTS),
    ("ploty", fences::TESTS),
    ("dma", dma::TESTS),
    ("procesy", process::TESTS),
    ("procesy", thread_state::TESTS),
    ("procesy", exceptions::TESTS),
    ("procesy", exceptions_fast::TESTS),
    ("procesy", fpu::TESTS),
    ("procesy", dynlink::TESTS),
    ("joby", jobs::TESTS),
    ("ovladace", drivers::TESTS),
    ("ovladace z manifestu", driverkit::TESTS),
    ("disky", disks::TESTS),
    ("sitove karty", nic::TESTS),
    ("acpi", power::TESTS),
    ("diagnostika", diag::TESTS),
    ("souborovy system", fs::TESTS),
    ("std", std_app::TESTS),
    ("ucty", accounts::TESTS),
    ("vstup", input::TESTS),
    ("usb", usb::TESTS),
    ("zvuk", audio::TESTS),
    ("media", media::TESTS),
    ("media ffmpeg", media_ffmpeg::TESTS),
    ("historie FS", fs_hist::TESTS),
    // Pri bezne sade jeden start (pocitadlo 1); xtask boottest ji pousti
    // opakovane nad stejnym diskem.
    ("opakovany-start", reboot::TESTS),
    ("data FS", fs_data::TESTS),
    ("cas", time::TESTS),
    ("synchronizace casu", clock_sync::TESTS),
    ("firewall", firewall::TESTS),
    ("sit", net::TESTS),
    ("std sit", std_net::TESTS),
    ("std procesy", std_proc::TESTS),
    ("std porty", std_ports::TESTS),
    ("cstd", cstd::TESTS),
    ("cstd sit", cstd::NET_TESTS),
    ("cstd porty", cports::TESTS),
    ("cstd dyn", cports::DYN_TESTS),
    ("cpp", cpp::TESTS),
    ("sit soubeh", net_v2::TESTS),
    ("sit pripravenost", net_ready::TESTS),
    ("TLS a HTTP", web::TESTS),
    ("sifrovane DNS", dns::TESTS),
    ("supervisor", supervisor::TESTS),
    ("obnova sluzeb", recovery::TESTS),
    ("shell", shell::TESTS),
    ("ulohy", tasks::TESTS),
    // Vulkan pred GUI: render uzel otevre init pres displej, ktery pak
    // prevezme kompozitor.
    ("vulkan", vulkan::TESTS),
    // Nastaveni obrazu (bochs-display, vlastni compd) pred GUI.
    ("displej", display::TESTS),
    ("agent AI", agent::TESTS),
    // GUI posledni: kompozitor prevezme fokus vstupu.
    ("gui", gui::TESTS),
    // Port SDL: okno kompozitoru (sdili ho se sadou gui, bezi-li).
    ("sdl", sdl::TESTS),
    // Zvuk po padu: ridi hlavni supervisor (jako obnova fsd, pred ni).
    ("obnova zvuku", audio_recovery::TESTS),
    // Po vsech testech FS: cena radku shellu na plnem svazku.
    ("vykon FS", fs_perf::TESTS),
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
            // Pamet po sade (job initu = cely system): unik se pozna podle
            // toho, ktera sada pamet nevrati.
            if let Ok(i) = libos::job::current().and_then(|j| j.info()) {
                let mib = |b: u64| b / (1024 * 1024);
                println!(
                    "init: pamet po sade {suite}: uzivatel {} MiB, jadro {} MiB",
                    mib(i.user.used),
                    mib(i.kernel.used)
                );
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
