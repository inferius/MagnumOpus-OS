//! Hardware pro ACPICA: I/O porty, konfigurace PCI a preruseni SCI.
//!
//! - porty: syscally `IOPORT_READ`/`IOPORT_WRITE` pres Resource sluzby
//!   (na aarch64 porty nejsou = chyba),
//! - konfigurace PCI: okna ECAM z tabulky MCFG ([`set_ecam`]); na x86 bez
//!   MCFG porty 0xCF8/0xCFC,
//! - preruseni: vlakno na objekt preruseni linky; po kazdem preruseni
//!   zavola obsluhu ACPICA a az pak potvrdi (linka spoustena urovni se
//!   odmaskuje, az obsluha smaze stav udalosti).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::ffi::c_void;

use abi::vm::{MAP_READ, MAP_WRITE};
use acpica::ffi::{OsdHandler, PciId};
use libos::{Mapping, Signals, Vmo, wait_one};
use spin::Mutex;

use super::{AE_BAD_PARAMETER, AE_ERROR, AE_NOT_EXIST, AE_OK, Status, env};

fn port_width(width: u32) -> Option<u8> {
    match width {
        8 => Some(1),
        16 => Some(2),
        32 => Some(4),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsReadPort(port: u64, value: *mut u32, width: u32) -> Status {
    let (Ok(port), Some(w)) = (u16::try_from(port), port_width(width)) else {
        return AE_BAD_PARAMETER;
    };
    match env().resource.port_read(port, w) {
        Ok(v) => {
            // SAFETY: ukazatel od ACPICA.
            unsafe { *value = v };
            AE_OK
        }
        Err(e) => {
            libos::println!("acpid: cteni portu {port:#x} selhalo: {e}");
            // Jako cteni neexistujiciho portu: same jednicky.
            // SAFETY: dtto.
            unsafe { *value = u32::MAX };
            AE_ERROR
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWritePort(port: u64, value: u32, width: u32) -> Status {
    let (Ok(port), Some(w)) = (u16::try_from(port), port_width(width)) else {
        return AE_BAD_PARAMETER;
    };
    let v = if w < 4 { value & ((1u32 << (8 * w)) - 1) } else { value };
    match env().resource.port_write(port, w, v) {
        Ok(()) => AE_OK,
        Err(e) => {
            libos::println!("acpid: zapis portu {port:#x} selhal: {e}");
            AE_ERROR
        }
    }
}

/// Okno ECAM jednoho segmentu (z MCFG).
#[derive(Clone, Copy)]
pub struct Ecam {
    pub phys: u64,
    pub segment: u16,
    pub bus_start: u8,
    pub bus_end: u8,
}

static ECAM: Mutex<Vec<Ecam>> = Mutex::new(Vec::new());
/// Namapovane stranky konfigurace (segment, sbernice, zarizeni, funkce).
static CONFIG_PAGES: Mutex<BTreeMap<(u16, u16, u16, u16), (Mapping, Vmo)>> = Mutex::new(BTreeMap::new());

/// Nastavi okna ECAM (po nacteni tabulek, z MCFG).
pub fn set_ecam(windows: Vec<Ecam>) {
    *ECAM.lock() = windows;
}

fn config_page(id: &PciId) -> Option<u64> {
    let key = (id.segment, id.bus, id.device, id.function);
    let mut pages = CONFIG_PAGES.lock();
    if let Some((m, _)) = pages.get(&key) {
        return Some(m.addr());
    }
    let bus = u8::try_from(id.bus).ok()?;
    let w = *ECAM.lock().iter().find(|w| w.segment == id.segment && (w.bus_start..=w.bus_end).contains(&bus))?;
    if id.device > 31 || id.function > 7 {
        return None;
    }
    let phys = w.phys + (((bus - w.bus_start) as u64) << 20) + ((id.device as u64) << 15) + ((id.function as u64) << 12);
    let size = libos::vm::page_size().max(4096);
    let vmo = env().resource.device_memory(phys, size).ok()?;
    let m = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE).ok()?;
    let addr = m.addr();
    pages.insert(key, (m, vmo));
    Some(addr)
}

/// Konfigurace pres porty 0xCF8/0xCFC (x86 bez MCFG, jen segment 0).
fn legacy_config(id: &PciId, reg: u32, width: u32, write: Option<u64>) -> Result<u64, Status> {
    if id.segment != 0 || id.bus > 255 || id.device > 31 || id.function > 7 || reg > 255 {
        return Err(AE_NOT_EXIST);
    }
    let r = &env().resource;
    let addr = 0x8000_0000 | (id.bus as u32) << 16 | (id.device as u32) << 11 | (id.function as u32) << 8 | (reg & 0xFC);
    r.port_write(0xCF8, 4, addr).map_err(|_| AE_ERROR)?;
    let port = 0xCFC + (reg & 3) as u16;
    let w = port_width(width).ok_or(AE_BAD_PARAMETER)?;
    match write {
        Some(v) => r.port_write(port, w, v as u32).map(|_| 0).map_err(|_| AE_ERROR),
        None => r.port_read(port, w).map(|v| v as u64).map_err(|_| AE_ERROR),
    }
}

fn pci_access(id: *mut PciId, reg: u32, width: u32, write: Option<u64>) -> Result<u64, Status> {
    if id.is_null() {
        return Err(AE_BAD_PARAMETER);
    }
    // SAFETY: ukazatel od ACPICA.
    let id = unsafe { *id };
    let bytes = match width {
        8 | 16 | 32 | 64 => width / 8,
        _ => return Err(AE_BAD_PARAMETER),
    };
    if reg % bytes != 0 || reg + bytes > 4096 {
        return Err(AE_BAD_PARAMETER);
    }
    let Some(base) = config_page(&id) else {
        if cfg!(target_arch = "x86_64") && width != 64 {
            return legacy_config(&id, reg, width, write);
        }
        return Err(AE_NOT_EXIST);
    };
    let a = base + reg as u64;
    // SAFETY: stranka konfigurace je namapovana, offset zarovnany a uvnitr.
    unsafe {
        Ok(match (width, write) {
            (8, None) => (a as *const u8).read_volatile() as u64,
            (16, None) => (a as *const u16).read_volatile() as u64,
            (32, None) => (a as *const u32).read_volatile() as u64,
            (_, None) => (a as *const u32).read_volatile() as u64 | ((a as *const u32).add(1).read_volatile() as u64) << 32,
            (8, Some(v)) => {
                (a as *mut u8).write_volatile(v as u8);
                0
            }
            (16, Some(v)) => {
                (a as *mut u16).write_volatile(v as u16);
                0
            }
            (32, Some(v)) => {
                (a as *mut u32).write_volatile(v as u32);
                0
            }
            (_, Some(v)) => {
                (a as *mut u32).write_volatile(v as u32);
                (a as *mut u32).add(1).write_volatile((v >> 32) as u32);
                0
            }
        })
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsReadPciConfiguration(id: *mut PciId, reg: u32, value: *mut u64, width: u32) -> Status {
    match pci_access(id, reg, width, None) {
        Ok(v) => {
            // SAFETY: ukazatel od ACPICA.
            unsafe { *value = v };
            AE_OK
        }
        Err(e) => {
            // SAFETY: dtto; neexistujici zarizeni cte same jednicky.
            unsafe { *value = u64::MAX };
            e
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWritePciConfiguration(id: *mut PciId, reg: u32, value: u64, width: u32) -> Status {
    match pci_access(id, reg, width, Some(value)) {
        Ok(_) => AE_OK,
        Err(e) => e,
    }
}

/// Obsluha preruseni: vlakno ceka na objekt preruseni linky a vola
/// obsluhu ACPICA. Volano pro SCI (x86) a pro linky GED (`events`).
pub fn spawn_interrupt_thread(
    name: &str,
    irq: libos::hw::Interrupt,
    mut handler: impl FnMut() + Send + 'static,
) -> libos::Result<()> {
    let t = libos::thread::spawn(name, move || {
        super::sched::register_thread();
        loop {
            if wait_one(irq.handle(), Signals::SIGNALED, None).is_err() {
                libos::println!("acpid: cekani na preruseni selhalo, obsluha konci");
                return;
            }
            handler();
            let _ = irq.ack();
        }
    })?;
    core::mem::forget(t);
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsInstallInterruptHandler(number: u32, handler: Option<OsdHandler>, ctx: *mut c_void) -> Status {
    let Some(handler) = handler else {
        return AE_BAD_PARAMETER;
    };
    let irq = match env().resource.interrupt_line(number) {
        Ok(i) => i,
        Err(e) => {
            libos::println!("acpid: preruseni SCI {number} nelze ziskat: {e}");
            return AE_ERROR;
        }
    };
    let ctx = ctx as usize;
    match spawn_interrupt_thread("acpid-sci", irq, move || {
        // SAFETY: obsluha a kontext od ACPICA (plati do odinstalovani).
        unsafe { handler(ctx as *mut c_void) };
    }) {
        Ok(()) => {
            libos::println!("acpid: SCI na lince {number}");
            AE_OK
        }
        Err(_) => AE_ERROR,
    }
}

/// Odinstalovani obsluhy SCI: ACPICA ho dela jen pri ukonceni, ktere sluzba
/// nedela (vlakno pobezi do konce procesu).
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsRemoveInterruptHandler(_number: u32, _handler: Option<OsdHandler>) -> Status {
    AE_OK
}
