//! Pamet: alokace pro ACPICA a mapovani fyzicke pameti.
//!
//! `AcpiOsMapMemory` dostane libovolnou fyzickou adresu (tabulka, oblast
//! `SystemMemory` v AML, registr z FADT). Mapuje se po strankach pres
//! `VMO_CREATE_PHYSICAL`: jadro povoli registry zarizeni (bez cache)
//! a pamet firmwaru ACPI (s cache), jinou RAM odmitne - AML tak nemuze cist
//! ani prepsat pamet jadra a procesu. RSDP se cte z kopie od jadra (muze
//! lezet v pameti, kterou jadro uz uvolnilo).

use alloc::vec::Vec;
use core::ffi::c_void;

use abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Mapping, Vmo};
use spin::Mutex;

use super::{AE_BAD_PARAMETER, AE_ERROR, AE_OK, Status, env};

/// Stranka jadra (zarovnani fyzickych objektu pameti).
const KERNEL_PAGE: u64 = 4096;

struct Mapped {
    addr: u64,
    len: u64,
    _map: Mapping,
    _vmo: Vmo,
}

static MAPS: Mutex<Vec<Mapped>> = Mutex::new(Vec::new());

/// Kopie RSDP od jadra (stalá adresa po celou dobu behu).
static RSDP: spin::Once<[u8; abi::acpi::RSDP_LEN]> = spin::Once::new();

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsAllocate(size: u64) -> *mut c_void {
    // SAFETY: malloc z cstd (heap programu).
    unsafe { cstd::stdlib::malloc(size as usize) as *mut c_void }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsFree(p: *mut c_void) {
    // SAFETY: `p` vratil AcpiOsAllocate (kontrakt ACPICA).
    unsafe { cstd::stdlib::free(p as *mut u8) }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetRootPointer() -> u64 {
    env().info.rsdp_phys
}

/// Namapuje fyzicky rozsah; vraci adresu odpovidajici `phys` nebo `None`.
pub fn map(phys: u64, len: u64) -> Option<u64> {
    let info = &env().info;
    let rsdp_end = info.rsdp_phys + abi::acpi::RSDP_LEN as u64;
    if phys >= info.rsdp_phys && phys.checked_add(len)? <= rsdp_end {
        let copy = RSDP.call_once(|| info.rsdp);
        return Some(copy.as_ptr() as u64 + (phys - info.rsdp_phys));
    }
    let base = phys & !(KERNEL_PAGE - 1);
    let page = libos::vm::page_size().max(KERNEL_PAGE);
    let size = (phys.checked_add(len)? - base).max(1).next_multiple_of(page);
    let vmo = env().resource.device_memory(base, size).ok()?;
    let m = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE).ok()?;
    let addr = m.addr();
    MAPS.lock().push(Mapped { addr, len: m.len(), _map: m, _vmo: vmo });
    Some(addr + (phys - base))
}

/// Zrusi mapovani obsahujici adresu `virt`.
pub fn unmap(virt: u64) {
    let mut maps = MAPS.lock();
    if let Some(i) = maps.iter().position(|m| virt >= m.addr && virt < m.addr + m.len) {
        maps.swap_remove(i);
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsMapMemory(phys: u64, len: u64) -> *mut c_void {
    match map(phys, len) {
        Some(a) => a as *mut c_void,
        None => {
            libos::println!("acpid: fyzickou pamet {phys:#x} (+{len}) nelze namapovat");
            core::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsUnmapMemory(virt: *mut c_void, _len: u64) {
    if let Some(r) = RSDP.get() {
        let a = virt as u64;
        let start = r.as_ptr() as u64;
        if a >= start && a < start + r.len() as u64 {
            return;
        }
    }
    unmap(virt as u64);
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsGetPhysicalAddress(_virt: *mut c_void, _phys: *mut u64) -> Status {
    AE_ERROR
}

/// Cteni/zapis jednoho registru ve fyzicke pameti (registry FADT v pameti,
/// GED). Mapuje se na dobu pristupu.
fn access(phys: u64, width: u32, write: Option<u64>) -> Result<u64, Status> {
    let bytes = match width {
        8 | 16 | 32 | 64 => width as u64 / 8,
        _ => return Err(AE_BAD_PARAMETER),
    };
    if phys % bytes != 0 {
        return Err(AE_BAD_PARAMETER);
    }
    let a = map(phys, bytes).ok_or(AE_ERROR)?;
    // SAFETY: `a` je namapovana a zarovnana adresa registru (overeno vyse).
    let v = unsafe {
        match (width, write) {
            (8, None) => (a as *const u8).read_volatile() as u64,
            (16, None) => (a as *const u16).read_volatile() as u64,
            (32, None) => (a as *const u32).read_volatile() as u64,
            (64, None) => (a as *const u64).read_volatile(),
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
                (a as *mut u64).write_volatile(v);
                0
            }
            _ => 0,
        }
    };
    unmap(a);
    Ok(v)
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsReadMemory(phys: u64, value: *mut u64, width: u32) -> Status {
    match access(phys, width, None) {
        Ok(v) => {
            // SAFETY: ukazatel od ACPICA.
            unsafe { *value = v };
            AE_OK
        }
        Err(e) => e,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWriteMemory(phys: u64, value: u64, width: u32) -> Status {
    match access(phys, width, Some(value)) {
        Ok(_) => AE_OK,
        Err(e) => e,
    }
}

/// Lze cist / psat? (jen pro debugger ACPICA, ktery neprekladame)
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsReadable(_p: *mut c_void, _len: u64) -> u8 {
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsWritable(_p: *mut c_void, _len: u64) -> u8 {
    1
}
