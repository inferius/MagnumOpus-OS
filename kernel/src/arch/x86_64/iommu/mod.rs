//! IOMMU: preklad DMA (Intel VT-d nebo AMD-Vi).
//!
//! Bez IOMMU muze zarizeni pres DMA zapsat kamkoli do fyzicke pameti - chyba
//! ovladace (spatna adresa v deskriptoru) pak muze prepsat jadro. S IOMMU
//! prochazi kazdy pristup zarizeni do pameti prekladem: zarizeni (podle
//! adresy na sbernici PCI) ma vlastni **domenu** se strankovymi tabulkami
//! a smi jen tam, kam ho jadro pusti. Pristup jinam IOMMU zablokuje a nahlasi
//! (preruseni -> zaznam v logu jadra).
//!
//! Pravidla (stejna pro oba backendy):
//! - po zapnuti nema zadne zarizeni domenu: veskere DMA je zakazane,
//! - domenu zarizeni dostane az omezeny Resource s identitou zarizeni
//!   (`RESOURCE_CREATE` od spravce zarizeni),
//! - do domeny se mapuje jen DMA pamet vytvorena pres tento Resource
//!   (`VMO_CREATE_CONTIGUOUS`), identicky (adresa pro zarizeni = fyzicka
//!   adresa) - ovladace se nemeni,
//! - pri zruseni objektu pameti se stranky z domeny odmapuji drive, nez se
//!   vrati alokatoru.
//!
//! Backendy: `vtd` (tabulka ACPI DMAR), `amdvi` (tabulka ACPI IVRS).

mod amdvi;
mod vtd;

use alloc::sync::Arc;

use super::acpi::{AmdIommuUnit, IommuUnit};
use crate::acpi::PciEcam;
use crate::error::Error;
use crate::log;

/// Vektor preruseni pro hlaseni chyb DMA.
pub use super::apic::VECTOR_IOMMU_FAULT;

/// Domena prekladu DMA jednoho zarizeni (levne klonovatelny odkaz).
#[derive(Clone)]
pub enum Domain {
    Vtd(Arc<vtd::Domain>),
    Amd(Arc<amdvi::Domain>),
}

impl crate::arch::api::IommuDomain for Domain {
    /// Povoli zarizeni cteni i zapis `pages` stranek od `phys` (identicky).
    fn map(&self, phys: u64, pages: u64) -> Result<(), Error> {
        match self {
            Domain::Vtd(d) => d.map(phys, pages),
            Domain::Amd(d) => d.map(phys, pages),
        }
    }

    /// Odebere zarizeni pristup ke strankam (pred vracenim pameti).
    fn unmap(&self, phys: u64, pages: u64) {
        match self {
            Domain::Vtd(d) => d.unmap(phys, pages),
            Domain::Amd(d) => d.unmap(phys, pages),
        }
    }
}

/// Zapne vsechny IOMMU, ktere platforma ma. Bez nich zaloguje varovani:
/// DMA ovladacu pak neni omezene.
///
/// # Safety
///
/// Volat jednou pri startu, pred spustenim ovladacu.
pub unsafe fn init(intel: &[IommuUnit], amd: &[AmdIommuUnit], ecam: &[PciEcam]) {
    // SAFETY: predano volajicim.
    unsafe {
        vtd::init(intel);
        amdvi::init(amd, ecam);
    }
    if !active() {
        log::warn!(
            "IOMMU neni: zarizeni smi pres DMA zapisovat kamkoli do pameti (chyba ovladace muze poskodit system)"
        );
    }
}

/// Je nejaka IOMMU aktivni?
pub fn active() -> bool {
    vtd::active() || amdvi::active()
}

/// Domena zarizeni (segment, sbernice, zarizeni << 3 | funkce): existujici,
/// nebo nova. `Ok(None)` = zadna IOMMU zarizeni nepokryva (DMA neomezene).
pub fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Domain>, Error> {
    if let Some(d) = vtd::domain(segment, bus, devfn)? {
        return Ok(Some(Domain::Vtd(d)));
    }
    Ok(amdvi::domain(segment, bus, devfn)?.map(Domain::Amd))
}

/// Obsluha preruseni chyb (oba backendy).
pub fn on_fault_interrupt() {
    vtd::on_fault_interrupt();
    amdvi::on_fault_interrupt();
}
