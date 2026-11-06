//! Resource: opravneni k primemu pristupu k hardwaru.
//!
//! Kdo ma handle na Resource s pravem MANAGE, smi vytvaret fyzicke objekty
//! pameti (registry zarizeni), souvislou pamet pro DMA s fyzickou adresou
//! a objekty preruseni. Jadro vytvori jediny korenovy Resource (bez omezeni)
//! a da ho procesu init; ten ho preda spravci zarizeni.
//!
//! Spravce zarizeni pak kazdemu ovladaci vytvori **omezeny** Resource
//! (`RESOURCE_CREATE`): jen fyzicke rozsahy jeho zarizeni (BARy, stranka
//! konfiguracniho prostoru), nejvys N preruseni a nejvys M bajtu DMA pameti.
//! Chyba v jednom ovladaci tak nemuze prepsat registry jineho zarizeni.
//!
//! Bez IOMMU muze zarizeni pres DMA zapsat kamkoli do pameti - ovladac, ktery
//! zarizeni naprogramuje spatne, porad muze poskodit system. TODO: IOMMU
//! (VT-d / AMD-Vi) a domena DMA na zarizeni.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use super::{KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::error::Error;

/// Omezeni ne-korenoveho prostredku.
struct Limits {
    /// Povolene fyzicke rozsahy (zacatek, delka).
    ranges: Vec<(u64, u64)>,
    irq_max: u32,
    irqs: AtomicU32,
    dma_max: u64,
    dma_used: AtomicU64,
}

pub struct Resource {
    core: ObjectCore,
    /// `None` = korenovy prostredek bez omezeni.
    limits: Option<Limits>,
}

/// Nejvic rozsahu jednoho omezeneho prostredku.
pub const MAX_RANGES: usize = 16;

fn contains(ranges: &[(u64, u64)], start: u64, len: u64) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    ranges
        .iter()
        .any(|&(s, l)| start >= s && end <= s.saturating_add(l))
}

impl Resource {
    pub fn new_root() -> Arc<Self> {
        Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            limits: None,
        })
    }

    /// Uzsi prostredek: rozsahy musi lezet v rozsazich rodice, limity
    /// nesmi prekrocit limity rodice.
    pub fn restrict(
        &self,
        ranges: Vec<(u64, u64)>,
        irq_max: u32,
        dma_max: u64,
    ) -> Result<Arc<Self>, Error> {
        if ranges.len() > MAX_RANGES
            || ranges
                .iter()
                .any(|&(s, l)| l == 0 || s.checked_add(l).is_none())
        {
            return Err(Error::InvalidArgs);
        }
        if let Some(p) = &self.limits {
            let inside = ranges.iter().all(|&(s, l)| contains(&p.ranges, s, l));
            if !inside || irq_max > p.irq_max || dma_max > p.dma_max {
                return Err(Error::AccessDenied);
            }
        }
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            limits: Some(Limits {
                ranges,
                irq_max,
                irqs: AtomicU32::new(0),
                dma_max,
                dma_used: AtomicU64::new(0),
            }),
        }))
    }

    /// Smi prostredek mapovat fyzicky rozsah?
    pub fn allows_range(&self, start: u64, len: u64) -> bool {
        self.limits
            .as_ref()
            .is_none_or(|l| contains(&l.ranges, start, len))
    }

    /// Zapocita jedno preruseni (limit plati po celou dobu zivota prostredku).
    pub fn charge_irq(&self) -> Result<(), Error> {
        let Some(l) = &self.limits else { return Ok(()) };
        l.irqs
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < l.irq_max).then_some(n + 1)
            })
            .map(|_| ())
            .map_err(|_| Error::NoResources)
    }

    /// Zapocita DMA pamet (limit plati po celou dobu zivota prostredku).
    pub fn charge_dma(&self, bytes: u64) -> Result<(), Error> {
        let Some(l) = &self.limits else { return Ok(()) };
        l.dma_used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |u| {
                u.checked_add(bytes).filter(|&n| n <= l.dma_max)
            })
            .map(|_| ())
            .map_err(|_| Error::NoResources)
    }
}

impl KernelObject for Resource {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Resource
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::MANAGE
    }
}
