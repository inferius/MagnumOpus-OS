//! Jedno preruseni zarizeni pres MSI nebo MSI-X (podle toho, co funkce
//! umi). Pro ovladace, kterym staci jeden vektor pro vsechny priciny
//! (sitove karty s jednou frontou prijmu a odesilani).

use libos::hw::{Interrupt, Resource};
use libos::Result;

use crate::msi::Msi;
use crate::msix::MsixTable;
use crate::{Address, Ecam};

/// Jakym mechanismem preruseni chodi.
pub enum Kind {
    Msi(Msi),
    /// Tabulka musi zit, dokud ovladac bezi (namapovane okno).
    Msix(MsixTable),
}

/// Ktery mechanismus zkusit prvni.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefer {
    Msi,
    Msix,
}

/// Preruseni s nastavenym mechanismem.
pub struct DeviceIrq {
    pub irq: Interrupt,
    pub kind: Kind,
}

impl DeviceIrq {
    /// Vytvori preruseni a nastavi ho do zarizeni: nejdriv `prefer`, kdyz
    /// ho funkce nema, druhy mechanismus. Bez MSI i MSI-X `NotSupported`
    /// (stare linky INTx ovladace v userspace nepouzivaji).
    pub fn attach(ecam: &Ecam, a: Address, resource: &Resource, prefer: Prefer) -> Result<Self> {
        let irq = resource.interrupt()?;
        let msi = |irq: &Interrupt| {
            Msi::find(ecam, a).map(|m| {
                m.enable(ecam, irq.msi());
                Kind::Msi(m)
            })
        };
        let msix = |irq: &Interrupt| {
            MsixTable::open(ecam, a, resource).and_then(|t| {
                t.set(0, irq.msi())?;
                t.enable(ecam);
                Ok(Kind::Msix(t))
            })
        };
        let kind = match prefer {
            Prefer::Msi => msi(&irq).or_else(|_| msix(&irq)),
            Prefer::Msix => msix(&irq).or_else(|_| msi(&irq)),
        }?;
        Ok(Self { irq, kind })
    }

    /// Nazev mechanismu pro hlaseni.
    pub fn name(&self) -> &'static str {
        match self.kind {
            Kind::Msi(_) => "MSI",
            Kind::Msix(_) => "MSI-X",
        }
    }
}
