//! Zbytek vrstvy OS: start a konec, prepisy tabulek a predem definovanych
//! jmen (nepouzivame - firmware se bere tak, jak je), signaly z AML.

use core::ffi::c_void;

use super::{AE_BAD_PARAMETER, AE_OK, Status};

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsInitialize() -> Status {
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsTerminate() -> Status {
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsPredefinedOverride(init: *const c_void, new: *mut *mut c_void) -> Status {
    if init.is_null() || new.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: ukazatel od ACPICA.
    unsafe { *new = core::ptr::null_mut() };
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsTableOverride(existing: *mut c_void, new: *mut *mut c_void) -> Status {
    if existing.is_null() || new.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: dtto.
    unsafe { *new = core::ptr::null_mut() };
    AE_OK
}

#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsPhysicalTableOverride(existing: *mut c_void, addr: *mut u64, len: *mut u32) -> Status {
    if existing.is_null() || addr.is_null() || len.is_null() {
        return AE_BAD_PARAMETER;
    }
    // SAFETY: dtto.
    unsafe {
        *addr = 0;
        *len = 0;
    }
    AE_OK
}

/// `AML Fatal` a body preruseni: jen do logu (system pokracuje).
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsSignal(function: u32, _info: *mut c_void) -> Status {
    const ACPI_SIGNAL_FATAL: u32 = 0;
    if function == ACPI_SIGNAL_FATAL {
        libos::println!("acpid: firmware hlasi fatalni chybu (AML Fatal)");
    }
    AE_OK
}

/// Hacek pred zapisem registru spanku: nic, ACPICA pokracuje sama.
#[unsafe(no_mangle)]
pub extern "C" fn AcpiOsEnterSleep(_state: u8, _a: u32, _b: u32) -> Status {
    AE_OK
}
