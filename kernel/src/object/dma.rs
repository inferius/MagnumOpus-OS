//! DMA po kusech: mapovani pripnute pameti do domeny zarizeni (`DMA_MAP`).
//!
//! - [`DeviceDomain`]: domena IOMMU jednoho zarizeni a jeji alokator IOVA
//!   (`libs/iova`). Okno IOVA lezi nad koncem RAM (`phys::ram_end`, aspon
//!   od 4 GiB) az po sirku prekladu domeny, takze se nikdy neprekryje
//!   s identickym mapovanim souvisle pameti pro DMA (adresa = fyzicka).
//!   Jedna domena na zarizeni (registr se slabymi odkazy): vsechny
//!   prostredky zarizeni (i po restartu ovladace) sdileji alokator.
//! - [`DmaMap`]: objekt jadra za handlem mapovani. Drzi objekt pameti
//!   (stranky zustavaji), rozsah IOVA a useky pro zarizeni. Zanik
//!   (zavreni handlu) = odmapovani se synchronnim vyprazdnenim IOTLB
//!   a az potom uvolneni rozsahu IOVA a odkazu na pamet. Po konci procesu
//!   ovladace jde mapovani do karanteny DMA (`quarantine.rs`): nejdriv se
//!   zastavi zarizeni, pak se odmapuje.
//! - Bez IOMMU jsou useky fyzicke adresy, smer nikdo nevynuti a jadro to
//!   zaloguje jednou na prostredek.
//!
//! Popis: `docs/cs/dma.md`.

use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use abi::dma::{Segment, merge_pages};
use iova::{IovaError, IovaSpace};

use super::job::{Charge, Job, Pool, object_cost};
use super::quarantine::{DeviceId, fmt_device};
use super::resource::Resource;
use super::vmo::MemoryObject;
use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::arch;
use crate::arch::iommu::{DmaAccess, Domain, IommuDomain};
use crate::log;
use crate::sync::SpinLock;

const PAGE: u64 = crate::arch::paging::PAGE_SIZE;

/// Okno IOVA zacina nejdrive tady (zarizeni s 32bitovou adresou DMA
/// nevidi nic z okna; pro ne zustava souvisla pamet pro DMA).
const IOVA_WINDOW_MIN: u64 = 1 << 32;
/// Zacatek okna se zaokrouhli nahoru na 1 GiB nad konec RAM.
const IOVA_WINDOW_ALIGN: u64 = 1 << 30;

/// Domena IOMMU zarizeni s alokatorem IOVA.
pub struct DeviceDomain {
    device: DeviceId,
    hw: Domain,
    iova: SpinLock<IovaSpace>,
}

/// Domeny podle zarizeni (slabe odkazy; domena zanikne s poslednim
/// prostredkem a mapovanim zarizeni).
static DOMAINS: SpinLock<BTreeMap<DeviceId, Weak<DeviceDomain>>> = SpinLock::new(BTreeMap::new());

impl DeviceDomain {
    /// Domena zarizeni: existujici, nebo nova. `Ok(None)` = zarizeni
    /// nepokryva zadna IOMMU.
    pub fn for_device(device: DeviceId) -> Result<Option<Arc<Self>>, Error> {
        let mut domains = DOMAINS.lock();
        if let Some(d) = domains.get(&device).and_then(Weak::upgrade) {
            return Ok(Some(d));
        }
        let Some(hw) = arch::iommu::domain(device.0, device.1, device.2)? else {
            return Ok(None);
        };
        let (base, end) = window(crate::mm::phys::ram_end(), hw.iova_bits());
        if base >= end {
            log::warn!(
                "DMA: zarizeni {} nema okno IOVA (domena preklada jen {} bitu), DMA_MAP selze",
                fmt_device(device),
                hw.iova_bits()
            );
        }
        let d = Arc::new(Self {
            device,
            hw,
            iova: SpinLock::new(IovaSpace::new(base, end)),
        });
        domains.retain(|_, w| w.strong_count() > 0);
        domains.insert(device, Arc::downgrade(&d));
        Ok(Some(d))
    }

    /// Domena IOMMU (pro identicke mapovani souvisle pameti).
    pub fn hw(&self) -> &Domain {
        &self.hw
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// Okno IOVA `[zacatek, konec)` (testy, diagnostika).
    pub fn window(&self) -> (u64, u64) {
        let s = self.iova.lock();
        (s.base(), s.end())
    }
}

/// Okno IOVA nad koncem RAM: `[max(zaokrouhleny konec RAM, 4 GiB),
/// 1 << bits)`.
fn window(ram_end: u64, bits: u32) -> (u64, u64) {
    let base = ram_end
        .checked_next_multiple_of(IOVA_WINDOW_ALIGN)
        .unwrap_or(u64::MAX)
        .max(IOVA_WINDOW_MIN);
    let end = 1u64.checked_shl(bits).unwrap_or(u64::MAX);
    (base, end)
}

/// Kam mapovani vede.
enum Target {
    /// Rozsah IOVA v domene zarizeni.
    Iommu { domain: Arc<DeviceDomain>, iova: u64 },
    /// Bez IOMMU: zarizeni pouziva fyzicke adresy.
    Direct,
}

/// Mapovani casti pripnuteho objektu pameti pro zarizeni.
pub struct DmaMap {
    core: ObjectCore,
    vmo: Arc<MemoryObject>,
    len: u64,
    /// Fyzicke stranky mapovani v poradi (DMA_SYNC, useky bez IOMMU).
    frames: Vec<u64>,
    segments: Vec<Segment>,
    target: Target,
    /// Zarizeni mapovani (karantena DMA).
    device: DeviceId,
    /// Prostredek, z jehoz kvoty DMA je mapovani strzene (vrati se v Drop).
    resource: Arc<Resource>,
    _charge: Charge,
}

impl DmaMap {
    /// Namapuje `len` bajtu objektu `vmo` od `offset` (obe zarovnane na
    /// stranky) pro zarizeni prostredku `resource`. Prava handlu overil
    /// volajici; tady se kontroluje pripnuti, meze a kvoty.
    pub fn new(
        resource: &Arc<Resource>,
        vmo: Arc<MemoryObject>,
        offset: u64,
        len: u64,
        access: DmaAccess,
        iova_hint: u64,
        job: &Arc<Job>,
    ) -> Result<Arc<Self>, Error> {
        let device = resource.device().ok_or(Error::AccessDenied)?;
        if !vmo.is_pinned() {
            return Err(Error::BadState);
        }
        let end = offset.checked_add(len).ok_or(Error::InvalidArgs)?;
        if len == 0
            || len > abi::dma::MAP_MAX
            || !offset.is_multiple_of(PAGE)
            || !len.is_multiple_of(PAGE)
            || end > vmo.size()
            || !(access.read || access.write)
            || !iova_hint.is_multiple_of(PAGE)
        {
            return Err(Error::InvalidArgs);
        }
        let domain = resource.device_domain().cloned();
        if domain.is_none() && iova_hint != 0 {
            return Err(Error::NotSupported);
        }
        let pages = len / PAGE;
        let charge = job.charge(Pool::Kernel, object_cost::<Self>() + pages * 8 + 16)?;
        let mut frames = Vec::new();
        frames.try_reserve_exact(pages as usize).map_err(|_| Error::NoMemory)?;
        for i in 0..pages {
            // Pripnuty objekt ma vsechny stranky (vznikly pri vytvoreni).
            let frame = vmo.page(offset + i * PAGE).ok_or(Error::BadState)?;
            frames.push(frame.as_u64());
        }
        let runs: Vec<Segment> = merge_pages(frames.iter().copied(), PAGE).collect();
        resource.charge_dma(len)?;
        let target = match &domain {
            Some(d) => match map_into(d, &runs, len, access, iova_hint) {
                Ok(iova) => Target::Iommu {
                    domain: d.clone(),
                    iova,
                },
                Err(e) => {
                    resource.refund_dma(len);
                    return Err(e);
                }
            },
            None => {
                resource.warn_unprotected_once(device);
                Target::Direct
            }
        };
        let segments = match &target {
            Target::Iommu { iova, .. } => alloc::vec![Segment { addr: *iova, len }],
            Target::Direct => runs,
        };
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            vmo,
            len,
            frames,
            segments,
            target,
            device,
            resource: resource.clone(),
            _charge: charge,
        }))
    }

    /// Useky z pohledu zarizeni (s IOMMU jeden).
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// Adresa zacatku pro zarizeni (IOVA, bez IOMMU prvni fyzicky usek).
    pub fn device_addr(&self) -> u64 {
        self.segments[0].addr
    }

    pub fn device(&self) -> DeviceId {
        self.device
    }

    /// Chrani mapovani IOMMU?
    pub fn is_protected(&self) -> bool {
        matches!(self.target, Target::Iommu { .. })
    }

    /// Objekt pameti pod mapovanim.
    pub fn memory(&self) -> &Arc<MemoryObject> {
        &self.vmo
    }

    /// Udrzba cache rozsahu `offset..+len` mapovani (`abi::dma::sync`).
    pub fn sync(&self, offset: u64, len: u64, direction: u64) -> Result<(), Error> {
        let end = offset.checked_add(len).ok_or(Error::InvalidArgs)?;
        if end > self.len {
            return Err(Error::InvalidArgs);
        }
        let op: fn(u64, u64) = match direction {
            abi::dma::sync::FOR_DEVICE => arch::dma_sync_for_device,
            abi::dma::sync::FOR_CPU => arch::dma_sync_for_cpu,
            _ => return Err(Error::InvalidArgs),
        };
        let mut at = offset;
        while at < end {
            let in_page = at % PAGE;
            let n = (PAGE - in_page).min(end - at);
            op(self.frames[(at / PAGE) as usize] + in_page, n);
            at += n;
        }
        if direction == abi::dma::sync::FOR_CPU {
            // Zarizeni dokoncilo praci: pripadne zablokovane pristupy
            // nahlasit hned (SMMUv3 zatim nema preruseni udalosti).
            arch::iommu::poll_faults();
        }
        Ok(())
    }
}

/// Vybere rozsah IOVA (nebo overi napovedu) a namapuje do nej fyzicke
/// useky za sebou. Pri chybe nic nezustane namapovane ani obsazene.
fn map_into(d: &DeviceDomain, runs: &[Segment], len: u64, access: DmaAccess, hint: u64) -> Result<u64, Error> {
    let iova = {
        let mut space = d.iova.lock();
        let r = if hint != 0 {
            space.reserve(hint, len).map(|()| hint)
        } else {
            space.alloc(len, PAGE)
        };
        r.map_err(|e| match e {
            IovaError::Invalid => Error::InvalidArgs,
            IovaError::Overlap => Error::AlreadyExists,
            IovaError::Full => Error::NoResources,
        })?
    };
    let mut at = iova;
    for run in runs {
        if let Err(e) = d.hw.map(at, run.addr, run.len / PAGE, access) {
            if at > iova {
                d.hw.unmap(iova, (at - iova) / PAGE);
            }
            d.iova.lock().free(iova);
            return Err(e);
        }
        at += run.len;
    }
    Ok(iova)
}

impl Drop for DmaMap {
    fn drop(&mut self) {
        // Zarizeni nejdriv ztrati pristup (synchronni vyprazdneni IOTLB),
        // teprve pak se rozsah IOVA uvolni pro dalsi mapovani a odkaz na
        // pamet zanikne (pole se ruseji az po tomto `drop`).
        if let Target::Iommu { domain, iova } = &self.target {
            domain.hw.unmap(*iova, self.len / PAGE);
            domain.iova.lock().free(*iova);
        }
        self.resource.refund_dma(self.len);
    }
}

impl KernelObject for DmaMap {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::DmaMap
    }

    fn default_rights(&self) -> Rights {
        // Bez TRANSFER: mapovani patri procesu ovladace, aby ho karantena
        // DMA pri jeho konci nasla (zarizeni nejdriv zastavit, pak odmapovat).
        Rights::READ | Rights::DUPLICATE | Rights::WAIT
    }
}

/// Okno IOVA pro testy jadra (`tests/dma.rs`).
pub fn window_for(ram_end: u64, bits: u32) -> (u64, u64) {
    window(ram_end, bits)
}
