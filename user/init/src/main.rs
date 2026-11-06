//! init: prvni proces v userspace.
//!
//! Spusti spravce zarizeni (ten spusti ovladace), pak testy rozhrani jadra,
//! libos a ovladacu (`tests/`). Pozdeji bude spoustet systemove sluzby podle
//! konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod system;
mod tests;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    println!("init: bezim v userspace");
    match system::start_devices(&startup) {
        Ok(devices) => tests::set_devices(devices),
        Err(e) => println!("init: spravce zarizeni nelze spustit: {e}"),
    }
    let failed = tests::run_all(&startup);
    if failed == 0 {
        println!("init: vsechny testy userspace prosly");
        0
    } else {
        println!("init: {failed} testu userspace selhalo");
        1
    }
}
