//! Testy kopie pri zapisu (`VMO_CLONE`): sdileni ramcu, kopie pri zapisu
//! v obou smerech, posledni vlastnik bez kopie, slib, mapovani, piny.

use abi::vm::{MAP_READ, MAP_WRITE};

use crate::arch::paging::PAGE_SIZE;
use crate::error::Error;
use crate::mm::{Access, VirtAddr, phys};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[
    ("CoW: klon sdili ramce, zapis kopiruje v obou smerech", clone_and_write),
    ("CoW: slib se po zaniku vrati cely", commit_balance),
    ("CoW: mapovani zdroje jen pro cteni, vypadek zkopiruje", mapped_source),
    ("CoW: pripnuty ramec se nesdili, DMA a meze", pinned_and_errors),
];

fn read(vmo: &MemoryObject, off: u64) -> u8 {
    let mut b = [0u8];
    vmo.read(off, &mut b).unwrap();
    b[0]
}

fn clone_and_write() {
    let src = MemoryObject::new(4 * PAGE_SIZE, None).unwrap();
    src.write(0, b"A").unwrap();
    src.write(PAGE_SIZE, b"B").unwrap();
    let clone = src.clone_cow(0, 4 * PAGE_SIZE, None).unwrap();
    let f0 = src.page(0).unwrap();
    assert_eq!(clone.page(0), Some(f0), "klon sdili ramec");
    assert_eq!(phys::shares(f0), 1);
    assert_eq!(clone.page_state(0), Some((f0, false)), "sdileny = jen pro cteni");
    assert_eq!(clone.page(2 * PAGE_SIZE), None, "nevznikla stranka zustava nevznikla");
    // Zapis v klonu: vlastni kopie, zdroj beze zmeny.
    clone.write(0, b"x").unwrap();
    assert_ne!(clone.page(0), Some(f0));
    assert_eq!((read(&clone, 0), read(&src, 0)), (b'x', b'A'));
    assert_eq!(phys::shares(f0), 0, "zdroj je ted jediny vlastnik");
    // Zdroj jako posledni vlastnik pise bez kopie.
    src.write(0, b"a").unwrap();
    assert_eq!(src.page(0), Some(f0), "posledni vlastnik bez kopie");
    // Zapis ve zdroji jako prvni: kopie dostane zdroj, klon necha puvodni.
    let f1 = src.page(PAGE_SIZE).unwrap();
    src.write(PAGE_SIZE, b"b").unwrap();
    assert_ne!(src.page(PAGE_SIZE), Some(f1));
    assert_eq!(clone.page(PAGE_SIZE), Some(f1));
    assert_eq!((read(&src, PAGE_SIZE), read(&clone, PAGE_SIZE)), (b'b', b'B'));
    // Klon klonu: tri vlastnici, kazdy pise do sveho.
    let c2 = clone.clone_cow(PAGE_SIZE, PAGE_SIZE, None).unwrap();
    assert_eq!(c2.page(0), Some(f1));
    c2.write(0, b"2").unwrap();
    assert_eq!((read(&c2, 0), read(&clone, PAGE_SIZE)), (b'2', b'B'));
}

fn commit_balance() {
    let base = phys::committed_pages();
    let (free0, _) = phys::stats();
    {
        let src = MemoryObject::new(8 * PAGE_SIZE, None).unwrap();
        for i in 0..8 {
            src.write(i * PAGE_SIZE, &[i as u8]).unwrap();
        }
        // Klon 8 stranek + 8 novych sdilenych ve zdroji.
        let clone = src.clone_cow(0, 8 * PAGE_SIZE, None).unwrap();
        assert_eq!(phys::committed_pages(), base + 16);
        for i in 0..4 {
            clone.write(i * PAGE_SIZE, b"c").unwrap();
        }
        // 4 kopie (slib klonu) + 4 prevzeti zdrojem bez kopie (slib zpet).
        for i in 0..4 {
            src.write(i * PAGE_SIZE, b"s").unwrap();
        }
        assert_eq!(phys::committed_pages(), base + 16 - 4 - 4);
        drop(clone);
    }
    assert_eq!(phys::committed_pages(), base, "slib po zaniku obou");
    let leaked = free0.saturating_sub(phys::stats().0);
    assert!(leaked < 64, "ramce po zaniku: {leaked} chybi");
}

fn mapped_source() {
    let p = Process::new("test-cow", Job::new_root()).unwrap();
    let space = p.address_space();
    let src = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let addr = space.map(src.clone(), 0, 2 * PAGE_SIZE, MAP_READ | MAP_WRITE, 0).unwrap();
    src.register_mapper(&p).unwrap();
    space.resolve_fault(addr, Access::Write).unwrap();
    space.copy_to_user(addr, b"z").unwrap();
    let f0 = src.page(0).unwrap();
    assert_eq!(space.translate(VirtAddr::new(addr)), Some(f0));
    let clone = src.clone_cow(0, 2 * PAGE_SIZE, None).unwrap();
    assert!(space.translate(VirtAddr::new(addr)).is_none(), "klon odmapoval zapisovatelny zaznam");
    // Cteni: sdileny ramec jen pro cteni. Zapis: kopie, pak vlastni ramec.
    space.resolve_fault(addr, Access::Read).unwrap();
    assert_eq!(space.translate(VirtAddr::new(addr)), Some(f0));
    space.resolve_fault(addr, Access::Write).unwrap();
    space.resolve_fault(addr, Access::Write).unwrap();
    let own = src.page(0).unwrap();
    assert_ne!(own, f0, "zdroj dostal kopii");
    assert_eq!(space.translate(VirtAddr::new(addr)), Some(own));
    assert_eq!(clone.page(0), Some(f0));
    // Kopirovani jadra do sdileneho ramce taky kopiruje.
    let c = clone.clone_cow(0, PAGE_SIZE, None).unwrap();
    let in_c = space.map(c.clone(), 0, PAGE_SIZE, MAP_READ | MAP_WRITE, 0).unwrap();
    c.register_mapper(&p).unwrap();
    space.copy_to_user(in_c, b"k").unwrap();
    assert_eq!((read(&c, 0), read(&clone, 0)), (b'k', b'z'));
}

fn pinned_and_errors() {
    let src = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let pinned = src.expose_phys(0).unwrap();
    src.write(PAGE_SIZE, b"q").unwrap();
    let clone = src.clone_cow(0, 2 * PAGE_SIZE, None).unwrap();
    assert_ne!(clone.page(0), Some(pinned), "pripnuty ramec se kopiruje hned");
    assert_eq!(phys::shares(pinned), 0);
    assert_eq!(clone.page(PAGE_SIZE), src.page(PAGE_SIZE), "ostatni se sdileji");
    // VMO_PHYS_ADDR na sdilenem ramci: nejdriv vlastni kopie, pak pin.
    let f1 = src.page(PAGE_SIZE).unwrap();
    let p1 = src.expose_phys(PAGE_SIZE).unwrap();
    assert_ne!(p1, f1);
    assert!(phys::is_pinned(p1) && !phys::is_pinned(f1));
    let dma = MemoryObject::new_contiguous_dma(PAGE_SIZE, None, None, None).unwrap();
    assert_eq!(dma.clone_cow(0, PAGE_SIZE, None).err(), Some(Error::NotSupported));
    assert_eq!(src.clone_cow(PAGE_SIZE, 2 * PAGE_SIZE, None).err(), Some(Error::OutOfRange));
    assert_eq!(src.clone_cow(1, PAGE_SIZE, None).err(), Some(Error::InvalidArgs));
}
