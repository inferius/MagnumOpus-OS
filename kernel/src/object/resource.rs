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
//! Pravo `SET_CLOCK` (nastaveni realneho casu, `CLOCK_SET`) ma jen korenovy
//! prostredek. init ho preda sluzbe casu (handle bez `MANAGE`) a spravci
//! zarizeni da koren bez `SET_CLOCK`.
//!
//! Krome pameti muze omezeny prostredek povolit I/O porty a stare linky
//! preruseni (ISA IRQ) - rozsahy oznacene horni dvojici bitu zacatku
//! (`abi::syscall::RANGE_IO_PORTS`, `RANGE_IRQ_LINES`). Potrebuji je
//! zarizeni na desce bez PCI (radic PS/2).
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
    /// Povolene I/O porty (zacatek, pocet).
    ports: Vec<(u64, u64)>,
    /// Povolene linky preruseni (bit = ISA IRQ).
    lines: u32,
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
    /// Zarizeni (segment, sbernice, zarizeni << 3 | funkce), kteremu
    /// prostredek patri, a jeho domena IOMMU (DMA pamet se mapuje do ni).
    device: Option<(u16, u8, u8)>,
    domain: Option<crate::arch::iommu::Domain>,
}

/// Nejvic rozsahu jednoho omezeneho prostredku.
pub const MAX_RANGES: usize = 16;

const PAGE: u64 = crate::arch::paging::PAGE_SIZE;

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
            device: None,
            domain: None,
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
        device: Option<(u16, u8, u8)>,
    ) -> Result<Arc<Self>, Error> {
        let bad = |&(s, l): &(u64, u64)| {
            l == 0
                || !s.is_multiple_of(PAGE)
                || !l.is_multiple_of(PAGE)
                || s.checked_add(l).is_none()
        };
        if ranges.len() > MAX_RANGES {
            return Err(Error::InvalidArgs);
        }
        let (mut memory, mut ports, mut lines) = (Vec::new(), Vec::new(), 0u32);
        for (start, len) in ranges {
            let base = start & !abi::syscall::RANGE_KIND_MASK;
            match start & abi::syscall::RANGE_KIND_MASK {
                0 => memory.push((start, len)),
                abi::syscall::RANGE_IO_PORTS => {
                    if len == 0 || base.checked_add(len).is_none_or(|e| e > 0x1_0000) {
                        return Err(Error::InvalidArgs);
                    }
                    ports.push((base, len));
                }
                abi::syscall::RANGE_IRQ_LINES => {
                    if len == 0 || base.checked_add(len).is_none_or(|e| e > 32) {
                        return Err(Error::InvalidArgs);
                    }
                    for line in base..base + len {
                        lines |= 1 << line;
                    }
                }
                _ => return Err(Error::InvalidArgs),
            }
        }
        if memory.iter().any(bad) {
            return Err(Error::InvalidArgs);
        }
        if let Some(p) = &self.limits {
            let inside = memory.iter().all(|&(s, l)| contains(&p.ranges, s, l))
                && ports.iter().all(|&(s, l)| contains(&p.ports, s, l))
                && lines & !p.lines == 0;
            if !inside || irq_max > p.irq_max || dma_max > p.dma_max {
                return Err(Error::AccessDenied);
            }
        }
        let ranges = memory;
        // Zarizeni: potomek zdedi zarizeni rodice; jine nesmi zvolit.
        let device = match (self.device, device) {
            (Some(p), Some(d)) if p != d => return Err(Error::AccessDenied),
            (Some(p), _) => Some(p),
            (None, d) => d,
        };
        let domain = match (&self.domain, device) {
            (Some(d), _) => Some(d.clone()),
            (None, Some((seg, bus, devfn))) => crate::arch::iommu::domain(seg, bus, devfn)?,
            (None, None) => None,
        };
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            limits: Some(Limits {
                ranges,
                ports,
                lines,
                irq_max,
                irqs: AtomicU32::new(0),
                dma_max,
                dma_used: AtomicU64::new(0),
            }),
            parent: Some(self.clone()),
            device,
            domain,
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

    /// Smi prostredek pristupovat k portum `port..port + width`?
    pub fn allows_ports(&self, port: u16, width: u8) -> bool {
        self.limits
            .as_ref()
            .is_none_or(|l| contains(&l.ports, port as u64, width as u64))
    }

    /// Smi prostredek vytvorit preruseni na lince `line` (ISA IRQ)?
    pub fn allows_line(&self, line: u32) -> bool {
        line < 32
            && self
                .limits
                .as_ref()
                .is_none_or(|l| l.lines & (1 << line) != 0)
    }

    /// Zarizeni, ke kteremu prostredek patri (segment, sbernice,
    /// zarizeni << 3 | funkce), pokud je omezeny na jedno zarizeni.
    pub fn device(&self) -> Option<(u16, u8, u8)> {
        self.device
    }

    /// Domena IOMMU zarizeni (DMA pamet z tohoto prostredku se mapuje do ni).
    pub fn domain(&self) -> Option<&crate::arch::iommu::Domain> {
        self.domain.as_ref()
    }

    /// Retezec omezenych prostredku od tohoto ke koreni.
    fn chain(&self) -> impl Iterator<Item = &Limits> {
        core::iter::successors(Some(self), |r| r.parent.as_deref())
            .filter_map(|r| r.limits.as_ref())
    }

    /// Zapocita jedno preruseni v celem retezci (limit plati po celou dobu
    /// zivota prostredku; neuspesny pokus nic nespotrebuje).
    pub fn charge_irq(&self) -> Result<(), Error> {
        for (taken, l) in self.chain().enumerate() {
            if !l.take_irq() {
                for l in self.chain().take(taken) {
                    l.irqs.fetch_sub(1, Ordering::AcqRel);
                }
                return Err(Error::NoResources);
            }
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
        for (taken, l) in self.chain().enumerate() {
            if !l.take_dma(bytes) {
                for l in self.chain().take(taken) {
                    l.dma_used.fetch_sub(bytes, Ordering::AcqRel);
                }
                return Err(Error::NoResources);
            }
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
        // Nastaveni hodin jen pro korenovy prostredek: omezene prostredky
        // ovladacu (`restrict`) ho nedostanou nikdy.
        if self.limits.is_none() {
            Rights::BASIC | Rights::MANAGE | Rights::SET_CLOCK
        } else {
            Rights::BASIC | Rights::MANAGE
        }
    }
}
