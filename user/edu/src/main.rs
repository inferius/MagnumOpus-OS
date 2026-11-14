//! Program ovladace edu: start nad jadrem. Logika je v knihovne
//! (`src/lib.rs`), aby sla testovat v simulaci na hostiteli.

#![cfg_attr(target_os = "magnumopus", no_std, no_main)]

#[cfg(target_os = "magnumopus")]
libos::entry!(main);

#[cfg(target_os = "magnumopus")]
fn main(startup: libos::Startup) -> i64 {
    driverkit::os::main::<edu::Edu<_>>(startup, "edu")
}

/// Na hostiteli program nic nedela (testy jsou v knihovne).
#[cfg(not(target_os = "magnumopus"))]
fn main() {}
