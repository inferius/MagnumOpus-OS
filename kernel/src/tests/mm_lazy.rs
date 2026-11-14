//! Testy stranek na pozadani: commit, vypadek, nulova stranka, piny.
//!
//! Vypadky se tu volaji primo (`AddressSpace::resolve_fault`) na prostoru,
//! ktery neni aktivni; skutecne vypadky z userspace testuje sada `pamet`
//! v initu.

use alloc::vec;

use abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE};

use crate::arch::paging::PAGE_SIZE;
use crate::error::Error;
use crate::mm::{Access, VirtAddr, phys, zero_frame};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[
    ("stranky na pozadani: commit bez ramcu", commit_without_frames),
    ("stranky na pozadani: strop commitu = volna pamet", commit_ceiling),
    ("vypadek: cteni = nulova stranka, zapis = vlastni ramec", fault_read_then_write),
    ("vypadek: prava a adresy mimo mapovani", fault_denied),
    ("nulova stranka: zapis jinou cestou ji odmapuje vsem", zero_page_coherence),
    ("kopirovani jadra pres dosud nevznikle stranky", kernel_copy_lazy),
    ("piny: DMA a VMO_PHYS_ADDR", pins),
];

const MIB: u64 = 1024 * 1024;

fn commit_without_frames() {
    let committed = phys::committed_pages();
    let (free, _) = phys::stats();
    let vmo = MemoryObject::new(64 * MIB, None).unwrap();
    assert_eq!(phys::committed_pages(), committed + 64 * MIB / PAGE_SIZE, "slib");
    // Jen seznam stranek v heapu (128 KiB), zadne ramce.
    let used = free - phys::stats().0;
    assert!(used < 1024, "vytvoreni 64 MiB vzalo {used} ramcu");
    assert_eq!(vmo.resident_pages(), 0);
    // Cteni dosud nevznikle stranky = nuly, ramec nevznikne.
    let mut buf = vec![0xAAu8; 16];
    vmo.read(5 * MIB, &mut buf).unwrap();
    assert!(buf.iter().all(|&b| b == 0));
    assert_eq!(vmo.resident_pages(), 0);
    // Zapis vytvori jen dotcene stranky (pres hranici = 2).
    vmo.write(PAGE_SIZE - 2, &[1, 2, 3, 4]).unwrap();
    assert_eq!(vmo.resident_pages(), 2);
    assert_eq!(phys::committed_pages(), committed + 64 * MIB / PAGE_SIZE - 2);
    drop(vmo);
    assert_eq!(phys::committed_pages(), committed, "slib po zaniku");
}

fn commit_ceiling() {
    let (_, total) = phys::stats();
    // Vic nez cela RAM nejde slibit ani bez rozpoctu jobu.
    assert_eq!(MemoryObject::new(2 * total * PAGE_SIZE, None).err(), Some(Error::NoMemory));
    // Slib ubira pamet ostatnim alokacim programu.
    let room = (1..=40)
        .map(|i| i * 32 * MIB)
        .take_while(|&s| phys::user_room(s / PAGE_SIZE))
        .last()
        .unwrap_or(0);
    assert!(room >= 64 * MIB, "volne pameti malo na test: {room}");
    let big = MemoryObject::new(room, None).unwrap();
    assert!(!phys::user_room(room / PAGE_SIZE), "slibena pamet je dal volna");
    drop(big);
    assert!(phys::user_room(room / PAGE_SIZE));
}

fn fault_read_then_write() {
    let p = Process::new("test-lazy", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(4 * PAGE_SIZE, None).unwrap();
    let addr = space.map(vmo.clone(), 0, 4 * PAGE_SIZE, MAP_READ | MAP_WRITE, 0).unwrap();
    let va = VirtAddr::new(addr + PAGE_SIZE);
    assert!(space.translate(va).is_none(), "mapovani nemapuje nevzniklou stranku");
    space.resolve_fault(addr + PAGE_SIZE + 8, Access::Read).unwrap();
    assert_eq!(space.translate(va), Some(zero_frame()), "cteni = nulova stranka");
    assert_eq!(vmo.resident_pages(), 0);
    // Planny vypadek (soubeh) nic nezmeni.
    space.resolve_fault(addr + PAGE_SIZE, Access::Read).unwrap();
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    let frame = vmo.page(PAGE_SIZE).expect("zapis vytvoril stranku");
    assert_ne!(frame, zero_frame());
    assert_eq!(space.translate(va), Some(frame), "zapis = vlastni ramec");
    assert_eq!(vmo.resident_pages(), 1);
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    assert_eq!(vmo.resident_pages(), 1);
    // Existujici stranka se pri novem mapovani namapuje hned.
    let again = space.map(vmo.clone(), 0, 4 * PAGE_SIZE, MAP_READ, 0).unwrap();
    assert_eq!(space.translate(VirtAddr::new(again + PAGE_SIZE)), Some(frame));
    assert!(space.translate(VirtAddr::new(again)).is_none());
}

fn fault_denied() {
    let p = Process::new("test-lazy2", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(PAGE_SIZE, None).unwrap();
    let ro = space.map(vmo.clone(), 0, PAGE_SIZE, MAP_READ, 0).unwrap();
    assert_eq!(space.resolve_fault(ro, Access::Write), Err(Error::AccessDenied));
    assert_eq!(space.resolve_fault(ro, Access::Exec), Err(Error::AccessDenied));
    assert_eq!(vmo.resident_pages(), 0, "odmitnuty zapis nic nevytvoril");
    let rx = space.map(vmo.clone(), 0, PAGE_SIZE, MAP_READ | MAP_EXECUTE, 0).unwrap();
    space.resolve_fault(rx, Access::Exec).unwrap();
    assert_eq!(space.resolve_fault(ro + 64 * PAGE_SIZE, Access::Read), Err(Error::BadAddress));
    assert_eq!(space.resolve_fault(0x1000, Access::Read), Err(Error::BadAddress));
}

fn zero_page_coherence() {
    let a = Process::new("test-zero-a", Job::new_root()).unwrap();
    let b = Process::new("test-zero-b", Job::new_root()).unwrap();
    let vmo = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let rw = MAP_READ | MAP_WRITE;
    let in_a = a.address_space().map(vmo.clone(), 0, 2 * PAGE_SIZE, rw, 0).unwrap();
    let in_a2 = a.address_space().map(vmo.clone(), 0, 2 * PAGE_SIZE, MAP_READ, 0).unwrap();
    let in_b = b.address_space().map(vmo.clone(), 0, 2 * PAGE_SIZE, rw, 0).unwrap();
    vmo.register_mapper(&a).unwrap();
    vmo.register_mapper(&b).unwrap();
    for (p, addr) in [(&a, in_a), (&a, in_a2), (&b, in_b)] {
        p.address_space().resolve_fault(addr, Access::Read).unwrap();
        assert_eq!(p.address_space().translate(VirtAddr::new(addr)), Some(zero_frame()));
    }
    // Zapis vypadkem v A: vlastni ramec v A, ostatni nulovou stranku ztrati.
    a.address_space().resolve_fault(in_a, Access::Write).unwrap();
    let frame = vmo.page(0).unwrap();
    assert_eq!(a.address_space().translate(VirtAddr::new(in_a)), Some(frame));
    assert!(a.address_space().translate(VirtAddr::new(in_a2)).is_none(), "druhe mapovani v A");
    assert!(b.address_space().translate(VirtAddr::new(in_b)).is_none(), "mapovani v B");
    b.address_space().resolve_fault(in_b, Access::Read).unwrap();
    assert_eq!(b.address_space().translate(VirtAddr::new(in_b)), Some(frame));
    // Zapis pres VMO_WRITE (jadro) do druhe stranky.
    for (p, addr) in [(&a, in_a2), (&b, in_b)] {
        p.address_space().resolve_fault(addr + PAGE_SIZE, Access::Read).unwrap();
    }
    vmo.write(PAGE_SIZE + 3, b"xyz").unwrap();
    for (p, addr) in [(&a, in_a2), (&b, in_b)] {
        assert!(p.address_space().translate(VirtAddr::new(addr + PAGE_SIZE)).is_none());
        let mut got = [0u8; 3];
        p.address_space().copy_from_user(addr + PAGE_SIZE + 3, &mut got).unwrap();
        assert_eq!(&got, b"xyz");
    }
}

fn kernel_copy_lazy() {
    let p = Process::new("test-copy", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let addr = space.map(vmo.clone(), 0, 2 * PAGE_SIZE, MAP_READ | MAP_WRITE, 0).unwrap();
    // Cteni nevzniklych stranek = nuly, nic nevznikne.
    let mut buf = [0xAAu8; 32];
    space.copy_from_user(addr + PAGE_SIZE - 16, &mut buf).unwrap();
    assert!(buf.iter().all(|&b| b == 0));
    assert_eq!(vmo.resident_pages(), 0);
    // Nulova stranka v tomto prostoru (proces neni v seznamu mapperu, jako
    // init po userboot): zapis jadra ji musi odmapovat i tak.
    space.resolve_fault(addr, Access::Read).unwrap();
    space.copy_to_user(addr + PAGE_SIZE - 2, b"ahoj").unwrap();
    assert_eq!(vmo.resident_pages(), 2);
    assert!(space.translate(VirtAddr::new(addr)).is_none(), "nulova stranka zustala");
    let mut back = [0u8; 4];
    vmo.read(PAGE_SIZE - 2, &mut back).unwrap();
    assert_eq!(&back, b"ahoj");
    // Zapis jadra do mapovani jen pro cteni se odmitne a nic nevytvori.
    let ro_vmo = MemoryObject::new(PAGE_SIZE, None).unwrap();
    let ro = space.map(ro_vmo.clone(), 0, PAGE_SIZE, MAP_READ, 0).unwrap();
    assert_eq!(space.copy_to_user(ro, b"x"), Err(Error::BadAddress));
    assert_eq!(ro_vmo.resident_pages(), 0);
}

fn pins() {
    let dma = MemoryObject::new_contiguous_dma(3 * PAGE_SIZE, None, None).unwrap();
    let frame = dma.page(PAGE_SIZE).unwrap();
    assert!(phys::is_pinned(frame), "DMA pamet je pripnuta");
    drop(dma);
    assert!(!phys::is_pinned(frame), "po zaniku odepnuta");
    // VMO_PHYS_ADDR na beznem objektu: stranka vznikne a pripne se.
    let vmo = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let phys_addr = vmo.expose_phys(PAGE_SIZE).unwrap();
    assert_eq!(vmo.page(PAGE_SIZE), Some(phys_addr));
    assert!(phys::is_pinned(phys_addr));
    assert_eq!(vmo.expose_phys(PAGE_SIZE).unwrap(), phys_addr, "adresa stala");
    assert_eq!(vmo.resident_pages(), 1);
    drop(vmo);
    assert!(!phys::is_pinned(phys_addr), "odepnuto jednou (pin jen jednou)");
}
