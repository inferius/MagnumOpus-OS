//! devmgr: spravce zarizeni.
//!
//! Projde sbernice PCI Express, zaloguje nalezena zarizeni a pro znama
//! zarizeni spusti ovladace z initfs. Rodici (init) posle po bootstrap
//! kanalu pro kazde blokove zarizeni zpravu `block` s koncem kanalu ke
//! klientovi ovladace a nakonec `done`.
//!
//! Startovni zprava: ROOT_RESOURCE, PCI_ECAM (jedno nebo vic oken), INITFS.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Process, Startup, println};
use pci::{Ecam, Function};

libos::entry!(main);

/// Virtio blokove zarizeni: moderni ID 0x1042, prechodne 0x1001.
const VIRTIO_BLK: (u16, u16) = (0x1042, 0x1001);

fn main(startup: Startup) -> i64 {
    let Some(resource) = startup.root_resource.as_ref() else {
        println!("devmgr: chybi pristup k hardwaru (ROOT_RESOURCE)");
        return 1;
    };
    let fs = match Initfs::open(&startup) {
        Ok(f) => f,
        Err(e) => {
            println!("devmgr: initfs: {e}");
            return 1;
        }
    };
    let mut drivers: Vec<Process> = Vec::new();
    for (vmo, arg) in &startup.pci_ecam {
        let ecam = match Ecam::new(vmo, *arg) {
            Ok(e) => e,
            Err(e) => {
                println!("devmgr: okno ECAM nelze namapovat: {e}");
                continue;
            }
        };
        let functions = ecam.scan();
        for f in &functions {
            println!(
                "devmgr: pci {} {:04x}:{:04x} trida {:02x}.{:02x}.{:02x}",
                f.address, f.vendor, f.device, f.class, f.subclass, f.prog_if
            );
        }
        for a in virtio::find(&functions, VIRTIO_BLK.0, VIRTIO_BLK.1) {
            let f = functions.iter().find(|f| f.address == a).unwrap();
            match start_block_driver(&fs, resource, vmo, *arg, f, &startup.bootstrap) {
                Ok(p) => drivers.push(p),
                Err(e) => println!("devmgr: ovladac pro {} nelze spustit: {e}", f.address),
            }
        }
    }
    let _ = startup.bootstrap.write(b"done", Vec::new());
    // Zustat bezet, dokud init drzi spojeni (pozdeji: hotplug, restart
    // spadlych ovladacu).
    let _ = startup.bootstrap.read(None);
    drop(drivers);
    0
}

fn start_block_driver(
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam: &libos::Vmo,
    arg: u32,
    f: &Function,
    parent: &Channel,
) -> libos::Result<Process> {
    let image = fs.find("virtio-blk").ok_or(libos::Error::NotFound)?;
    let (client_end, driver_end) = Channel::create()?;
    let handles = vec![
        StartHandle::user(resource.handle().duplicate(None)?),
        StartHandle::user(ecam.handle().duplicate(None)?),
        StartHandle::user(driver_end.into_handle()),
    ];
    let (process, control) = libos::spawn("virtio-blk", image, handles)?;
    let a = f.address;
    let config = format!("pci {arg} {} {} {}", a.bus, a.device, a.function);
    control.write(config.as_bytes(), Vec::new())?;
    // Konec kanalu ke klientovi patri initu (ten ho preda souborovemu systemu).
    parent.write(b"block", vec![client_end.into_handle()])?;
    Ok(process)
}
