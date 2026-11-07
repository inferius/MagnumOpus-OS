//! devmgr: spravce zarizeni.
//!
//! Projde sbernice PCI Express, zaloguje nalezena zarizeni a pro znama
//! zarizeni spusti ovladace z initfs. Rodici (init) posle po bootstrap
//! kanalu pro kazde zarizeni zpravu s koncem kanalu ke klientovi ovladace:
//! `block` (disk, protokol `blockproto`) nebo `input` (klavesnice, mys,
//! protokol `inputproto::device`), a nakonec `done`.
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

/// Ovladac pro jeden typ zarizeni virtio.
struct Driver {
    /// ID zarizeni PCI: moderni (0x1040 + typ virtio) a prechodne.
    ids: (u16, u16),
    /// Program v initfs.
    program: &'static str,
    /// Zprava pro init s kanalem ke klientovi ovladace.
    message: &'static [u8],
}

const DRIVERS: &[Driver] = &[
    // Blokove zarizeni (typ 2).
    Driver {
        ids: (0x1042, 0x1001),
        program: "virtio-blk",
        message: b"block",
    },
    // Vstupni zarizeni (typ 18): klavesnice, mys, tablet. Prechodne
    // rozhrani nema.
    Driver {
        ids: (0x1052, 0x1052),
        program: "virtio-input",
        message: b"input",
    },
];

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
        for driver in DRIVERS {
            for a in virtio::find(&functions, driver.ids.0, driver.ids.1) {
                let f = functions.iter().find(|f| f.address == a).unwrap();
                match start_driver(
                    driver,
                    &fs,
                    resource,
                    vmo,
                    &ecam,
                    (*arg >> 16) as u16,
                    f,
                    &startup.bootstrap,
                ) {
                    Ok(p) => drivers.push(p),
                    Err(e) => println!(
                        "devmgr: ovladac {} pro {} nelze spustit: {e}",
                        driver.program, f.address
                    ),
                }
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

/// Nejvic preruseni a DMA pameti pro jeden ovladac.
const DRIVER_IRQ_MAX: u32 = 8;
const DRIVER_DMA_MAX: u64 = 4 * 1024 * 1024;

/// Prostredky jednoho zarizeni: jeho pametove BARy (zaokrouhlene na
/// stranky) a stranka jeho konfiguracniho prostoru.
fn device_ranges(
    ecam: &Ecam,
    ecam_vmo: &libos::Vmo,
    resource: &libos::hw::Resource,
    f: &Function,
) -> libos::Result<(Vec<(u64, u64)>, u64)> {
    let page = libos::vm::page_size();
    let a = f.address;
    let mut ranges = Vec::new();
    let mut index = 0;
    while index < 6 {
        let bar = ecam.memory_bar(a, index);
        if let (Some(bar), Some(size)) = (bar, ecam.memory_bar_size(a, index))
            && bar.phys != 0
        {
            let start = bar.phys & !(page - 1);
            let end = bar.phys.saturating_add(size).next_multiple_of(page);
            ranges.push((start, end - start));
        }
        index += if bar.is_some_and(|b| b.is_64bit) {
            2
        } else {
            1
        };
    }
    let offset = ecam.config_offset(a).ok_or(libos::Error::InvalidArgs)?;
    let config = resource.phys_addr(ecam_vmo, offset)?;
    ranges.push((config, pci::CONFIG_SIZE));
    Ok((ranges, config))
}

#[allow(clippy::too_many_arguments)]
fn start_driver(
    driver: &Driver,
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &libos::Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
    parent: &Channel,
) -> libos::Result<Process> {
    let image = fs.find(driver.program).ok_or(libos::Error::NotFound)?;
    // Ovladac dostane jen sve zarizeni: BARy, svou konfiguraci, kvoty.
    let (ranges, config_phys) = device_ranges(ecam, ecam_vmo, resource, f)?;
    let a = f.address;
    let id = (segment, a.bus, a.device, a.function);
    let device = resource.restrict_device(&ranges, DRIVER_IRQ_MAX, DRIVER_DMA_MAX, id)?;
    let config = device.device_memory(config_phys, pci::CONFIG_SIZE)?;
    let (client_end, driver_end) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
        StartHandle::user(driver_end.into_handle()),
    ];
    let (process, control) = libos::spawn(driver.program, image, handles)?;
    let config = format!("pci {} {} {}", a.bus, a.device, a.function);
    control.write(config.as_bytes(), Vec::new())?;
    // Konec kanalu ke klientovi patri initu (ten ho preda souborovemu
    // systemu nebo sluzbe vstupu).
    parent.write(driver.message, vec![client_end.into_handle()])?;
    Ok(process)
}
