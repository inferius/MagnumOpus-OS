//! Karantena DMA: zarizeni se zastavi drive, nez se vrati jeho pamet.
//!
//! Zarizeni zapisuje do pameti samo (DMA) podle deskriptoru, ktere mu dal
//! ovladac. Kdyz ovladac skonci (pad, kill, restart), zarizeni muze dal
//! zapisovat - bez IOMMU do stranek, ktere uz dostal nekdo jiny. Proto:
//!
//! 1. konec procesu, ktery zarizeni ridil (sam pro nej vytvoril pamet pro
//!    DMA), preda tuto pamet sem (`driver_gone`),
//! 2. jadro zarizeni zastavi: v konfiguraci PCI vypne bus master (jen bit
//!    COMMAND, nic jineho) a pocka, az zarizeni dokonci rozbehnute prenosy
//!    (PCIe Device Status, Transactions Pending; nejvys 100 ms, bez PCIe
//!    schopnosti pevne 10 ms),
//! 3. teprve pak pamet pusti (posledni odkaz vrati stranky alokatoru).
//!
//! Novy ovladac si bus master zapne sam (`libs/pci`). Zarizeni, ktere nejde
//! zastavit (neni na PCI, nereaguje), si pamet necha v karantene navzdy
//! a jadro to zaloguje s textem `karantena DMA` (diagnostika `report`).
//!
//! Klient, ktery jen sdilel buffer ovladace a skoncil, zarizeni nezastavi:
//! zastavuje se jen zarizeni, pro ktere umirajici proces pamet pro DMA sam
//! vytvoril nebo namapoval (`DMA_MAP`).
//!
//! Mapovani `DMA_MAP` (`dma.rs`) jde sem stejne: zarizeni se nejdriv
//! zastavi, teprve pak se mapovani zrusi (odmapovani s vyprazdnenim IOTLB,
//! pak pamet). Bez IOMMU by jinak zarizeni mohlo psat do stranek, ktere uz
//! dostal nekdo jiny.

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use super::KernelObject;
use super::dma::DmaMap;
use super::vmo::MemoryObject;
use crate::log;
use crate::mm::{PhysAddr, VirtAddr};
use crate::sync::SpinLock;

/// Identita zarizeni PCI: (segment, sbernice, zarizeni << 3 | funkce).
pub type DeviceId = (u16, u8, u8);

/// Pamet a mapovani zarizeni, ktere zustavaji v karantene (zarizeni nejde
/// zastavit).
static HELD: SpinLock<Vec<(Option<DeviceId>, Arc<dyn KernelObject>)>> = SpinLock::new(Vec::new());

/// Namapovane stranky konfigurace PCI (virtualni adresa v jadre); mapuji
/// se jednou na zarizeni.
static CONFIG: SpinLock<BTreeMap<DeviceId, u64>> = SpinLock::new(BTreeMap::new());

const COMMAND: usize = 0x04;
const STATUS: usize = 0x06;
const CAP_PTR: usize = 0x34;
const COMMAND_BUS_MASTER: u16 = 1 << 2;
const STATUS_CAP_LIST: u16 = 1 << 4;
const CAP_PCIE: u8 = 0x10;
/// Device Status v PCIe schopnosti (offset od schopnosti).
const PCIE_DEVSTA: usize = 0x0A;
const DEVSTA_TRANSACTIONS_PENDING: u16 = 1 << 5;
const PENDING_LIMIT: Duration = Duration::from_millis(100);
const NO_PCIE_WAIT: Duration = Duration::from_millis(10);

pub fn fmt_device(d: DeviceId) -> alloc::string::String {
    alloc::format!("{:04x}:{:02x}:{:02x}.{}", d.0, d.1, d.2 >> 3, d.2 & 7)
}

/// Proc zarizeni nejde zastavit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stuck {
    /// Neni v zadnem okne ECAM (neni na PCI Express).
    NotPci,
    /// Rozbehnute prenosy nedobehly do limitu.
    Pending,
}

/// Stranka konfigurace zarizeni (namapovana bez cache), nebo `None`.
fn config_page(d: DeviceId) -> Option<u64> {
    if let Some(&v) = CONFIG.lock().get(&d) {
        return Some(v);
    }
    let platform = crate::PLATFORM.get()?;
    let ecam = platform
        .pci_ecam
        .iter()
        .find(|e| e.segment == d.0 && (e.bus_start..=e.bus_end).contains(&d.1))?;
    let phys = ecam.phys + (((d.1 - ecam.bus_start) as u64) << 20) + ((d.2 as u64) << 12);
    let virt = crate::mm::map_mmio(PhysAddr::new(phys), 4096).as_u64();
    Some(*CONFIG.lock().entry(d).or_insert(virt))
}

fn read16(page: u64, off: usize) -> u16 {
    // SAFETY: `page` je namapovana stranka konfigurace PCI (4 KiB, bez
    // cache), `off` je zarovnany a uvnitr stranky.
    unsafe { core::ptr::read_volatile(VirtAddr::new(page + off as u64).as_mut_ptr::<u16>()) }
}

fn read8(page: u64, off: usize) -> u8 {
    // SAFETY: viz `read16`.
    unsafe { core::ptr::read_volatile(VirtAddr::new(page + off as u64).as_mut_ptr::<u8>()) }
}

/// Vypne zarizeni bus master a pocka na rozbehnute prenosy.
fn quiesce(d: DeviceId) -> Result<(), Stuck> {
    let page = config_page(d).ok_or(Stuck::NotPci)?;
    if read16(page, 0) == 0xFFFF {
        return Ok(()); // zarizeni uz neni (odpojeno)
    }
    let cmd = read16(page, COMMAND);
    if cmd & COMMAND_BUS_MASTER != 0 {
        // SAFETY: zapis jen registru COMMAND zarizeni, jehoz ovladac
        // skoncil; vypnuti bus master zarizeni zastavi DMA.
        unsafe {
            core::ptr::write_volatile(
                VirtAddr::new(page + COMMAND as u64).as_mut_ptr::<u16>(),
                cmd & !COMMAND_BUS_MASTER,
            )
        };
    }
    // PCIe schopnost (Transactions Pending), jinak pevna pauza.
    let mut pcie = None;
    if read16(page, STATUS) & STATUS_CAP_LIST != 0 {
        let mut ptr = read8(page, CAP_PTR) & 0xFC;
        for _ in 0..48 {
            if ptr < 0x40 {
                break;
            }
            if read8(page, ptr as usize) == CAP_PCIE {
                pcie = Some(ptr as usize);
                break;
            }
            ptr = read8(page, ptr as usize + 1) & 0xFC;
        }
    }
    let Some(cap) = pcie else {
        super::wait::sleep(NO_PCIE_WAIT);
        return Ok(());
    };
    let end = crate::arch::now_ns() + PENDING_LIMIT.as_nanos() as u64;
    while read16(page, cap + PCIE_DEVSTA) & DEVSTA_TRANSACTIONS_PENDING != 0 {
        if crate::arch::now_ns() > end {
            return Err(Stuck::Pending);
        }
        super::wait::sleep(Duration::from_millis(1));
    }
    Ok(())
}

/// Proces skoncil. `driven` = zarizeni, ktera ridil, `dma` =
/// pamet pro DMA, kterou mel (mapovanou nebo za handlem), `maps` = jeho
/// mapovani `DMA_MAP`. Pamet a mapovani zarizeni, ktera ridil, pusti az po
/// jejich zastaveni; ostatni hned (klient sdileneho bufferu - ovladac zije
/// a za pamet odpovida).
pub fn driver_gone(driven: &[DeviceId], dma: Vec<Arc<MemoryObject>>, maps: Vec<Arc<DmaMap>>) {
    let mut by_device: BTreeMap<DeviceId, Vec<Arc<dyn KernelObject>>> = BTreeMap::new();
    for map in maps {
        // Mapovani vytvari jen ovladac zarizeni (prostredek s identitou).
        by_device.entry(map.device()).or_default().push(map);
    }
    for vmo in dma {
        match vmo.dma_device() {
            Some(d) if driven.contains(&d) => by_device.entry(d).or_default().push(vmo),
            Some(_) => drop(vmo),
            None if vmo.is_unprotected_dma() => {
                // Bez identity zarizeni nejde nic zastavit (dnes takova
                // pamet vzniknout nemuze, jen objekty jadra a testu).
                log::warn!("karantena DMA: pamet bez identity zarizeni zustava navzdy ({} B)", vmo.allocated_bytes());
                HELD.lock().push((None, vmo));
            }
            None => drop(vmo),
        }
    }
    for (d, vmos) in by_device {
        match quiesce(d) {
            Ok(()) => {
                log::info!("DMA: zarizeni {} zastaveno, {} objektu pameti vraceno", fmt_device(d), vmos.len());
                drop(vmos);
            }
            Err(why) => {
                let text = match why {
                    Stuck::NotPci => "neni na PCI Express, nejde zastavit",
                    Stuck::Pending => "nedokoncilo prenosy do 100 ms",
                };
                log::warn!(
                    "karantena DMA: zarizeni {} {text}; {} objektu pameti zustava v karantene",
                    fmt_device(d),
                    vmos.len()
                );
                let mut held = HELD.lock();
                for v in vmos {
                    held.push((Some(d), v));
                }
            }
        }
    }
}

/// Kolik objektu pameti je v karantene (testy, diagnostika).
pub fn held_count() -> usize {
    HELD.lock().len()
}
