//! Ovladace sitovych karet s registry v pameti (Intel e1000e a igc,
//! Realtek r8169) podle vendor/device ID (`nic::ids`). Kazda karta dostane
//! vlastni proces jako virtio, navic adresu a velikost okna registru
//! (prvni pametovy BAR; velikost zmeri spravce, ovladac BARy neprepisuje).
//! Kanal ke klientovi jde initu zpravou `net`.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Process, println};
use pci::{Ecam, Function};

/// Preruseni a DMA (2 kruhy po 256 deskriptorech + 512 bufferu po 2 KiB,
/// `nic::Layout::SIZE` ~ 1,03 MiB; rezerva na zaokrouhleni).
const IRQ_MAX: u32 = 4;
const DMA_MAX: u64 = 2 << 20;

/// Spusti ovladace pro vsechny podporovane karty mezi `functions`.
#[allow(clippy::too_many_arguments)]
pub fn start_all(
    functions: &[Function],
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &libos::Vmo,
    ecam: &Ecam,
    segment: u16,
    parent: &Channel,
    procs: &mut Vec<Process>,
) {
    for f in functions {
        let Some(program) = nic::ids::program_for(f.vendor, f.device) else {
            continue;
        };
        match start(program, fs, resource, ecam_vmo, ecam, segment, f, parent) {
            Ok(p) => procs.push(p),
            Err(e) => println!("devmgr: ovladac {program} pro {} nelze spustit: {e}", f.address),
        }
    }
}

/// Prvni pametovy BAR s nenulovou velikosti: (index, adresa, velikost).
fn regs_bar(ecam: &Ecam, f: &Function) -> Option<(u64, u64)> {
    let mut index = 0;
    while index < 6 {
        let bar = ecam.memory_bar(f.address, index);
        if let Some(b) = bar
            && b.phys != 0
            && let Some(size) = ecam.memory_bar_size(f.address, index)
        {
            return Some((b.phys, size));
        }
        index += if bar.is_some_and(|b| b.is_64bit) { 2 } else { 1 };
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn start(
    program: &str,
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &libos::Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
    parent: &Channel,
) -> libos::Result<Process> {
    let image = fs.find(program).ok_or(libos::Error::NotFound)?;
    let (regs, regs_size) = regs_bar(ecam, f).ok_or(libos::Error::NotSupported)?;
    let (ranges, config_phys) = super::device_ranges(ecam, ecam_vmo, resource, f)?;
    let a = f.address;
    let id = (segment, a.bus, a.device, a.function);
    let device = resource.restrict_device(&ranges, IRQ_MAX, DMA_MAX, id)?;
    let config = device.device_memory(config_phys, pci::CONFIG_SIZE)?;
    let (client_end, driver_end) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
        StartHandle::user(driver_end.into_handle()),
    ];
    let (process, control) = libos::spawn(program, image, handles)?;
    control.write(format!("pci {} {} {}", a.bus, a.device, a.function).as_bytes(), Vec::new())?;
    control.write(format!("regs {regs} {regs_size}").as_bytes(), Vec::new())?;
    parent.write(b"net", vec![client_end.into_handle()])?;
    Ok(process)
}
