//! Objekt pameti (memory object): stranky fyzicke pameti, ktere lze cist,
//! zapisovat a mapovat do adresnich prostoru procesu.
//!
//! Druhy objektu:
//! - bezny: stranky RAM alokovane a vynulovane hned pri vytvoreni,
//! - souvisly: fyzicky souvisla RAM pro DMA (ovladace zjisti fyzickou adresu),
//! - fyzicky: okno na fyzicky rozsah mimo RAM (registry zarizeni, BAR);
//!   stranky nepatri alokatoru. Typ cache (bez cache, WC, s cache) urci
//!   drzitel prostredku pri vytvoreni a kazde mapovani ho zdedi. Drzitel
//!   prostredku ho muze odvolat (`revoke`): mapovani ve vsech procesech
//!   zmizi a dalsi mapovani selze.
//!
//! Objekt nemeni velikost. TODO: lazy alokace pri prvnim pristupu,
//! copy-on-write klony.
//!
//! Stranky (bezne i souvisle) se strhavaji z rozpoctu uzivatelske pameti
//! jobu, seznam stranek a objekt samotny z rozpoctu jadra - pred alokaci,
//! takze prilis velky objekt selze hned (`NO_RESOURCES`). Seznam stranek
//! je dvouurovnove pole (bloky po 64 KiB = 32 MiB pameti objektu), takze
//! velikost objektu neomezuje nejvetsi souvisly blok heapu.

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use super::job::{Charge, Job, Pool, object_cost};
use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::arch;
use crate::arch::iommu::IommuDomain;
use crate::mm::chunked::ChunkedVec;
use crate::mm::{PhysAddr, phys};
use crate::object::process::Process;
use crate::sync::SpinLock;

const PAGE: u64 = crate::arch::paging::PAGE_SIZE;

/// Blok seznamu stranek (64 KiB = 8192 stranek).
const PAGE_LIST_CHUNK: usize = 64 * 1024;
type PageList = ChunkedVec<PhysAddr, PAGE_LIST_CHUNK>;

/// Strhne rozpocty za objekt s `count` strankami, z toho `user_bytes`
/// uzivatelske pameti.
fn charge(job: Option<&Arc<Job>>, count: u64, user_bytes: u64) -> Result<[Charge; 2], Error> {
    let Some(job) = job else {
        return Ok([Charge::none(), Charge::none()]);
    };
    let kernel = object_cost::<MemoryObject>() + count * core::mem::size_of::<PhysAddr>() as u64;
    let k = job.charge(Pool::Kernel, kernel)?;
    let u = job.charge(Pool::User, user_bytes)?;
    Ok([k, u])
}

fn page_list(count: u64) -> Result<PageList, Error> {
    let mut pages = PageList::new();
    pages.try_reserve(usize::try_from(count).map_err(|_| Error::InvalidArgs)?)?;
    Ok(pages)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Jednotlive stranky z alokatoru.
    Pages,
    /// Jeden souvisly blok z alokatoru (2^order stranek).
    Contiguous { order: usize },
    /// Fyzicky rozsah mimo RAM (registry zarizeni, BAR) s typem cache
    /// (`abi::vm::cache`).
    Physical { cache: u64 },
}

/// Odvolani fyzickeho objektu: priznak a procesy, ktere ho mapuji.
struct Revocation {
    revoked: AtomicBool,
    /// Procesy s mapovanim objektu (slabe odkazy, mrtve se uklidi).
    mappers: SpinLock<Vec<Weak<Process>>>,
}

pub struct MemoryObject {
    core: ObjectCore,
    size: u64,
    pages: PageList,
    kind: Kind,
    /// Domena IOMMU, ve ktere je pamet namapovana pro DMA zarizeni
    /// (odmapuje se pred vracenim stranek alokatoru).
    dma: Option<crate::arch::iommu::Domain>,
    /// Pamet pro DMA zarizeni: do procesu se mapuje s `MapFlags::DMA`
    /// a jadro pri pristupu pres primou mapu udrzuje cache (na nekoherentni
    /// platforme; na koherentni jsou `arch::dma_sync_*` prazdne).
    for_dma: bool,
    /// Jen fyzicke objekty: odvolani (`revoke`).
    revocation: Option<Revocation>,
    /// Strzeni z rozpoctu jadra a uzivatelske pameti jobu.
    _charges: [Charge; 2],
}

impl MemoryObject {
    /// Novy objekt o velikosti `size` (zaokrouhleno nahoru na stranky),
    /// vyplneny nulami, strzeny z rozpoctu `job`.
    pub fn new(size: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        if size == 0 || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        let charges = charge(job, count, count * PAGE)?;
        let mut vmo = Self {
            core: ObjectCore::new(Signals::NONE),
            size: count * PAGE,
            pages: page_list(count)?,
            kind: Kind::Pages,
            dma: None,
            for_dma: false,
            revocation: None,
            _charges: charges,
        };
        for _ in 0..count {
            // Pri neuspechu se uz alokovane stranky uvolni v Drop.
            vmo.pages
                .push(phys::alloc_user_frame_zeroed().ok_or(Error::NoMemory)?)?;
        }
        Ok(Arc::new(vmo))
    }

    /// Fyzicky souvisly objekt pro DMA (max. 4 MiB), vyplneny nulami; s
    /// domenou IOMMU ho zarizeni smi cist i zapisovat (identicky).
    pub fn new_contiguous_dma(
        size: u64,
        domain: Option<crate::arch::iommu::Domain>,
        job: Option<&Arc<Job>>,
    ) -> Result<Arc<Self>, Error> {
        let vmo = Self::new_contiguous(size, job)?;
        let mut vmo = Arc::try_unwrap(vmo).map_err(|_| Error::BadState)?;
        vmo.for_dma = true;
        // Nuly zapsane pres primou mapu musi byt v pameti driv, nez je uvidi
        // zarizeni nebo mapovani bez cache v procesu.
        arch::dma_sync_for_device(vmo.pages[0].as_u64(), vmo.allocated_bytes());
        if let Some(domain) = domain {
            domain.map(vmo.pages[0].as_u64(), vmo.allocated_bytes() / PAGE)?;
            vmo.dma = Some(domain);
        }
        Ok(Arc::new(vmo))
    }

    /// Fyzicky souvisly objekt pro DMA (max. 4 MiB), vyplneny nulami.
    pub fn new_contiguous(size: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        if size == 0 || size > (PAGE << phys::MAX_ORDER) {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        let order = count.next_power_of_two().trailing_zeros() as usize;
        let charges = charge(job, count, PAGE << order)?;
        let mut pages = page_list(count)?;
        let base = phys::alloc_user_pages(order).ok_or(Error::NoMemory)?;
        // SAFETY: prave alokovany blok 2^order stranek v prime mape.
        unsafe {
            base.to_virt()
                .as_mut_ptr::<u8>()
                .write_bytes(0, (PAGE << order) as usize)
        };
        for i in 0..count {
            // Misto je rezervovane (`page_list`), push nealokuje.
            pages.push(PhysAddr::new(base.as_u64() + i * PAGE))?;
        }
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            size: count * PAGE,
            pages,
            kind: Kind::Contiguous { order },
            dma: None,
            for_dma: false,
            revocation: None,
            _charges: charges,
        }))
    }

    /// Okno na fyzicky rozsah mimo RAM s typem cache `cache`
    /// (`abi::vm::cache`). Volajici ruci (pres `platform::is_device_memory`),
    /// ze rozsah nelezi v RAM.
    pub fn new_physical(
        phys_start: u64,
        size: u64,
        cache: u64,
        job: Option<&Arc<Job>>,
    ) -> Result<Arc<Self>, Error> {
        if size == 0 || !phys_start.is_multiple_of(PAGE) || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        if cache > abi::vm::cache::CACHED {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        // Registry zarizeni nejsou RAM: jen seznam stranek z rozpoctu jadra.
        let charges = charge(job, count, 0)?;
        let mut pages = page_list(count)?;
        for i in 0..count {
            pages.push(PhysAddr::new(phys_start + i * PAGE))?;
        }
        Ok(Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            size: count * PAGE,
            pages,
            kind: Kind::Physical { cache },
            dma: None,
            for_dma: false,
            revocation: Some(Revocation {
                revoked: AtomicBool::new(false),
                mappers: SpinLock::new(Vec::new()),
            }),
            _charges: charges,
        }))
    }

    /// Kolik fyzicke pameti objekt skutecne drzi (souvisly objekt zabira
    /// cely blok 2^order stranek).
    pub fn allocated_bytes(&self) -> u64 {
        match self.kind {
            Kind::Contiguous { order } => PAGE << order,
            _ => self.size,
        }
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    /// Fyzicky rozsah mimo RAM (neni v prime mape, jadro do nej nepristupuje)?
    pub fn is_physical(&self) -> bool {
        matches!(self.kind, Kind::Physical { .. })
    }

    /// Priznaky typu cache pro mapovani (`READ` = zadne, bezna pamet s cache).
    pub fn cache_flags(&self) -> arch::paging::MapFlags {
        use arch::paging::MapFlags;
        match self.kind {
            Kind::Physical { cache: abi::vm::cache::UNCACHED } => MapFlags::UNCACHED,
            Kind::Physical { cache: abi::vm::cache::WRITE_COMBINING } => MapFlags::WRITE_COMBINING,
            _ => MapFlags::READ,
        }
    }

    /// Fyzicky rozsah objektu (jen fyzicke objekty, pro kontrolu prostredku).
    pub fn physical_range(&self) -> Option<(u64, u64)> {
        self.is_physical().then(|| (self.pages[0].as_u64(), self.size))
    }

    /// Je objekt odvolany?
    pub fn is_revoked(&self) -> bool {
        self.revocation.as_ref().is_some_and(|r| r.revoked.load(Ordering::Acquire))
    }

    /// Zaznamena proces, ktery objekt prave namapoval. Odvolany objekt =
    /// `AccessDenied` (volajici mapovani vrati). Priznak a seznam jsou pod
    /// jednim zamkem: mapovani soubezne s `revoke` bud revoke uvidi, nebo
    /// samo uvidi priznak.
    pub fn register_mapper(&self, p: &Arc<Process>) -> Result<(), Error> {
        let Some(r) = &self.revocation else { return Ok(()) };
        let mut m = r.mappers.lock();
        if r.revoked.load(Ordering::Acquire) {
            return Err(Error::AccessDenied);
        }
        if m.len() == m.capacity() {
            m.retain(|w| w.strong_count() > 0);
        }
        if !m.iter().any(|w| core::ptr::eq(w.as_ptr(), Arc::as_ptr(p))) {
            m.try_reserve(1).map_err(|_| Error::NoMemory)?;
            m.push(Arc::downgrade(p));
        }
        Ok(())
    }

    /// Odvola objekt: dalsi mapovani selze. Vraci procesy, ve kterych je
    /// objekt namapovany (volajici jim mapovani zrusi). Jen fyzicke objekty.
    pub fn revoke(&self) -> Result<Vec<Arc<Process>>, Error> {
        let r = self.revocation.as_ref().ok_or(Error::NotSupported)?;
        let mut m = r.mappers.lock();
        r.revoked.store(true, Ordering::Release);
        Ok(core::mem::take(&mut *m).into_iter().filter_map(|w| w.upgrade()).collect())
    }

    /// Pamet pro DMA (mapuje se s `MapFlags::DMA`)?
    pub fn is_dma(&self) -> bool {
        self.for_dma
    }

    /// Fyzicka stranka na offsetu `offset` (zarovnanem na stranku).
    pub fn page(&self, offset: u64) -> Option<PhysAddr> {
        self.pages.get((offset / PAGE) as usize).copied()
    }

    /// Registry zarizeni nejsou v prime mape a cteni by mohlo mit vedlejsi
    /// efekty: jde jen mapovat, ne cist a zapisovat pres jadro.
    fn check_access(&self) -> Result<(), Error> {
        if self.is_physical() {
            Err(Error::NotSupported)
        } else {
            Ok(())
        }
    }

    fn check_range(&self, offset: u64, len: u64) -> Result<(), Error> {
        match offset.checked_add(len) {
            Some(end) if end <= self.size => Ok(()),
            _ => Err(Error::OutOfRange),
        }
    }

    /// Projde rozsah po kouscich v ramci stranek: `f(ukazatel do prime mapy,
    /// offset v bufferu, delka)`.
    fn for_each_chunk(&self, offset: u64, len: usize, mut f: impl FnMut(*mut u8, usize, usize)) {
        let mut done = 0;
        while done < len {
            let pos = offset + done as u64;
            let in_page = (pos % PAGE) as usize;
            let n = (PAGE as usize - in_page).min(len - done);
            let page = self.pages[(pos / PAGE) as usize];
            let ptr = page.to_virt().as_mut_ptr::<u8>().wrapping_add(in_page);
            f(ptr, done, n);
            done += n;
        }
    }

    pub fn read(&self, offset: u64, buf: &mut [u8]) -> Result<(), Error> {
        self.check_access()?;
        self.check_range(offset, buf.len() as u64)?;
        // Data mohlo zapsat zarizeni nebo proces pres mapovani bez cache.
        self.dma_sync(offset, buf.len(), arch::dma_sync_for_cpu);
        self.for_each_chunk(offset, buf.len(), |ptr, at, n| {
            // SAFETY: `ptr..ptr+n` lezi v jedne strance tohoto objektu (prima
            // mapa); buffer je mimo pamet objektu. Soubezny zapis jineho
            // vlakna muze dat smichana data, ale ne nedefinovane chovani
            // jadra (kopirujeme bajty).
            unsafe { core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr().add(at), n) }
        });
        Ok(())
    }

    pub fn write(&self, offset: u64, data: &[u8]) -> Result<(), Error> {
        self.check_access()?;
        self.check_range(offset, data.len() as u64)?;
        self.for_each_chunk(offset, data.len(), |ptr, at, n| {
            // SAFETY: viz `read`.
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr().add(at), ptr, n) }
        });
        // Zarizeni a mapovani bez cache musi zapsana data videt.
        self.dma_sync(offset, data.len(), arch::dma_sync_for_device);
        Ok(())
    }

    /// U pameti pro DMA zavola `sync` na fyzicky rozsah (objekt je souvisly).
    fn dma_sync(&self, offset: u64, len: usize, sync: fn(u64, u64)) {
        if self.for_dma && len > 0 {
            sync(self.pages[0].as_u64() + offset, len as u64);
        }
    }
}

impl Drop for MemoryObject {
    fn drop(&mut self) {
        // Zarizeni nejdriv ztrati pristup (IOMMU), az pak se pamet vrati.
        if let Some(d) = self.dma.take() {
            d.unmap(self.pages[0].as_u64(), self.allocated_bytes() / PAGE);
        }
        // Pres mapovani bez cache se psalo mimo cache: jeji pripadne stare
        // radky (spekulativni cteni pres primou mapu) zahodit, aby dalsi
        // vlastnik stranek nevidel stara data.
        if self.for_dma {
            arch::dma_sync_for_cpu(self.pages[0].as_u64(), self.allocated_bytes());
        }
        // Objekt uz nikdo nema namapovany (mapovani drzi Arc), takze stranky
        // smi zpet do alokatoru. Fyzicke okno alokatoru nepatri.
        match self.kind {
            Kind::Pages => {
                for &page in self.pages.iter() {
                    phys::free_pages(page, 0);
                }
            }
            Kind::Contiguous { order } => phys::free_pages(self.pages[0], order),
            Kind::Physical { .. } => {}
        }
    }
}

impl KernelObject for MemoryObject {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::MemoryObject
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP | Rights::EXECUTE
    }
}
