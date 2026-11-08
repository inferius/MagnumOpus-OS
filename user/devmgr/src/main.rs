//! devmgr: spravce zarizeni.
//!
//! Projde sbernice PCI Express, zaloguje nalezena zarizeni a pro znama
//! zarizeni spusti ovladace z initfs (virtio; sitove karty Intel
//! a Realtek podle ID v `nic.rs`). Na x86 navic spusti ovladac radice
//! PS/2 (`ps2`, zarizeni na desce mimo PCI; kdyz radic chybi, skonci).
//! Pro radice USB (xHCI) spusti `xhci` a podle jeho zprav tridni ovladace
//! (`usb.rs`). Pro radice disku NVMe a AHCI (podle tridy PCI) spusti `nvme`
//! a `ahci`; ty samy hlasi nalezene disky (`block`, viz
//! `blockproto::server::report_disks`). Rodici (init) posle po bootstrap kanalu pro kazde zarizeni
//! zpravu s koncem kanalu ke klientovi ovladace: `block` (disk, protokol
//! `blockproto`), `input` (klavesnice, mys, protokol `inputproto::device`),
//! `net` (sitova karta, `netproto::link`), `rng` (zdroj nahody),
//! `display` (grafika, `guiproto::display`), `display-native` (nativni
//! ovladac displeje, viz `native.rs`) nebo `audio` (zvukova karta Intel
//! HDA, `audioproto::device`; po padu ovladace znovu, viz `restart.rs`),
//! a `done`, az jsou nalezena zarizeni pri startu
//! (vcetne USB). Pozdeji pripojena zarizeni USB hlasi dalsimi `input`.
//! Kdyz ceka na ovladac disku dlouho (reset radice, roztoceni disku), posila
//! initu kazdych 5 s `busy`, aby init start zarizeni nevzdal.
//!
//! Ovladace z manifestu (`<jmeno>.driver.toml` v initfs, `manifest.rs`)
//! dostanou zarizeni, pro ktera neni vestaveny ovladac; initu se hlasi
//! zpravou `dev <jmeno>` a po padu je devmgr obnovi.
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
use libos::{Channel, Process, Signals, Startup, println};
use pci::{Ecam, Function};

mod restart;
mod manifest;
mod nic;
mod native;
mod usb;

libos::entry!(main);

/// Jak dlouho cekat na prvni pruchod sbernic USB pred hlasenim `done`.
const USB_READY_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(10);

/// Ovladac pro jeden typ zarizeni virtio.
pub struct Driver {
    /// ID zarizeni PCI: moderni (0x1040 + typ virtio) a prechodne.
    ids: (u16, u16),
    /// Program v initfs.
    program: &'static str,
    /// Zprava pro init s kanalem ke klientovi ovladace.
    message: &'static [u8],
    /// Nejvic preruseni a pameti pro DMA (nejmensi prava: ovladac dostane
    /// jen to, co jeho zarizeni potrebuje).
    irq_max: u32,
    dma_max: u64,
}

const DRIVERS: &[Driver] = &[
    // Blokove zarizeni (typ 2).
    Driver {
        ids: (0x1042, 0x1001),
        program: "virtio-blk",
        message: b"block",
        // Fronta 16 + buffer dat az 1 MiB na pozadavek.
        irq_max: 4,
        dma_max: 4 << 20,
    },
    // Sitova karta (typ 1): 2 fronty po 256 bufferech 2 KiB (1 MiB) + krouzky.
    Driver {
        ids: (0x1041, 0x1000),
        program: "virtio-net",
        message: b"net",
        irq_max: 4,
        dma_max: 2 << 20,
    },
    // Zdroj nahody (typ 4): fronta 4 + jedna stranka dat.
    Driver {
        ids: (0x1044, 0x1005),
        program: "virtio-rng",
        message: b"rng",
        irq_max: 2,
        dma_max: 256 << 10,
    },
    // Grafika (typ 16): fronta prikazu + obraz vystupu po kusech 4 MiB
    // (1920x1200 = 9 MiB; rezerva na zmenu rozliseni). Prechodne rozhrani
    // nema.
    Driver {
        ids: (0x1050, 0x1050),
        program: "virtio-gpu",
        message: b"display",
        irq_max: 4,
        dma_max: 40 << 20,
    },
    // Vstupni zarizeni (typ 18): klavesnice, mys, tablet. Prechodne
    // rozhrani nema.
    Driver {
        ids: (0x1052, 0x1052),
        program: "virtio-input",
        message: b"input",
        irq_max: 4,
        dma_max: 1 << 20,
    },
];

/// Zvukova karta Intel HDA (PCI trida 04:03, libovolny vyrobce): radic,
/// CORB/RIRB a BDL v jedne strance + buffer streamu (max. 256 KiB).
const HDA_CLASS: (u8, u8) = (0x04, 0x03);
const HDA: Driver = Driver {
    ids: (0, 0),
    program: "hda",
    message: b"audio",
    irq_max: 2,
    dma_max: 1 << 20,
};
/// Ovladac radice disku podle tridy PCI. Radic muze mit vic disku (porty
/// SATA, jmenne prostory NVMe): ovladac kazdy nahlasi zpravou `block`
/// s kanalem pro klienta a nakonec `done`.
struct ClassDriver {
    /// Trida, podtrida, programove rozhrani.
    class: (u8, u8, u8),
    program: &'static str,
    /// BAR s registry radice.
    bar: u8,
    irq_max: u32,
    dma_max: u64,
}

const CLASS_DRIVERS: &[ClassDriver] = &[
    // NVMe: fronty + buffer 32 KiB (pod 64 KiB).
    ClassDriver {
        class: (0x01, 0x08, 0x02),
        program: "nvme",
        bar: 0,
        irq_max: 2,
        dma_max: 1 << 20,
    },
    // AHCI: az 32 portu po nejvys 64 KiB (seznam prikazu, FIS, buffer).
    ClassDriver {
        class: (0x01, 0x06, 0x01),
        program: "ahci",
        bar: 5,
        irq_max: 2,
        dma_max: 4 << 20,
    },
];

/// Jak dlouho nejvys cekat, nez ovladac radice disku nahlasi sve disky.
/// Reset radice NVMe smi podle CAP.TO trvat az 127,5 s, roztoceni disku SATA
/// desitky sekund; zaseknuty ovladac ale nesmi start zdrzet navzdy.
const DISK_REPORT_TIMEOUT: core::time::Duration = core::time::Duration::from_secs(150);
/// Behem cekani na ovladac disku posilat initu `busy` po tolika sekundach:
/// init ceka na kazdou zpravu spravce jen omezene dlouho (DEVMGR_TIMEOUT).
const BUSY_INTERVAL: core::time::Duration = core::time::Duration::from_secs(5);

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
    let mut controllers: Vec<usb::Controller> = Vec::new();
    // Ovladace, ktere se po padu spousti znovu (vlakno `restart`).
    let mut watched: Vec<restart::Watched> = Vec::new();
    let mut from_manifest = manifest::Drivers::load(&fs);
    for (window, (vmo, arg)) in startup.pci_ecam.iter().enumerate() {
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
        for f in functions.iter().filter(|f| (f.class, f.subclass) == HDA_CLASS) {
            let Some(image) = fs.find(HDA.program) else { continue };
            match start_driver_image(&HDA, image, resource, vmo, &ecam, (*arg >> 16) as u16, f, &startup.bootstrap) {
                Ok(process) => watched.push(restart::Watched { driver: &HDA, image, function: *f, window, process }),
                Err(e) => println!("devmgr: ovladac hda pro {} nelze spustit: {e}", f.address),
            }
        }
        for driver in CLASS_DRIVERS {
            for f in functions
                .iter()
                .filter(|f| (f.class, f.subclass, f.prog_if) == driver.class)
            {
                let segment = (*arg >> 16) as u16;
                match start_disk_controller(driver, &fs, resource, vmo, &ecam, segment, f, &startup.bootstrap) {
                    Ok(p) => drivers.push(p),
                    Err(e) => println!(
                        "devmgr: ovladac {} pro {} nelze spustit: {e}",
                        driver.program, f.address
                    ),
                }
            }
        }
        // Sitove karty az po virtio: prvni karta pro netd zustava virtio-net.
        nic::start_all(&functions, &fs, resource, vmo, &ecam, (*arg >> 16) as u16, &startup.bootstrap, &mut drivers);
        // Nativni ovladace displeje (nastaveni obrazu nad framebufferem firmwaru).
        for f in &functions {
            native::start(&fs, resource, vmo, &ecam, (*arg >> 16) as u16, f, &startup.bootstrap, &mut drivers);
        }
        for f in functions
            .iter()
            .filter(|f| (f.class, f.subclass, f.prog_if) == usb::XHCI_CLASS)
        {
            match start_xhci(&fs, resource, vmo, &ecam, (*arg >> 16) as u16, f) {
                Ok(c) => controllers.push(c),
                Err(e) => println!("devmgr: ovladac xhci pro {} nelze spustit: {e}", f.address),
            }
        }
        from_manifest.claim(&startup, &fs, &ecam, window, &functions, builtin, &startup.bootstrap);
    }
    from_manifest.report_unused();
    #[cfg(target_arch = "x86_64")]
    match start_ps2(&fs, resource, &startup.bootstrap) {
        Ok(Some(p)) => drivers.push(p),
        Ok(None) => {}
        Err(e) => println!("devmgr: ovladac ps2 nelze spustit: {e}"),
    }
    // Zarizeni USB pri startu: pockat, az radice projdou porty.
    let deadline = libos::time::deadline_after(USB_READY_TIMEOUT);
    while controllers.iter().any(|c| !c.ready) {
        if !serve_usb(&fs, &mut controllers, &startup, &mut drivers, &mut from_manifest, Some(deadline)) {
            println!("devmgr: radic USB nedokoncil pruchod porty");
            break;
        }
    }
    let _ = startup.bootstrap.write(b"done", Vec::new());
    // Restart spadlych ovladacu ve vlastnim vlakne (handly jsou duplikaty).
    let ctx = (|| -> libos::Result<restart::Ctx> {
        let mut windows = Vec::new();
        for (v, a) in &startup.pci_ecam {
            windows.push((libos::Vmo::from_handle(v.handle().duplicate(None)?), *a));
        }
        Ok(restart::Ctx {
            resource: libos::hw::Resource::from_handle(resource.handle().duplicate(None)?),
            windows,
            parent: Channel::from_handle(startup.bootstrap.handle().duplicate(None)?),
        })
    })();
    match ctx {
        Ok(ctx) => restart::spawn(watched, ctx),
        Err(e) => println!("devmgr: restart ovladacu nebude: {e}"),
    }
    // Zustat bezet, dokud init drzi spojeni: hotplug USB (pozdeji restart
    // spadlych ovladacu).
    while serve_usb(&fs, &mut controllers, &startup, &mut drivers, &mut from_manifest, None) {}
    drop(drivers);
    0
}

/// Ma funkce vestaveny ovladac (tabulky vyse, xHCI, sitove karty `nic`)? Ovladace z manifestu
/// dostanou jen ostatni.
fn builtin(f: &Function) -> bool {
    let class = (f.class, f.subclass, f.prog_if);
    DRIVERS.iter().any(|d| f.vendor == virtio::VENDOR && (f.device == d.ids.0 || f.device == d.ids.1))
        || CLASS_DRIVERS.iter().any(|d| d.class == class)
        || class == usb::XHCI_CLASS
        || ::nic::ids::program_for(f.vendor, f.device).is_some()
}

/// Jedno kolo cekani na radice USB, konec ovladacu z manifestu (a zavreni
/// initu). `false` = konec (init skoncil nebo vyprsel termin).
fn serve_usb(
    fs: &Initfs,
    controllers: &mut [usb::Controller],
    startup: &Startup,
    procs: &mut Vec<Process>,
    from_manifest: &mut manifest::Drivers,
    deadline: Option<u64>,
) -> bool {
    let parent = &startup.bootstrap;
    let mut items = vec![(parent.handle(), Signals::PEER_CLOSED)];
    // Zavreny kanal by mel PEER_CLOSED porad nastaveny a cekani by se
    // vracelo okamzite: skoncene radice vynechat.
    let open: Vec<usize> = (0..controllers.len()).filter(|&i| !controllers[i].closed).collect();
    for &i in &open {
        items.push((controllers[i].channel.handle(), Signals::READABLE | Signals::PEER_CLOSED));
    }
    let first_driver = items.len();
    items.extend(from_manifest.wait_items());
    let observed = match libos::wait_many(&items, deadline) {
        Ok((_, o)) => o,
        Err(_) => return false,
    };
    drop(items);
    if observed[0].intersects(Signals::PEER_CLOSED) {
        return false;
    }
    from_manifest.reap(&observed[first_driver..], startup, fs, parent);
    for (k, &i) in open.iter().enumerate() {
        let c = &mut controllers[i];
        let s = observed[k + 1];
        if s.intersects(Signals::READABLE) {
            while let Ok(m) = c.channel.try_read() {
                if usb::handle(fs, m, parent, procs) {
                    c.ready = true;
                }
            }
        } else if s.intersects(Signals::PEER_CLOSED) {
            // Radic skoncil: nic dalsiho nehlasi.
            println!("devmgr: ovladac radice USB skoncil");
            c.ready = true;
            c.closed = true;
        }
    }
    true
}

/// Spusti ovladac radice xHCI: jako virtio, navic kanal ke spravci
/// (hlaseni rozhrani) a adresa a velikost BAR0.
fn start_xhci(
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &libos::Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
) -> libos::Result<usb::Controller> {
    let image = fs.find("xhci").ok_or(libos::Error::NotFound)?;
    let a = f.address;
    let bar = ecam.memory_bar(a, 0).ok_or(libos::Error::NotSupported)?;
    let bar_size = ecam.memory_bar_size(a, 0).ok_or(libos::Error::NotSupported)?;
    let (ranges, config_phys) = device_ranges(ecam, ecam_vmo, resource, f)?;
    let id = (segment, a.bus, a.device, a.function);
    let device = resource.restrict_device(&ranges, XHCI_IRQ_MAX, XHCI_DMA_MAX, id)?;
    let config = device.device_memory(config_phys, pci::CONFIG_SIZE)?;
    let (mine, theirs) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
        StartHandle::user(theirs.into_handle()),
    ];
    let (process, control) = libos::spawn("xhci", image, handles)?;
    control.write(format!("pci {} {} {}", a.bus, a.device, a.function).as_bytes(), Vec::new())?;
    control.write(format!("bar0 {} {}", bar.phys, bar_size).as_bytes(), Vec::new())?;
    Ok(usb::Controller {
        _process: process,
        channel: mine,
        ready: false,
        closed: false,
    })
}

/// Spusti ovladac radice disku a initu preda kanaly k disku, ktere nahlasi.
#[allow(clippy::too_many_arguments)]
fn start_disk_controller(
    driver: &ClassDriver,
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &libos::Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
    parent: &Channel,
) -> libos::Result<Process> {
    let image = fs.find(driver.program).ok_or(libos::Error::NotFound)?;
    let a = f.address;
    let bar = ecam.memory_bar(a, driver.bar).ok_or(libos::Error::NotSupported)?;
    let bar_size = ecam
        .memory_bar_size(a, driver.bar)
        .ok_or(libos::Error::NotSupported)?;
    let (ranges, config_phys) = device_ranges(ecam, ecam_vmo, resource, f)?;
    let id = (segment, a.bus, a.device, a.function);
    let device = resource.restrict_device(&ranges, driver.irq_max, driver.dma_max, id)?;
    let config = device.device_memory(config_phys, pci::CONFIG_SIZE)?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
    ];
    let (process, control) = libos::spawn(driver.program, image, handles)?;
    control.write(format!("pci {} {} {}", a.bus, a.device, a.function).as_bytes(), Vec::new())?;
    control.write(format!("bar {} {}", bar.phys, bar_size).as_bytes(), Vec::new())?;
    let deadline = libos::time::deadline_after(DISK_REPORT_TIMEOUT);
    loop {
        let slice = libos::time::deadline_after(BUSY_INTERVAL).min(deadline);
        let msg = match control.read(Some(slice)) {
            Ok(m) => m,
            Err(libos::Error::TimedOut) if slice < deadline => {
                // Ovladac jeste pracuje: init nesmi start zarizeni vzdat.
                parent.write(b"busy", Vec::new())?;
                continue;
            }
            Err(e) => {
                println!("devmgr: ovladac {} pro {} nenahlasil disky: {e}", driver.program, a);
                break;
            }
        };
        match &msg.data[..] {
            b"block" => parent.write(b"block", msg.handles)?,
            b"done" => break,
            other => println!("devmgr: {}: neznama zprava {:?}", driver.program, other),
        }
    }
    Ok(process)
}

/// Preruseni a DMA pameti pro ovladac radice xHCI.
const XHCI_IRQ_MAX: u32 = 8;
const XHCI_DMA_MAX: u64 = 4 * 1024 * 1024;

/// Prostredky jednoho zarizeni: jeho pametove BARy (zaokrouhlene na
/// stranky) a stranka jeho konfiguracniho prostoru.
pub(crate) fn device_ranges(
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
    start_driver_image(driver, image, resource, ecam_vmo, ecam, segment, f, parent)
}

/// Spusti ovladac z obrazu `image` (i z vlakna restartu).
#[allow(clippy::too_many_arguments)]
pub fn start_driver_image(
    driver: &Driver,
    image: &[u8],
    resource: &libos::hw::Resource,
    ecam_vmo: &libos::Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
    parent: &Channel,
) -> libos::Result<Process> {
    // Ovladac dostane jen sve zarizeni: BARy, svou konfiguraci, kvoty.
    let (ranges, config_phys) = device_ranges(ecam, ecam_vmo, resource, f)?;
    let a = f.address;
    let id = (segment, a.bus, a.device, a.function);
    let device = resource.restrict_device(&ranges, driver.irq_max, driver.dma_max, id)?;
    let config = device.device_memory(config_phys, pci::CONFIG_SIZE)?;
    let (client_end, driver_end) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
        StartHandle::user(driver_end.into_handle()),
    ];
    let (process, control) = libos::spawn::service(driver.program, image, handles)?;
    let config = format!("pci {} {} {}", a.bus, a.device, a.function);
    control.write(config.as_bytes(), Vec::new())?;
    // Konec kanalu ke klientovi patri initu (ten ho preda souborovemu
    // systemu, sluzbe vstupu nebo sluzbe site).
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
    let (process, control) = libos::spawn::service("ps2", image, handles)?;
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
