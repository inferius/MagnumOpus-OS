//! Vrstva OS pro ACPICA (`AcpiOs*`) nad libos.
//!
//! ACPICA je knihovna v C; vse, co potrebuje od systemu, vola pres funkce
//! `AcpiOs*` (seznam v `acpiosxf.h`). Tady jsou v Rustu s ABI C:
//!
//! - [`mem`]: alokace, mapovani fyzicke pameti (tabulky, oblasti AML),
//!   cteni a zapis fyzicke pameti, RSDP,
//! - [`sync`]: spinlocky a semafory (ACPICA z nich sklada i mutexy),
//! - [`sched`]: odlozena prace (`AcpiOsExecute`), cekani, cas, ID vlakna,
//! - [`hw`]: I/O porty, konfigurace PCI, preruseni SCI,
//! - [`misc`]: prepisy tabulek a jmen (nepouzivame), signaly, start a konec.
//!
//! `AcpiOsPrintf`/`AcpiOsVprintf` (promenny pocet argumentu) a globalni
//! zamek jsou v C (`libs/acpica/c/glue.c`).
//!
//! Vsechny funkce jsou volane z kodu ACPICA; ukazatele od ni bereme jako
//! platne (kontrakt `acpiosxf.h`).

pub mod hw;
pub mod mem;
pub mod misc;
pub mod sched;
pub mod sync;

use abi::acpi::Info;
use libos::hw::Resource;

/// Co vrstva OS potrebuje od sluzby (nastavi `init` pred prvnim volanim
/// ACPICA).
pub struct Env {
    /// Prostredek s rozsahy pameti, porty a linkami preruseni (MANAGE).
    pub resource: Resource,
    /// Udaje od jadra: RSDP a oblasti pameti ACPI.
    pub info: Info,
}

static ENV: spin::Once<Env> = spin::Once::new();

pub fn init(env: Env) {
    ENV.call_once(|| env);
    sched::register_thread();
}

pub fn env() -> &'static Env {
    ENV.get().expect("acpid: vrstva OS neni inicializovana")
}

/// Typ `ACPI_STATUS` (u32).
pub type Status = acpica::Status;
pub use acpica::ffi::{AE_BAD_PARAMETER, AE_ERROR, AE_NO_MEMORY, AE_NOT_EXIST, AE_OK, AE_TIME};
