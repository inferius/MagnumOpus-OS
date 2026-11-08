//! ACPICA (referencni implementace ACPI od Intelu, licence BSD-3 z dualni
//! BSD/GPL) prelozena clangem pro nase cile a rozhrani pro Rust.
//!
//! - zdroje: `third_party/acpica` (pevna verze, viz README tam),
//! - konfigurace platformy: `include/acmagnum.h`,
//! - preklad: `build.rs` pres `libs/cbuild` (stejne priznaky a ochrany jako
//!   veskery kod v C),
//! - knihovna C: `libs/cstd` (crate se prilinkuje sam - ACPICA vola
//!   `strlen`, `memcpy`, ...),
//! - vrstvu OS (`AcpiOs*`) dodava program, ktery ACPICA pouziva
//!   (`user/acpid/src/osl`); bez ni se program neslinkuje.
//!
//! [`ffi`] jsou surove deklarace funkci ACPICA (jen ty, ktere system
//! pouziva), [`ns`] bezpecnejsi obal nad jmennym prostorem a objekty.
//! Rozlozeni struktur hlida `c/glue.c` (`_Static_assert`).

#![no_std]

extern crate alloc;

pub use cstd;

pub mod ffi;
pub mod ns;

pub use ffi::{Handle, Status};

/// Kod chyby ACPICA jako text (pro log).
pub fn status_name(s: Status) -> &'static str {
    match s {
        ffi::AE_OK => "AE_OK",
        ffi::AE_ERROR => "AE_ERROR",
        ffi::AE_NO_MEMORY => "AE_NO_MEMORY",
        ffi::AE_NOT_FOUND => "AE_NOT_FOUND",
        ffi::AE_NOT_EXIST => "AE_NOT_EXIST",
        ffi::AE_ALREADY_EXISTS => "AE_ALREADY_EXISTS",
        ffi::AE_NOT_IMPLEMENTED => "AE_NOT_IMPLEMENTED",
        ffi::AE_SUPPORT => "AE_SUPPORT",
        ffi::AE_LIMIT => "AE_LIMIT",
        ffi::AE_TIME => "AE_TIME",
        ffi::AE_NOT_ACQUIRED => "AE_NOT_ACQUIRED",
        ffi::AE_NO_HARDWARE_RESPONSE => "AE_NO_HARDWARE_RESPONSE",
        ffi::AE_ACCESS => "AE_ACCESS",
        ffi::AE_BAD_PARAMETER => "AE_BAD_PARAMETER",
        ffi::AE_BAD_ADDRESS => "AE_BAD_ADDRESS",
        _ => "AE_?",
    }
}

/// Prevod chyby ACPICA na kod chyby systemu.
pub fn to_error(s: Status) -> abi::Error {
    use abi::Error;
    match s {
        ffi::AE_NO_MEMORY => Error::NoMemory,
        ffi::AE_NOT_FOUND | ffi::AE_NOT_EXIST => Error::NotFound,
        ffi::AE_ALREADY_EXISTS => Error::AlreadyExists,
        ffi::AE_NOT_IMPLEMENTED | ffi::AE_SUPPORT => Error::NotSupported,
        ffi::AE_TIME => Error::TimedOut,
        ffi::AE_BAD_PARAMETER | ffi::AE_BAD_ADDRESS => Error::InvalidArgs,
        ffi::AE_ACCESS => Error::AccessDenied,
        _ => Error::IoError,
    }
}
