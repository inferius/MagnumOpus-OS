//! Testy DMA po kusech (`object/dma.rs`) bez skutecneho prenosu: okno IOVA,
//! pripnuta pamet v rozpoctu `DMA`, mapovani bez IOMMU (fyzicke useky)
//! a s IOMMU (souvisly rozsah IOVA, napoveda, uvolneni). Skutecny prenos
//! zarizeni a zablokovany pristup po odmapovani testuje sada `ovladace`
//! v initu (zarizeni edu).

use alloc::sync::Arc;
use alloc::vec;

use crate::arch::iommu::DmaAccess;
use crate::arch::paging::PAGE_SIZE as PAGE;
use crate::log;
use crate::mm::phys;
use crate::object::dma::{DmaMap, window_for};
use crate::object::job::{Job, Pool};
use crate::object::resource::Resource;
use crate::object::vmo::MemoryObject;
use crate::object::Error;

pub const TESTS: &[(&str, fn())] = &[
    ("DMA: okno IOVA nad koncem RAM", window),
    ("DMA: pripnuta pamet v rozpoctu DMA", pinned_budget),
    ("DMA: mapovani bez IOMMU = fyzicke useky", map_direct),
    ("DMA: mapovani s IOMMU = souvisla IOVA", map_iommu),
];

const RW: DmaAccess = DmaAccess::READ_WRITE;
const GIB: u64 = 1 << 30;

fn window() {
    assert_eq!(window_for(3 * GIB, 39), (4 * GIB, 1 << 39), "pod 4 GiB okno od 4 GiB");
    assert_eq!(window_for(5 * GIB + 1, 48), (6 * GIB, 1 << 48), "zaokrouhleni na 1 GiB");
    let (b, e) = window_for(1 << 40, 39);
    assert!(b >= e, "RAM nad sirkou domeny: prazdne okno");
}

/// Job s vlastnim limitem pripnute pameti.
fn job_with_dma(limit: u64) -> Arc<Job> {
    let job = Job::new_child(&Job::new_root()).unwrap();
    job.set_limit(Pool::Dma, limit);
    job
}

fn pinned_budget() {
    let job = job_with_dma(2 * PAGE);
    assert_eq!(
        MemoryObject::new_pinned(3 * PAGE, Some(&job)).err(),
        Some(Error::NoResources),
        "nad limit DMA"
    );
    assert_eq!(job.info().dma.used, 0, "neuspech nic nestrhl");
    let vmo = MemoryObject::new_pinned(2 * PAGE, Some(&job)).unwrap();
    assert!(vmo.is_pinned());
    let info = job.info();
    assert_eq!((info.dma.used, info.user.used), (2 * PAGE, 2 * PAGE), "DMA navic k uzivatelske");
    let frames = [vmo.page(0).unwrap(), vmo.page(PAGE).unwrap()];
    assert!(frames.iter().all(|&f| phys::is_pinned(f)), "stranky pripnute hned");
    assert_eq!(vmo.seal(), Err(Error::NotSupported), "pripnutou pamet nejde zapecetit");
    // Bezny objekt pripnuty neni a DMA_MAP ho odmitne.
    assert!(!MemoryObject::new(PAGE, Some(&job)).unwrap().is_pinned());
    drop(vmo);
    assert_eq!(job.info().dma.used, 0, "zanik vratil rozpocet");
    assert!(frames.iter().all(|&f| !phys::is_pinned(f)), "zanik odepnul stranky");
    // Korenovy job: povinny strop, nejde zvednout.
    let root = Job::new_root();
    let cap = root.info().dma.limit;
    assert!(cap > 0 && cap != abi::job::UNLIMITED, "koren ma strop pripnute pameti");
    root.set_limit(Pool::Dma, abi::job::UNLIMITED);
    assert_eq!(root.info().dma.limit, cap, "strop korene nejde zvednout");
}

/// Prostredek ovladace pro zarizeni `device` s kvotou DMA `dma_max`.
fn resource(device: (u16, u8, u8), dma_max: u64) -> Arc<Resource> {
    Resource::new_root().restrict(vec![], 0, dma_max, Some(device)).unwrap()
}

fn map_direct() {
    // Prostredek zarizeni bez domeny (i na stroji s IOMMU).
    let res = Resource::new_test_device(3 * PAGE, (0xFFF2, 0, 0));
    assert!(res.device_domain().is_none());
    let job = job_with_dma(1 << 20);
    let vmo = MemoryObject::new_pinned(4 * PAGE, Some(&job)).unwrap();
    let map = |off, len, hint| DmaMap::new(&res, vmo.clone(), off, len, RW, hint, &job);
    // Spatne argumenty.
    assert_eq!(map(1, PAGE, 0).err(), Some(Error::InvalidArgs), "nezarovnany offset");
    assert_eq!(map(0, 0, 0).err(), Some(Error::InvalidArgs), "nulova delka");
    assert_eq!(map(PAGE, 4 * PAGE, 0).err(), Some(Error::InvalidArgs), "za koncem objektu");
    assert_eq!(map(0, PAGE, 1 << 40).err(), Some(Error::NotSupported), "napoveda bez IOMMU");
    let none = DmaAccess { read: false, write: false };
    assert_eq!(
        DmaMap::new(&res, vmo.clone(), 0, PAGE, none, 0, &job).err(),
        Some(Error::InvalidArgs),
        "bez smeru"
    );
    let plain = MemoryObject::new(PAGE, Some(&job)).unwrap();
    assert_eq!(
        DmaMap::new(&res, plain, 0, PAGE, RW, 0, &job).err(),
        Some(Error::BadState),
        "nepripnuta pamet"
    );
    // Kvota prostredku: 3 stranky; 4 neprojdou, vratka po zaniku.
    assert_eq!(map(0, 4 * PAGE, 0).err(), Some(Error::NoResources), "nad kvotu prostredku");
    let m = map(PAGE, 3 * PAGE, 0).unwrap();
    assert!(!m.is_protected());
    let total: u64 = m.segments().iter().map(|s| s.len).sum();
    assert_eq!(total, 3 * PAGE);
    // Useky = fyzicke stranky v poradi (sousedni slouceny).
    let mut at = PAGE;
    for s in m.segments() {
        let mut off = 0;
        while off < s.len {
            assert_eq!(vmo.page(at).unwrap().as_u64(), s.addr + off, "usek neodpovida strance");
            off += PAGE;
            at += PAGE;
        }
    }
    assert_eq!(m.sync(0, 3 * PAGE + 1, abi::dma::sync::FOR_CPU), Err(Error::InvalidArgs));
    assert_eq!(m.sync(0, PAGE, 7), Err(Error::InvalidArgs));
    m.sync(PAGE / 2, PAGE, abi::dma::sync::FOR_DEVICE).unwrap();
    assert_eq!(map(0, PAGE, 0).err(), Some(Error::NoResources), "kvota plna");
    drop(m);
    let m = map(0, 3 * PAGE, 0).unwrap();
    // Mapovani drzi pamet: objekt prezije zanik handlu.
    let frame = vmo.page(0).unwrap();
    drop(vmo);
    assert!(phys::is_pinned(frame), "mapovani drzi pripnutou pamet");
    assert!(Arc::strong_count(m.memory()) >= 1);
}

fn map_iommu() {
    // Neexistujici zarizeni na posledni sbernici segmentu 0: pokud ho
    // nektera IOMMU pokryva, domena vznikne (nikdo jiny ji nepouziva).
    let device = (0, 0xFF, 0xF8);
    let res = resource(device, 1 << 20);
    let Some(domain) = res.device_domain().cloned() else {
        log::info!("DMA: zadna IOMMU nepokryva zarizeni 00:ff:1f.0, test mapovani s IOMMU preskocen");
        return;
    };
    let (base, end) = domain.window();
    assert!(base >= phys::ram_end() && base >= 1 << 32 && base < end, "okno {base:#x}..{end:#x}");
    let job = job_with_dma(1 << 20);
    let vmo = MemoryObject::new_pinned(8 * PAGE, Some(&job)).unwrap();
    let map = |off, len, hint| DmaMap::new(&res, vmo.clone(), off, len, RW, hint, &job);
    let a = map(0, 3 * PAGE, 0).unwrap();
    assert!(a.is_protected());
    assert_eq!(a.segments().len(), 1, "s IOMMU jeden souvisly usek");
    let ia = a.device_addr();
    assert!(ia >= base && ia + 3 * PAGE <= end, "IOVA v okne");
    let b = map(3 * PAGE, PAGE, 0).unwrap();
    assert!(b.device_addr() >= ia + 4 * PAGE, "ochranna stranka za prvnim rozsahem");
    // Napoveda: prekryv odmitnut, volne misto prijato.
    assert_eq!(map(0, PAGE, ia + PAGE).err(), Some(Error::AlreadyExists), "prekryv napovedy");
    assert_eq!(map(0, PAGE, end).err(), Some(Error::InvalidArgs), "napoveda mimo okno");
    let hint = base + (1 << 24);
    let h = map(4 * PAGE, 2 * PAGE, hint).unwrap();
    assert_eq!(h.device_addr(), hint);
    // Jen cteni zarizenim projde take (smer vynucuje IOMMU).
    let ro = DmaMap::new(&res, vmo.clone(), 6 * PAGE, PAGE, DmaAccess { read: true, write: false }, 0, &job);
    assert!(ro.is_ok());
    // Zanik uvolni rozsah IOVA: nove mapovani dostane stejnou adresu.
    drop(a);
    let again = map(0, 2 * PAGE, 0).unwrap();
    assert_eq!(again.device_addr(), ia, "uvolneny rozsah IOVA se pouzije znovu");
    drop((again, b, h, ro));
    // Dalsi prostredek stejneho zarizeni sdili domenu i alokator.
    let res2 = resource(device, 1 << 20);
    assert!(Arc::ptr_eq(res2.device_domain().unwrap(), &domain));
    assert_eq!(domain.device(), device);
}
