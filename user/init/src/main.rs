//! init: prvni proces v userspace.
//!
//! Zatim spousti testy rozhrani jadra a libos (`tests/`); pozdeji bude
//! spoustet systemove sluzby podle konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod tests;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    println!("init: bezim v userspace");
    let failed = tests::run_all(&startup);
    if failed == 0 {
        println!("init: vsechny testy userspace prosly");
        0
    } else {
        println!("init: {failed} testu userspace selhalo");
        1
    }
}
