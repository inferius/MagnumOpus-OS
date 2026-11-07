//! init: prvni proces v userspace.
//!
//! Spusti sluzbu casu, spravce zarizeni (ten spusti ovladace), sluzbu
//! souboroveho systemu a sluzbu site, pak testy rozhrani jadra, libos,
//! ovladacu, FS a site (`tests/`). Pozdeji bude spoustet systemove sluzby
//! podle konfigurace.

#![no_std]
#![no_main]

extern crate alloc;

mod system;
mod tests;

use libos::{Startup, println};

libos::entry!(main);

fn main(startup: Startup) -> i64 {
    println!("init: bezim v userspace");
    match system::start_time(&startup) {
        Ok(t) => tests::set_time_service(t),
        Err(e) => println!("init: sluzbu casu nelze spustit: {e}"),
    }
    match system::start_devices(&startup) {
        Ok(mut devices) => {
            if let Err(e) = system::start_fs(&startup, &mut devices) {
                println!("init: souborovy system nelze spustit: {e}");
            }
            match system::start_net(&startup, &mut devices) {
                Ok(n) => tests::set_net_service(n),
                Err(e) => println!("init: sluzbu site nelze spustit: {e}"),
            }
            tests::set_devices(devices);
        }
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
