//! Nativni ovladace displeje (nastaveni obrazu bez akcelerace):
//! `bochs-display` (QEMU) a `intel-display` (Intel Gen9 az Xe2).
//!
//! Ovladac spusti spravce zarizeni, ale init ho po padu spousti znovu
//! (v rezimu obnovy stavu firmwaru), proto initu posle vse potrebne:
//! zprava `display-native <program> <sbernice> <zarizeni> <funkce>`
//! s handly [konec klienta, konec ovladace, prostredek zarizeni,
//! konfigurace PCI, zaznam firmwaru, proces]. Konec ovladace drzi init,
//! takze kanal ke kompozitoru prezije pad ovladace.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Process, Vmo, println};
use pci::{Ecam, Function};

/// Velikost zaznamu stavu firmwaru (`displaydrv::runtime::SNAPSHOT_SIZE`).
const SNAPSHOT_SIZE: u64 = 64 * 1024;

struct Native {
    program: &'static str,
    matches: fn(&Function) -> bool,
    irq_max: u32,
    /// Pamet pro buffery obrazu (Intel: 2 x 3840x2160x4 + rezerva).
    dma_max: u64,
}

const NATIVE: &[Native] = &[
    // Bochs VBE (bochs-display, stdvga, secondary-vga): stejne MMIO registry.
    Native {
        program: "bochs-display",
        matches: |f| f.vendor == 0x1234 && f.device == 0x1111,
        irq_max: 0,
        dma_max: 0,
    },
    // Integrovana grafika Intel (trida 03); generaci overi ovladac sam.
    Native {
        program: "intel-display",
        matches: |f| f.vendor == 0x8086 && f.class == 0x03,
        irq_max: 2,
        dma_max: 72 << 20,
    },
];

#[allow(clippy::too_many_arguments)]
pub fn start(
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
    parent: &Channel,
    procs: &mut Vec<Process>,
) {
    let Some(n) = NATIVE.iter().find(|n| (n.matches)(f)) else { return };
    match spawn(n, fs, resource, ecam_vmo, ecam, segment, f, parent) {
        Ok(p) => procs.push(p),
        Err(e) => println!("devmgr: ovladac {} pro {} nelze spustit: {e}", n.program, f.address),
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    n: &Native,
    fs: &Initfs,
    resource: &libos::hw::Resource,
    ecam_vmo: &Vmo,
    ecam: &Ecam,
    segment: u16,
    f: &Function,
    parent: &Channel,
) -> libos::Result<Process> {
    let image = fs.find(n.program).ok_or(libos::Error::NotFound)?;
    let (ranges, config_phys) = crate::device_ranges(ecam, ecam_vmo, resource, f)?;
    let a = f.address;
    let device = resource.restrict_device(&ranges, n.irq_max, n.dma_max, (segment, a.bus, a.device, a.function))?;
    let config = device.device_memory(config_phys, pci::CONFIG_SIZE)?;
    let snapshot = Vmo::create(SNAPSHOT_SIZE)?;
    let (client_end, driver_end) = Channel::create()?;
    let handles = vec![
        StartHandle::user(device.handle().duplicate(None)?),
        StartHandle::user(config.handle().duplicate(None)?),
        StartHandle::user(driver_end.handle().duplicate(None)?),
        StartHandle::user(snapshot.handle().duplicate(None)?),
    ];
    let (process, control) = libos::spawn(n.program, image, handles)?;
    let pci = format!("{} {} {}", a.bus, a.device, a.function);
    control.write(format!("pci {pci}").as_bytes(), Vec::new())?;
    parent.write(
        format!("display-native {} {pci}", n.program).as_bytes(),
        vec![
            client_end.into_handle(),
            driver_end.into_handle(),
            device.handle().duplicate(None)?,
            config.handle().duplicate(None)?,
            snapshot.into_handle(),
            process.handle().duplicate(None)?,
        ],
    )?;
    Ok(process)
}
