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
    dma_max: u64,
    /// Dalsi fyzicke rozsahy mimo BARy (Intel: stolen pamet s
    /// framebufferem firmwaru a druhym bufferem).
    extra: fn(&Ecam, pci::Address) -> Vec<(u64, u64)>,
}

fn none(_: &Ecam, _: pci::Address) -> Vec<(u64, u64)> {
    Vec::new()
}

/// Stolen pamet integrovane grafiky Intel (BDSM 0x5C, velikost z GGC
/// 0x50 bity 15:8 - kodovani Gen9+, `i915_gem_stolen.c`). MTL+ ma stolen
/// pamet jinde: prazdne (ovladac pak jen jeden buffer v aperture/BAR).
fn intel_dsm(ecam: &Ecam, a: pci::Address) -> Vec<(u64, u64)> {
    let base = (ecam.read32(a, 0x5C) & 0xFFF0_0000) as u64;
    let gms = (ecam.read16(a, 0x50) >> 8) as u64;
    let size = if gms < 0xF0 { gms * (32 << 20) } else { (gms - 0xF0 + 1) * (4 << 20) };
    if base == 0 || size == 0 { Vec::new() } else { vec![(base, size)] }
}

const NATIVE: &[Native] = &[
    // Bochs VBE (bochs-display, stdvga, secondary-vga): stejne MMIO registry.
    Native {
        program: "bochs-display",
        matches: |f| f.vendor == 0x1234 && f.device == 0x1111,
        irq_max: 0,
        dma_max: 0,
        extra: none,
    },
    // Integrovana grafika Intel (trida 03); generaci overi ovladac sam.
    Native {
        program: "intel-display",
        matches: |f| f.vendor == 0x8086 && f.class == 0x03,
        // Bez preruseni a DMA: buffery jsou ve stolen pameti / pameti karty.
        irq_max: 0,
        dma_max: 0,
        extra: intel_dsm,
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
    let (mut ranges, config_phys) = crate::device_ranges(ecam, ecam_vmo, resource, f)?;
    let a = f.address;
    ranges.extend((n.extra)(ecam, a));
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
