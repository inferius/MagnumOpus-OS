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
    /// Rodic omezeneho prostredku: kvoty se pocitaji v celem retezci, takze
    /// potomci dohromady nikdy neprekroci limit rodice.
    parent: Option<Arc<Resource>>,
}

/// Nejvic rozsahu jednoho omezeneho prostredku.
pub const MAX_RANGES: usize = 16;

const PAGE: u64 = abi::vm::PAGE_SIZE;

fn contains(ranges: &[(u64, u64)], start: u64, len: u64) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    ranges
        .iter()
        .any(|&(s, l)| start >= s && end <= s.saturating_add(l))
}

impl Limits {
    fn take_irq(&self) -> bool {
        self.irqs
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.irq_max).then_some(n + 1)
            })
            .is_ok()
    }

    fn take_dma(&self, bytes: u64) -> bool {
        self.dma_used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |u| {
                u.checked_add(bytes).filter(|&n| n <= self.dma_max)
            })
            .is_ok()
    }
}

impl Resource {
    pub fn new_root() -> Arc<Self> {
        Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            limits: None,
            parent: None,
        })
    }

    /// Uzsi prostredek: rozsahy (zarovnane na stranky) musi lezet
    /// v rozsazich rodice, limity nesmi prekrocit limity rodice. Kvoty
    /// potomka se odecitaji i od rodice.
    pub fn restrict(
        self: &Arc<Self>,
        ranges: Vec<(u64, u64)>,
        irq_max: u32,
        dma_max: u64,
    ) -> Result<Arc<Self>, Error> {
        let bad = |&(s, l): &(u64, u64)| {
            l == 0
                || !s.is_multiple_of(PAGE)
                || !l.is_multiple_of(PAGE)
                || s.checked_add(l).is_none()
        };
        if ranges.len() > MAX_RANGES || ranges.iter().any(bad) {
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
            parent: Some(self.clone()),
        }))
    }

    /// Smi prostredek mapovat fyzicky rozsah? Kontroluje cele stranky
    /// (mapuje se po strankach, nesmi se pridat cast sousedniho zarizeni).
    pub fn allows_range(&self, start: u64, len: u64) -> bool {
        let Some(end) = start
            .checked_add(len)
            .and_then(|e| e.checked_next_multiple_of(PAGE))
        else {
            return false;
        };
        let first = start - start % PAGE;
        self.limits
            .as_ref()
            .is_none_or(|l| contains(&l.ranges, first, end - first))
    }

    /// Retezec omezenych prostredku od tohoto ke koreni.
    fn chain(&self) -> impl Iterator<Item = &Limits> {
        core::iter::successors(Some(self), |r| r.parent.as_deref())
            .filter_map(|r| r.limits.as_ref())
    }

    /// Zapocita jedno preruseni v celem retezci (limit plati po celou dobu
    /// zivota prostredku; neuspesny pokus nic nespotrebuje).
    pub fn charge_irq(&self) -> Result<(), Error> {
        let mut taken = 0;
        for l in self.chain() {
            if !l.take_irq() {
                for l in self.chain().take(taken) {
                    l.irqs.fetch_sub(1, Ordering::AcqRel);
                }
                return Err(Error::NoResources);
            }
            taken += 1;
        }
        Ok(())
    }

    /// Vrati zapocitane preruseni (vytvoreni objektu selhalo).
    pub fn refund_irq(&self) {
        for l in self.chain() {
            l.irqs.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Zapocita DMA pamet v celem retezci.
    pub fn charge_dma(&self, bytes: u64) -> Result<(), Error> {
        let mut taken = 0;
        for l in self.chain() {
            if !l.take_dma(bytes) {
                for l in self.chain().take(taken) {
                    l.dma_used.fetch_sub(bytes, Ordering::AcqRel);
                }
                return Err(Error::NoResources);
            }
            taken += 1;
        }
        Ok(())
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
