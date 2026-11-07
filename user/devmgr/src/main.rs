//! devmgr: spravce zarizeni.
//!
//! Projde sbernice PCI Express, zaloguje nalezena zarizeni a pro znama
//! zarizeni spusti ovladace z initfs. Na x86 navic spusti ovladac radice
//! PS/2 (`ps2`, zarizeni na desce mimo PCI; kdyz radic chybi, skonci). Rodici (init) posle po bootstrap
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
    #[cfg(target_arch = "x86_64")]
    match start_ps2(&fs, resource, &startup.bootstrap) {
        Ok(Some(p)) => drivers.push(p),
        Ok(None) => {}
        Err(e) => println!("devmgr: ovladac ps2 nelze spustit: {e}"),
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

/// Jak dlouho cekat, nez ovladac PS/2 nahlasi, co nasel (reset klavesnice
/// a mysi trva stovky ms).
#[cfg(target_arch = "x86_64")]
const PS2_REPORT_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(5);

/// Spusti ovladac PS/2 a initu preda kanaly k nalezenym zarizenim
/// (`input`). `Ok(None)` = radic chybi nebo nic nenasel (ovladac skoncil).
#[cfg(target_arch = "x86_64")]
fn start_ps2(
    fs: &Initfs,
    resource: &libos::hw::Resource,
    parent: &Channel,
) -> libos::Result<Option<Process>> {
    use libos::hw::{io_ports, irq_lines};
    let image = fs.find("ps2").ok_or(libos::Error::NotFound)?;
    // Jen data a prikazy radice a jeho dve linky preruseni; zadna DMA.
    let ranges = [
        io_ports(0x60, 1),
        io_ports(0x64, 1),
        irq_lines(1, 1),
        irq_lines(12, 1),
    ];
    let device = resource.restrict(&ranges, 2, 0)?;
    let (kbd_client, kbd_driver) = Channel::create()?;
    let (mouse_client, mouse_driver) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(kbd_driver.into_handle()),
        StartHandle::user(mouse_driver.into_handle()),
    ];
    let (process, control) = libos::spawn("ps2", image, handles)?;
    let msg = control.read(Some(libos::time::deadline_after(PS2_REPORT_TIMEOUT)))?;
    let report = core::str::from_utf8(&msg.data).unwrap_or("");
    let mut words = report.split_whitespace();
    if words.next() != Some("ps2") {
        return Err(libos::Error::Corrupted);
    }
    let mut any = false;
    for w in words {
        let ch = match w {
            "klavesnice" => &kbd_client,
            "mys" => &mouse_client,
            _ => continue,
        };
        parent.write(b"input", vec![ch.handle().duplicate(None)?])?;
        any = true;
    }
    if !any {
        println!("devmgr: radic PS/2 nenalezen");
        return Ok(None);
    }
    Ok(Some(process))
}
