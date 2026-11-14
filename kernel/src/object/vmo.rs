//! Objekt pameti (memory object): stranky fyzicke pameti, ktere lze cist,
//! zapisovat a mapovat do adresnich prostoru procesu.
//!
//! Druhy objektu:
//! - bezny: stranky RAM na pozadani - pri vytvoreni se jen slibi (commit
//!   v `phys` a rozpocet jobu), ramec vznikne pri prvnim zapisu (vypadek,
//!   `VMO_WRITE`, kopirovani jadra). Cteni dosud nevznikle stranky vraci
//!   nuly; do procesu se pro cteni mapuje sdilena nulova stranka jen pro
//!   cteni (`mm::zero_frame`), kterou zapis nahradi vlastnim ramcem,
//! - souvisly: fyzicky souvisla RAM pro DMA (ovladace zjisti fyzickou adresu),
//! - fyzicky: okno na fyzicky rozsah mimo RAM (registry zarizeni, BAR,
//!   framebuffer z firmwaru); stranky nepatri alokatoru. Typ cache (bez
//!   cache, WC, s cache) urci drzitel prostredku pri vytvoreni (framebuffer
//!   = WC) a kazde mapovani ho zdedi. Drzitel prostredku ho muze odvolat
//!   (`revoke`): mapovani ve vsech procesech zmizi a dalsi mapovani selze.
//!
//! Kopie pri zapisu (`VMO_CLONE`, `cow.rs`): klon sdili ramce zdroje,
//! prvni zapis kterehokoli vlastnika si udela kopii.
//!
//! Objekt nemeni velikost. Nulova stranka: kdyz ji nektere mapovani
//! ukazuje (`zero_mapped`) a stranka pak vznikne jinou cestou, objekt ji
//! vsem mapperum odmapuje (`AddressSpace::drop_zero_ptes`), aby nikdo
//! necetl stare nuly misto zapsanych dat. Souvisla a fyzicka pamet je vzdy
//! cela pritomna (DMA ji potrebuje hned a na stalych adresach).
//!
//! Stranky (bezne i souvisle) se strhavaji z rozpoctu uzivatelske pameti
//! jobu, seznam stranek a objekt samotny z rozpoctu jadra - pred alokaci,
//! takze prilis velky objekt selze hned (`NO_RESOURCES`). Seznam stranek
//! je dvouurovnove pole (bloky po 64 KiB = 32 MiB pameti objektu), takze
//! velikost objektu neomezuje nejvetsi souvisly blok heapu.

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
/// Polozka = fyzicka adresa | `PRESENT` [| `PINNED`], 0 = stranka jeste
/// nevznikla.
type PageList = ChunkedVec<AtomicU64, PAGE_LIST_CHUNK>;
const PRESENT: u64 = 1;
/// Stranku pripnul `expose_phys` (adresu zna ovladac a muze ji dat
/// zarizeni); odepne se v Drop.
const PINNED: u64 = 2;
/// Ramec je (nebo byl) sdileny s klonem: zapis ho musi zkopirovat, pokud
/// ma jeste dalsi vlastniky (`phys::shares`). Polozka se SHARED je soucasti
/// `unbacked` (zapis muze potrebovat novy ramec).
const SHARED: u64 = 4;
const ENTRY_FLAGS: u64 = PAGE - 1;

mod cow;

fn present(phys: PhysAddr) -> AtomicU64 {
    AtomicU64::new(phys.as_u64() | PRESENT)
}

/// Strhne rozpocty za objekt s `count` strankami, z toho `user_bytes`
/// uzivatelske pameti.
/// `pinned_bytes` = pripnuta pamet (rozpocet `DMA`, navic k uzivatelske).
fn charge(job: Option<&Arc<Job>>, count: u64, user_bytes: u64, pinned_bytes: u64) -> Result<[Charge; 3], Error> {
    let Some(job) = job else {
        return Ok([Charge::none(), Charge::none(), Charge::none()]);
    };
    let kernel = object_cost::<MemoryObject>() + count * core::mem::size_of::<PhysAddr>() as u64;
    let k = job.charge(Pool::Kernel, kernel)?;
    let u = job.charge(Pool::User, user_bytes)?;
    let d = if pinned_bytes > 0 {
        job.charge(Pool::Dma, pinned_bytes)?
    } else {
        Charge::none()
    };
    Ok([k, u, d])
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


pub struct MemoryObject {
    core: ObjectCore,
    size: u64,
    pages: PageList,
    /// Slibene stranky, ktere jeste nemaji ramec (vrati se `phys` v Drop).
    unbacked: AtomicU64,
    /// Nektere mapovani ukazuje nulovou stranku misto dosud nevznikle.
    zero_mapped: AtomicBool,
    /// Procesy s mapovanim objektu (slabe odkazy, mrtve se uklidi): pro
    /// odvolani fyzickeho objektu a odmapovani nulove stranky.
    mappers: SpinLock<Vec<Weak<Process>>>,
    /// Serializuje zmeny sdilenych polozek (klonovani, kopie pri zapisu,
    /// pripnuti). Nikdy se nedrzi pri shootdownu ani se zamkem prostoru.
    cow_lock: SpinLock<()>,
    kind: Kind,
    /// Domena IOMMU, ve ktere je pamet namapovana pro DMA zarizeni
    /// (odmapuje se pred vracenim stranek alokatoru).
    dma: Option<crate::arch::iommu::Domain>,
    /// Zarizeni, pro ktere pamet pro DMA vznikla (identita z prostredku),
    /// a cislo procesu, ktery ji vytvoril (ovladac); karantena pri konci
    /// ovladace, `quarantine.rs`.
    device: Option<(super::quarantine::DeviceId, u64)>,
    /// Pamet pro DMA zarizeni: do procesu se mapuje s `MapFlags::DMA`
    /// a jadro pri pristupu pres primou mapu udrzuje cache (na nekoherentni
    /// platforme; na koherentni jsou `arch::dma_sync_*` prazdne).
    for_dma: bool,
    /// Zapeceteny objekt (`VMO_SEAL`): obsah se uz nikdy nezmeni.
    sealed: AtomicBool,
    /// Jen fyzicke objekty jdou odvolat (`revoke`).
    revocable: bool,
    revoked: AtomicBool,
    /// Vsechny stranky vznikly hned a jsou pripnute po celou dobu zivota
    /// (`VMO_PINNED`): zarizeni k nim smi pres `DMA_MAP`.
    pinned: bool,
    /// Strzeni z rozpoctu jadra, uzivatelske pameti a pripnute pameti jobu.
    _charges: [Charge; 3],
}

impl MemoryObject {
    fn with(size: u64, pages: PageList, kind: Kind, unbacked: u64, charges: [Charge; 3]) -> Self {
        Self {
            core: ObjectCore::new(Signals::NONE),
            size,
            pages,
            unbacked: AtomicU64::new(unbacked),
            zero_mapped: AtomicBool::new(false),
            mappers: SpinLock::new(Vec::new()),
            cow_lock: SpinLock::new(()),
            sealed: AtomicBool::new(false),
            kind,
            dma: None,
            device: None,
            for_dma: false,
            pinned: false,
            revocable: false,
            revoked: AtomicBool::new(false),
            _charges: charges,
        }
    }

    /// Novy objekt o velikosti `size` (zaokrouhleno nahoru na stranky),
    /// cteny jako nuly, strzeny z rozpoctu `job`. Cela velikost se slibi
    /// hned (rozpocet jobu i `phys::commit`), ramce vzniknou az pri zapisu.
    pub fn new(size: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        if size == 0 || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        // Nejdriv levne kontroly: seznam stranek je v halde jadra (8 B na
        // stranku) a obri pozadavek by ji jinak na chvili vycerpal (ostatni
        // procesy by mezitim dostaly NO_MEMORY), i kdyz slib stejne neprojde.
        let list_pages = (count * core::mem::size_of::<AtomicU64>() as u64).div_ceil(PAGE);
        if !phys::user_room(count.saturating_add(list_pages)) {
            return Err(Error::NoMemory);
        }
        let charges = charge(job, count, count * PAGE, 0)?;
        if !phys::commit(count) {
            return Err(Error::NoMemory);
        }
        let mut pages = match page_list(count) {
            Ok(p) => p,
            Err(e) => {
                phys::uncommit(count);
                return Err(e);
            }
        };
        for _ in 0..count {
            // Misto je rezervovane (`page_list`), push nealokuje.
            if let Err(e) = pages.push(AtomicU64::new(0)) {
                phys::uncommit(count);
                return Err(e);
            }
        }
        Ok(Arc::new(Self::with(count * PAGE, pages, Kind::Pages, count, charges)))
    }

    /// Pripnuty objekt (`VMO_PINNED`): vsechny stranky hned (nesouvisle,
    /// vynulovane) a pripnute po celou dobu zivota. Strhava se uzivatelska
    /// pamet i rozpocet `DMA` jobu; do domeny zarizeni se mapuje az pres
    /// `DMA_MAP` (`object/dma.rs`).
    pub fn new_pinned(size: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        if size == 0 || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        let list_pages = (count * core::mem::size_of::<AtomicU64>() as u64).div_ceil(PAGE);
        if !phys::user_room(count.saturating_add(list_pages)) {
            return Err(Error::NoMemory);
        }
        let charges = charge(job, count, count * PAGE, count * PAGE)?;
        let mut pages = page_list(count)?;
        let mut vmo = Self::with(count * PAGE, PageList::new(), Kind::Pages, 0, charges);
        vmo.pinned = true;
        for _ in 0..count {
            // Bez slibu: ramec vznika hned (alloc_user_pages respektuje
            // rezervu jadra i sliby ostatnich objektu). Pri chybe Drop
            // objektu vrati uz pridane stranky.
            let Some(frame) = phys::alloc_user_pages(0) else {
                vmo.pages = pages;
                return Err(Error::NoMemory);
            };
            // SAFETY: prave alokovana stranka v prime mape.
            unsafe { frame.to_virt().as_mut_ptr::<u8>().write_bytes(0, PAGE as usize) };
            phys::pin(frame);
            // Misto je rezervovane (`page_list`), push nealokuje.
            pages.push(AtomicU64::new(frame.as_u64() | PRESENT | PINNED))?;
        }
        vmo.pages = pages;
        // Nuly zapsane pres primou mapu musi byt v pameti driv, nez je uvidi
        // zarizeni (nekoherentni platforma).
        for p in vmo.pages.iter() {
            arch::dma_sync_for_device(p.load(Ordering::Relaxed) & !ENTRY_FLAGS, PAGE);
        }
        Ok(Arc::new(vmo))
    }

    /// Jsou vsechny stranky trvale pripnute (`VMO_PINNED` nebo souvisla
    /// pamet pro DMA)? Jen takovy objekt jde predat `DMA_MAP`.
    pub fn is_pinned(&self) -> bool {
        self.pinned || self.for_dma
    }

    /// Fyzicky souvisly objekt pro DMA (max. 4 MiB), vyplneny nulami; s
    /// domenou IOMMU ho zarizeni smi cist i zapisovat (identicky).
    pub fn new_contiguous_dma(
        size: u64,
        domain: Option<crate::arch::iommu::Domain>,
        device: Option<(super::quarantine::DeviceId, u64)>,
        job: Option<&Arc<Job>>,
    ) -> Result<Arc<Self>, Error> {
        let vmo = Self::contiguous(size, job)?;
        let mut vmo = Arc::try_unwrap(vmo).map_err(|_| Error::BadState)?;
        vmo.for_dma = true;
        vmo.device = device;
        // Zarizeni muze do stranek zapisovat po celou dobu zivota objektu:
        // pripnout (CoW ani pager je nesmi presunout).
        for p in vmo.pages.iter() {
            phys::pin(PhysAddr::new(p.load(Ordering::Relaxed) & !ENTRY_FLAGS));
        }
        // Nuly zapsane pres primou mapu musi byt v pameti driv, nez je uvidi
        // zarizeni nebo mapovani bez cache v procesu.
        arch::dma_sync_for_device(vmo.first_frame().as_u64(), vmo.allocated_bytes());
        if let Some(domain) = domain {
            let first = vmo.first_frame().as_u64();
            domain.map(first, first, vmo.allocated_bytes() / PAGE, crate::arch::iommu::DmaAccess::READ_WRITE)?;
            vmo.dma = Some(domain);
        }
        Ok(Arc::new(vmo))
    }

    /// Fyzicky souvisly blok (max. 4 MiB), vyplneny nulami; strhne i rozpocet
    /// pripnute pameti (`DMA`), objekt je pro DMA.
    fn contiguous(size: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        if size == 0 || size > (PAGE << phys::MAX_ORDER) {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        let order = count.next_power_of_two().trailing_zeros() as usize;
        let bytes = PAGE << order;
        let charges = charge(job, count, bytes, bytes)?;
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
            pages.push(present(PhysAddr::new(base.as_u64() + i * PAGE)))?;
        }
        Ok(Arc::new(Self::with(count * PAGE, pages, Kind::Contiguous { order }, 0, charges)))
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
        Self::physical(phys_start, size, job, cache)
    }

    /// Okno na framebuffer z firmwaru: fyzicky objekt s typem cache
    /// `WRITE_COMBINING`. Volajici ruci, ze rozsah nepatri alokatoru RAM.
    pub fn new_framebuffer(phys_start: u64, size: u64) -> Result<Arc<Self>, Error> {
        Self::physical(phys_start, size, None, abi::vm::cache::WRITE_COMBINING)
    }

    /// Okno na pamet firmwaru s tabulkami ACPI (AcpiReclaimable, AcpiNvs):
    /// RAM, kterou jadro nikdy nedava alokatoru. Vzdy s cache (stejne
    /// atributy jako v prime mape jadra; jiny typ cache by na ARM znamenal
    /// nesouhlasne atributy jedne stranky). Volajici ruci (pres
    /// `mm::is_acpi_memory`), ze rozsah lezi cely v pameti ACPI.
    pub fn new_firmware(phys_start: u64, size: u64, job: Option<&Arc<Job>>) -> Result<Arc<Self>, Error> {
        Self::physical(phys_start, size, job, abi::vm::cache::CACHED)
    }

    fn physical(
        phys_start: u64,
        size: u64,
        job: Option<&Arc<Job>>,
        cache: u64,
    ) -> Result<Arc<Self>, Error> {
        if size == 0 || !phys_start.is_multiple_of(PAGE) || size > abi::vm::VMO_MAX_SIZE {
            return Err(Error::InvalidArgs);
        }
        if cache > abi::vm::cache::CACHED {
            return Err(Error::InvalidArgs);
        }
        let count = size.div_ceil(PAGE);
        // Registry zarizeni nejsou RAM: jen seznam stranek z rozpoctu jadra.
        let charges = charge(job, count, 0, 0)?;
        let mut pages = page_list(count)?;
        for i in 0..count {
            pages.push(present(PhysAddr::new(phys_start + i * PAGE)))?;
        }
        let mut vmo = Self::with(count * PAGE, pages, Kind::Physical { cache }, 0, charges);
        vmo.revocable = true;
        Ok(Arc::new(vmo))
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
            Kind::Physical { cache: abi::vm::cache::WRITE_COMBINING } => MapFlags::WRITE_COMBINE,
            _ => MapFlags::READ,
        }
    }

    /// Fyzicky rozsah objektu (jen fyzicke objekty, pro kontrolu prostredku).
    pub fn physical_range(&self) -> Option<(u64, u64)> {
        self.is_physical().then(|| (self.first_frame().as_u64(), self.size))
    }

    /// Fyzicka adresa prvni stranky (jen souvisle a fyzicke objekty, ktere
    /// jsou cele pritomne).
    fn first_frame(&self) -> PhysAddr {
        self.page(0).expect("souvisly objekt je cely pritomny")
    }

    /// Je objekt odvolany?
    pub fn is_revoked(&self) -> bool {
        self.revocable && self.revoked.load(Ordering::Acquire)
    }

    /// Zaznamena proces, ktery objekt prave namapoval (vsechny objekty:
    /// odvolani a odmapovani nulove stranky). Odvolany objekt =
    /// `AccessDenied` (volajici mapovani vrati). Priznak a seznam jsou pod
    /// jednim zamkem: mapovani soubezne s `revoke` bud revoke uvidi, nebo
    /// samo uvidi priznak.
    pub fn register_mapper(&self, p: &Arc<Process>) -> Result<(), Error> {
        let mut m = self.mappers.lock();
        if self.is_revoked() {
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
        if !self.revocable {
            return Err(Error::NotSupported);
        }
        let mut m = self.mappers.lock();
        self.revoked.store(true, Ordering::Release);
        Ok(core::mem::take(&mut *m).into_iter().filter_map(|w| w.upgrade()).collect())
    }

    /// Je objekt zapeceteny?
    pub fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::SeqCst)
    }

    /// Zapeceti objekt: obsah se uz nikdy nezmeni (zapis jadrem, mapovani
    /// pro zapis i pripnuti pro zarizeni selzou). Jen bezne objekty. Kdyz
    /// nektere mapovani smi zapisovat (i pres `VM_PROTECT`), `BadState`.
    ///
    /// Soubeh s `VM_MAP`: mapovani se nejdriv zapise mezi mappery a pak pod
    /// zamkem prostoru overi pecet; pecet se nastavi pred prohledanim
    /// mapperu (oboji SeqCst) - bud pecet mapovani najde, nebo mapovani
    /// uvidi pecet.
    pub fn seal(&self) -> Result<(), Error> {
        // Do pripnute pameti muze zapisovat zarizeni: obsah se muze zmenit.
        if !matches!(self.kind, Kind::Pages) || self.for_dma || self.pinned {
            return Err(Error::NotSupported);
        }
        if self.sealed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        if self.mapper_processes().iter().any(|p| p.address_space().maps_writable(self)) {
            self.sealed.store(false, Ordering::SeqCst);
            return Err(Error::BadState);
        }
        Ok(())
    }

    /// Zive procesy s mapovanim objektu.
    fn mapper_processes(&self) -> Vec<Arc<Process>> {
        self.mappers.lock().iter().filter_map(Weak::upgrade).collect()
    }

    /// Mapovani prave dostalo nulovou stranku za dosud nevzniklou stranku.
    /// Volat pod zamkem adresniho prostoru pred kontrolou `page` (viz
    /// `commit_page`).
    pub fn note_zero_mapped(&self) {
        self.zero_mapped.store(true, Ordering::SeqCst);
    }

    /// Ramec pro zapis na offsetu `offset` bez cekani: dosud nevznikla
    /// stranka vznikne (vraci `fresh = true`), vlastni ramec se vrati.
    /// Sdileny ramec (kopie pri zapisu) vraci `None`: volajici pusti zamky
    /// a zavola `writable_page`. Smi se volat pod zamkem prostoru.
    ///
    /// Poradi vuci nulove strance (vse SeqCst): vznik stranky (CAS) je
    /// pred ctenim `zero_mapped`; mapovani nulove stranky nastavi
    /// `zero_mapped` pred ctenim polozky. Kdo nulovou stranku namapoval
    /// po vzniku stranky, uvidel ji uz pritomnou; kdo pred, toho najde
    /// `forget_zero` (bere zamek jeho adresniho prostoru).
    pub fn commit_page_fast(&self, offset: u64) -> Result<Option<(PhysAddr, bool)>, Error> {
        let slot = self.pages.get((offset / PAGE) as usize).ok_or(Error::OutOfRange)?;
        let cur = slot.load(Ordering::SeqCst);
        if cur & SHARED != 0 {
            return Ok(None);
        }
        if cur != 0 {
            return Ok(Some((PhysAddr::new(cur & !ENTRY_FLAGS), false)));
        }
        // Slib je dany objektem, ramec selze jen kdyz jadro vycerpalo
        // pamet nad svou rezervu.
        let frame = phys::alloc_committed_zeroed().ok_or(Error::NoMemory)?;
        match slot.compare_exchange(0, frame.as_u64() | PRESENT, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => {
                self.unbacked.fetch_sub(1, Ordering::AcqRel);
                Ok(Some((frame, true)))
            }
            Err(_) => {
                // Jine vlakno bylo rychlejsi: ramec vratit (i se slibem)
                // a podivat se znovu (ramec mohl mezitim i zacit sdilet).
                phys::free_committed(frame);
                self.commit_page_fast(offset)
            }
        }
    }

    /// Stav stranky: ramec a zda ho objekt vlastni sam (smi se mapovat pro
    /// zapis). `None` = stranka jeste nevznikla.
    pub fn page_state(&self, offset: u64) -> Option<(PhysAddr, bool)> {
        match self.pages.get((offset / PAGE) as usize)?.load(Ordering::SeqCst) {
            0 => None,
            v => Some((PhysAddr::new(v & !ENTRY_FLAGS), v & SHARED == 0)),
        }
    }

    /// Po vzniku stranek objektu jinou cestou nez vypadkem v danem
    /// mapovani: odmapuje vsem mapperum nulovou stranku na `offset..+len`
    /// (dalsi pristup uz najde skutecny ramec). Volat bez zamku adresnich
    /// prostoru.
    pub fn forget_zero(&self, offset: u64, len: u64) {
        if self.zero_mapped.load(Ordering::SeqCst) {
            self.forget_frame(offset, len, crate::mm::zero_frame());
        }
    }

    /// Odmapuje vsem mapperum ramec `frame` na offsetech `offset..+len`
    /// (stranka dostala jiny ramec). Volat bez zamku prostoru a `cow_lock`.
    fn forget_frame(&self, offset: u64, len: u64, frame: PhysAddr) {
        let mut flush = false;
        for p in self.mapper_processes() {
            flush |= p.address_space().drop_frame_ptes(self, offset, len, Some(frame));
        }
        if flush {
            crate::mm::tlb::shootdown();
        }
    }

    /// Vytvori vsechny stranky rozsahu pro zapis jadra (vcetne kopie pri
    /// zapisu); ostatnim mapovanim odmapuje stare ramce.
    fn commit_range(&self, offset: u64, len: u64) -> Result<(), Error> {
        let mut pos = offset & !(PAGE - 1);
        while pos < offset + len {
            self.writable_page(pos)?;
            pos += PAGE;
        }
        Ok(())
    }

    /// Fyzicka adresa stranky pro ovladac (`VMO_PHYS_ADDR`). Bezny objekt
    /// stranku vytvori a pripne: adresa plati po celou dobu zivota objektu
    /// a zadna pozdejsi operace (CoW, pager) ramec nevymeni.
    pub fn expose_phys(&self, offset: u64) -> Result<PhysAddr, Error> {
        if !matches!(self.kind, Kind::Pages) {
            return self.page(offset).ok_or(Error::OutOfRange);
        }
        // Zarizeni by do zapeceteneho obsahu mohlo zapisovat.
        if self.is_sealed() {
            return Err(Error::AccessDenied);
        }
        let slot = self.pages.get((offset / PAGE) as usize).ok_or(Error::OutOfRange)?;
        loop {
            // Vlastni ramec (sdileny se nejdriv zkopiruje); pripnuti pod
            // `cow_lock`, aby ho klonovani mezitim nezacalo sdilet.
            let (frame, _) = self.writable_page(offset)?;
            let _g = self.cow_lock.lock();
            let cur = slot.load(Ordering::SeqCst);
            if cur & SHARED != 0 || cur & !ENTRY_FLAGS != frame.as_u64() {
                continue;
            }
            if slot.fetch_or(PINNED, Ordering::AcqRel) & PINNED == 0 {
                phys::pin(frame);
            }
            return Ok(frame);
        }
    }

    /// Kolik stranek objektu ma ramec (pro testy a prehled).
    pub fn resident_pages(&self) -> u64 {
        self.pages.iter().filter(|p| p.load(Ordering::Relaxed) != 0).count() as u64
    }

    /// Pamet pro DMA (mapuje se s `MapFlags::DMA`)?
    pub fn is_dma(&self) -> bool {
        self.for_dma
    }

    /// Zarizeni pameti pro DMA (`None` = bezna pamet nebo bez identity).
    pub fn dma_device(&self) -> Option<super::quarantine::DeviceId> {
        self.device.map(|(d, _)| d)
    }

    /// Cislo procesu, ktery pamet pro DMA vytvoril (ovladac zarizeni).
    pub fn dma_creator(&self) -> Option<u64> {
        self.device.map(|(_, pid)| pid)
    }

    /// Pamet pro DMA bez domeny IOMMU: zarizeni do ni smi zapisovat bez
    /// omezeni, dokud ho nekdo nezastavi. Po konci procesu ovladace se proto
    /// nevraci alokatoru hned (viz `Process::finish`).
    pub fn is_unprotected_dma(&self) -> bool {
        self.for_dma && self.dma.is_none()
    }

    /// Fyzicka stranka na offsetu `offset` (zarovnanem na stranku), `None`
    /// mimo objekt nebo kdyz jeste nevznikla.
    pub fn page(&self, offset: u64) -> Option<PhysAddr> {
        match self.pages.get((offset / PAGE) as usize)?.load(Ordering::SeqCst) {
            0 => None,
            v => Some(PhysAddr::new(v & !ENTRY_FLAGS)),
        }
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

    /// Projde rozsah po kouscich v ramci stranek: `f(ukazatel do prime mapy
    /// nebo null pro dosud nevzniklou stranku, offset v bufferu, delka)`.
    fn for_each_chunk(&self, offset: u64, len: usize, mut f: impl FnMut(*mut u8, usize, usize)) {
        let mut done = 0;
        while done < len {
            let pos = offset + done as u64;
            let in_page = (pos % PAGE) as usize;
            let n = (PAGE as usize - in_page).min(len - done);
            let ptr = match self.page(pos - pos % PAGE) {
                Some(page) => page.to_virt().as_mut_ptr::<u8>().wrapping_add(in_page),
                None => core::ptr::null_mut(),
            };
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
            if ptr.is_null() {
                // Stranka jeste nevznikla: cte se jako nuly.
                buf[at..at + n].fill(0);
                return;
            }
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
        if self.is_sealed() {
            return Err(Error::AccessDenied);
        }
        self.check_range(offset, data.len() as u64)?;
        self.commit_range(offset, data.len() as u64)?;
        self.for_each_chunk(offset, data.len(), |ptr, at, n| {
            debug_assert!(!ptr.is_null(), "stranka vznikla v commit_range");
            // SAFETY: viz `read`; stranky rozsahu vznikly vyse a uz
            // nezaniknou (zije s objektem).
            unsafe { core::ptr::copy_nonoverlapping(data.as_ptr().add(at), ptr, n) }
        });
        // Zarizeni a mapovani bez cache musi zapsana data videt.
        self.dma_sync(offset, data.len(), arch::dma_sync_for_device);
        Ok(())
    }

    /// U pameti pro DMA zavola `sync` na fyzicky rozsah (objekt je souvisly).
    fn dma_sync(&self, offset: u64, len: usize, sync: fn(u64, u64)) {
        if self.for_dma && len > 0 {
            sync(self.first_frame().as_u64() + offset, len as u64);
        }
    }
}

impl Drop for MemoryObject {
    fn drop(&mut self) {
        // Zarizeni nejdriv ztrati pristup (IOMMU), az pak se pamet vrati.
        if let Some(d) = self.dma.take() {
            d.unmap(self.first_frame().as_u64(), self.allocated_bytes() / PAGE);
        }
        // Pres mapovani bez cache se psalo mimo cache: jeji pripadne stare
        // radky (spekulativni cteni pres primou mapu) zahodit, aby dalsi
        // vlastnik stranek nevidel stara data.
        if self.for_dma {
            arch::dma_sync_for_cpu(self.first_frame().as_u64(), self.allocated_bytes());
            for p in self.pages.iter() {
                phys::unpin(PhysAddr::new(p.load(Ordering::Relaxed) & !ENTRY_FLAGS));
            }
        }
        // Objekt uz nikdo nema namapovany (mapovani drzi Arc), takze stranky
        // smi zpet do alokatoru. Fyzicke okno alokatoru nepatri.
        match self.kind {
            Kind::Pages => {
                for p in self.pages.iter() {
                    let v = p.load(Ordering::Relaxed);
                    let frame = PhysAddr::new(v & !ENTRY_FLAGS);
                    if v & PINNED != 0 {
                        phys::unpin(frame);
                    }
                    // Sdileny ramec drzi dal ostatni vlastnici.
                    if v != 0 && !(v & SHARED != 0 && phys::try_unshare(frame)) {
                        phys::free_pages(frame, 0);
                    }
                }
                // Slib stranek, ktere nikdy nevznikly.
                phys::uncommit(*self.unbacked.get_mut());
            }
            Kind::Contiguous { order } => phys::free_pages(self.first_frame(), order),
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
        Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MAP | Rights::EXECUTE | Rights::DMA
    }
}
