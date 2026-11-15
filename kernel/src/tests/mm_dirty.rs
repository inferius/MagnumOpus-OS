//! Testy sledovani zapisu (`VMO_DIRTY`, `object/vmo/dirty.rs`): prvni
//! volani vrati existujici stranky, cista stranka se mapuje jen pro cteni,
//! zapis pres mapovani, `VMO_WRITE` i kopii jadra stranku oznaci, po
//! vycisteni zapis znovu vyvola vypadek.

use abi::vm::{MAP_READ, MAP_WRITE};

use crate::arch::paging::PAGE_SIZE;
use crate::mm::{Access, VirtAddr};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[("sledovani zapisu: spinave stranky, vypadek na prvni zapis po vycisteni", dirty_pages)];

fn dirty_pages() {
    let p = Process::new("test-dirty", Job::new_root()).unwrap();
    let space = p.address_space();
    let v = MemoryObject::new(8 * PAGE_SIZE, None).unwrap();
    v.write(PAGE_SIZE, b"a").unwrap();
    v.write(5 * PAGE_SIZE, b"b").unwrap();
    assert!(!v.tracking());
    // Prvni volani: sledovani zapne, existujici stranky jsou spinave.
    assert_eq!(v.take_dirty(64).unwrap(), [1, 5]);
    assert!(v.tracking());
    assert_eq!(v.take_dirty(64).unwrap(), [] as [u64; 0], "po vycisteni nic");
    // Mapovani pro zapis: cista stranka jen pro cteni.
    let addr = space.map(v.clone(), 0, 8 * PAGE_SIZE, MAP_READ | MAP_WRITE, 0).unwrap();
    v.register_mapper(&p).unwrap();
    assert!(v.tracked_clean(PAGE_SIZE));
    space.resolve_fault(addr + PAGE_SIZE, Access::Read).unwrap();
    assert!(space.translate(VirtAddr::new(addr + PAGE_SIZE)).is_some());
    assert!(v.tracked_clean(PAGE_SIZE), "cteni stranku neoznaci");
    // Zapis: vypadek oznaci (i namapovanou pro cteni) a namapuje pro zapis.
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    space.resolve_fault(addr + 3 * PAGE_SIZE, Access::Write).unwrap();
    assert!(!v.tracked_clean(PAGE_SIZE));
    // Kopie jadra do pameti procesu a VMO_WRITE oznaci primo.
    space.copy_to_user(addr + 6 * PAGE_SIZE, b"k").unwrap();
    v.write(7 * PAGE_SIZE, b"w").unwrap();
    // Omezeni poctu: zbytek pristim volanim.
    assert_eq!(v.take_dirty(2).unwrap(), [1, 3]);
    assert_eq!(v.take_dirty(64).unwrap(), [6, 7]);
    // Vycistene stranky jsou odmapovane (dalsi zapis = vypadek).
    assert!(space.translate(VirtAddr::new(addr + PAGE_SIZE)).is_none(), "odmapovano pri vycisteni");
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    assert_eq!(v.take_dirty(64).unwrap(), [1]);
    let mut b = [0u8];
    v.read(6 * PAGE_SIZE, &mut b).unwrap();
    assert_eq!(b[0], b'k');
}
