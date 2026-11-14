//! Testy oblasti adresniho prostoru: rezervace, mapovani do rezervace,
//! castecne odmapovani, zmena prav, dotaz, ochranna oblast.

use abi::vm::{
    MAP_EXECUTE, MAP_FIXED, MAP_IN_RESERVATION, MAP_READ, MAP_WRITE, RESERVE_GUARD, UNMAP_DECOMMIT,
    region,
};

use crate::arch::paging::PAGE_SIZE;
use crate::error::Error;
use crate::mm::{Access, VirtAddr};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[
    ("oblasti: rezervace 64 GiB stoji jen zaznam", reserve_is_cheap),
    ("oblasti: mapovani do rezervace a decommit", map_in_reservation),
    ("oblasti: castecne odmapovani rozdeli mapovani", partial_unmap),
    ("oblasti: VM_PROTECT a max. prava", protect),
    ("oblasti: dotaz na mapovani, rezervaci a volne misto", query),
];

const GIB: u64 = 1 << 30;
const RW: u32 = MAP_READ | MAP_WRITE;

fn reserve_is_cheap() {
    let job = Job::new_root();
    let p = Process::new("test-res", job.clone()).unwrap();
    let before = job.info().kernel.used;
    let space = p.address_space();
    let base = space.reserve(0, 64 * GIB, 0).unwrap();
    let cost = job.info().kernel.used - before;
    assert!(cost <= 256, "rezervace 64 GiB stala {cost} B");
    // Obsazene misto: bezne mapovani ani dalsi rezervace pres ni nejde.
    let vmo = MemoryObject::new(PAGE_SIZE, None).unwrap();
    assert_eq!(space.map(vmo.clone(), 0, PAGE_SIZE, RW | MAP_FIXED, base), Err(Error::BadState));
    assert_eq!(space.reserve(base + GIB, PAGE_SIZE, MAP_FIXED), Err(Error::BadState));
    // Pristup do rezervace = chyba procesu (nic nevznikne).
    assert_eq!(space.resolve_fault(base + 8, Access::Read), Err(Error::AccessDenied));
    space.unmap(base, 64 * GIB, 0).unwrap();
    assert_eq!(job.info().kernel.used, before, "rozpocet po zruseni rezervace");
    // Ochranna oblast se pozna.
    let guard = space.reserve(0, PAGE_SIZE, RESERVE_GUARD).unwrap();
    assert!(space.is_guard(guard + 16));
    assert!(!space.is_guard(guard + PAGE_SIZE));
    assert_eq!(space.reserve(0, PAGE_SIZE, 1 << 9), Err(Error::InvalidArgs));
}

fn map_in_reservation() {
    let p = Process::new("test-res2", Job::new_root()).unwrap();
    let space = p.address_space();
    let base = space.reserve(0, 16 * PAGE_SIZE, 0).unwrap();
    let vmo = MemoryObject::new(4 * PAGE_SIZE, None).unwrap();
    // Bez MAP_FIXED ani mimo rezervaci nejde.
    assert_eq!(
        space.map(vmo.clone(), 0, 4 * PAGE_SIZE, RW | MAP_IN_RESERVATION, base),
        Err(Error::InvalidArgs)
    );
    assert_eq!(
        space.map(vmo.clone(), 0, 4 * PAGE_SIZE, RW | MAP_FIXED | MAP_IN_RESERVATION, base + 14 * PAGE_SIZE),
        Err(Error::BadState),
        "pres konec rezervace"
    );
    // Uprostred: rezervace se rozdeli na 3 oblasti.
    let at = base + 4 * PAGE_SIZE;
    space
        .map(vmo.clone(), 0, 4 * PAGE_SIZE, RW | MAP_FIXED | MAP_IN_RESERVATION, at)
        .unwrap();
    assert_eq!(space.mapping_count(), 3);
    space.resolve_fault(at + PAGE_SIZE, Access::Write).unwrap();
    assert_eq!(space.query(at).unwrap().attrs & region::ATTR_IN_RESERVATION, region::ATTR_IN_RESERVATION);
    // Decommit: pamet pryc, adresy zustanou rezervovane, jde mapovat znovu.
    space.unmap(at, 4 * PAGE_SIZE, UNMAP_DECOMMIT).unwrap();
    assert!(space.translate(VirtAddr::new(at + PAGE_SIZE)).is_none());
    assert_eq!(space.query(at).unwrap().kind, region::RESERVED);
    space
        .map(vmo.clone(), 0, 4 * PAGE_SIZE, RW | MAP_FIXED | MAP_IN_RESERVATION, at)
        .unwrap();
    // Bez priznaku se cela oblast uvolni.
    space.unmap(base, 16 * PAGE_SIZE, 0).unwrap();
    assert_eq!(space.mapping_count(), 0);
    assert_eq!(space.query(base).unwrap().kind, region::FREE);
}

fn partial_unmap() {
    let job = Job::new_root();
    let p = Process::new("test-split", job.clone()).unwrap();
    let space = p.address_space();
    let before = job.info().kernel.used;
    let vmo = MemoryObject::new(8 * PAGE_SIZE, None).unwrap();
    let addr = space.map(vmo.clone(), 0, 8 * PAGE_SIZE, RW, 0).unwrap();
    for i in 0..8 {
        space.copy_to_user(addr + i * PAGE_SIZE, &[i as u8 + 1]).unwrap();
        space.resolve_fault(addr + i * PAGE_SIZE, Access::Read).unwrap();
    }
    // Vyriznout stranky 2..4: zbudou dve mapovani se spravnymi offsety.
    space.unmap(addr + 2 * PAGE_SIZE, 2 * PAGE_SIZE, 0).unwrap();
    assert_eq!(space.mapping_count(), 2);
    assert!(space.translate(VirtAddr::new(addr + 2 * PAGE_SIZE)).is_none());
    assert_eq!(space.resolve_fault(addr + 3 * PAGE_SIZE, Access::Read), Err(Error::BadAddress));
    let mut b = [0u8];
    space.copy_from_user(addr + 5 * PAGE_SIZE, &mut b).unwrap();
    assert_eq!(b[0], 6, "offset druheho kusu");
    assert_eq!(space.query(addr + 4 * PAGE_SIZE).unwrap().vmo_offset, 4 * PAGE_SIZE);
    // Diry v rozsahu = NotFound, nic se nezmeni.
    assert_eq!(space.unmap(addr, 8 * PAGE_SIZE, 0), Err(Error::NotFound));
    assert_eq!(space.mapping_count(), 2);
    space.unmap(addr, 2 * PAGE_SIZE, 0).unwrap();
    space.unmap(addr + 4 * PAGE_SIZE, 4 * PAGE_SIZE, 0).unwrap();
    assert_eq!(job.info().kernel.used, before, "rozpocet po rozdeleni a odmapovani");
}

fn protect() {
    let p = Process::new("test-prot", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(4 * PAGE_SIZE, None).unwrap();
    let addr = space.map(vmo.clone(), 0, 4 * PAGE_SIZE, RW, 0).unwrap();
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    // Jen pro cteni ve strance 1: zapis se odmitne, stranka odmapovana.
    space.protect(addr + PAGE_SIZE, PAGE_SIZE, MAP_READ).unwrap();
    assert_eq!(space.mapping_count(), 3);
    assert!(space.translate(VirtAddr::new(addr + PAGE_SIZE)).is_none());
    assert_eq!(space.resolve_fault(addr + PAGE_SIZE, Access::Write), Err(Error::AccessDenied));
    space.resolve_fault(addr + PAGE_SIZE, Access::Read).unwrap();
    assert_eq!(space.copy_to_user(addr + PAGE_SIZE, b"x"), Err(Error::BadAddress));
    // Bez pristupu: ani cteni.
    space.protect(addr, PAGE_SIZE, 0).unwrap();
    assert_eq!(space.resolve_fault(addr, Access::Read), Err(Error::AccessDenied));
    let mut b = [0u8];
    assert_eq!(space.copy_from_user(addr, &mut b), Err(Error::BadAddress));
    // Zpet na RW jde (max. prava), spusteni ne a W+X nikdy.
    space.protect(addr, 4 * PAGE_SIZE, RW).unwrap();
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    assert_eq!(space.protect(addr, PAGE_SIZE, MAP_READ | MAP_EXECUTE), Err(Error::AccessDenied));
    assert_eq!(space.protect(addr, PAGE_SIZE, RW | MAP_EXECUTE), Err(Error::InvalidArgs));
    // Rezervace a diry.
    let res = space.reserve(0, PAGE_SIZE, 0).unwrap();
    assert_eq!(space.protect(res, PAGE_SIZE, MAP_READ), Err(Error::BadState));
    // Za rezervaci (lezi hned za mapovanim) je volne misto.
    assert_eq!(space.protect(res + PAGE_SIZE, PAGE_SIZE, MAP_READ), Err(Error::NotFound));
}

fn query() {
    let p = Process::new("test-query", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let a = 0x7000_0000;
    space.map(vmo.clone(), 0, 2 * PAGE_SIZE, RW | MAP_FIXED, a).unwrap();
    space.reserve(a + 4 * PAGE_SIZE, PAGE_SIZE, MAP_FIXED | RESERVE_GUARD).unwrap();
    let m = space.query(a + 100).unwrap();
    assert_eq!((m.start, m.len, m.kind, m.flags, m.max_flags), (a, 2 * PAGE_SIZE, region::MAPPED, RW, RW));
    let free = space.query(a + 3 * PAGE_SIZE).unwrap();
    assert_eq!((free.start, free.len, free.kind), (a + 2 * PAGE_SIZE, 2 * PAGE_SIZE, region::FREE));
    let g = space.query(a + 4 * PAGE_SIZE).unwrap();
    assert_eq!((g.kind, g.attrs), (region::RESERVED, region::ATTR_GUARD));
    assert_eq!(space.query(0x1000), Err(Error::InvalidArgs));
}
