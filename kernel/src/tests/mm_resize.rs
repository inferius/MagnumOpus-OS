//! Testy zvetsitelneho objektu pameti (`VMO_RESIZABLE`, `VMO_SET_SIZE`):
//! slib jen za velikost, cteni/zapis jen do konce, mapovani az do
//! kapacity, vypadek za koncem, zmenseni odmapuje a vrati slib.

use abi::vm::{MAP_READ, MAP_WRITE};

use crate::arch::paging::PAGE_SIZE;
use crate::error::Error;
use crate::mm::{Access, VirtAddr, phys};
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[
    ("zvetsitelny objekt: velikost, slib a meze cteni/zapisu", size_and_commit),
    ("zvetsitelny objekt: mapovani do kapacity, vypadek za koncem, zmenseni odmapuje", mapped_resize),
    ("zvetsitelny objekt: chyby (kapacita, pripnuta stranka, bezny objekt)", errors),
];

fn read(vmo: &MemoryObject, off: u64) -> u8 {
    let mut b = [0u8];
    vmo.read(off, &mut b).unwrap();
    b[0]
}

fn size_and_commit() {
    let base = phys::committed_pages();
    {
        let v = MemoryObject::new_resizable(PAGE_SIZE, 64 * PAGE_SIZE, None).unwrap();
        assert_eq!((v.size(), v.capacity()), (PAGE_SIZE, 64 * PAGE_SIZE));
        assert_eq!(phys::committed_pages(), base + 1, "slib jen za velikost");
        v.write(0, b"a").unwrap();
        assert_eq!(v.write(PAGE_SIZE, b"b").err(), Some(Error::OutOfRange), "zapis za konec");
        // Zvetseni: dalsi slib, zapis do nove casti.
        v.set_size(8 * PAGE_SIZE).unwrap();
        assert_eq!(phys::committed_pages() + 1, base + 8, "slib 8 stranek, 1 uz ma ramec");
        v.write(7 * PAGE_SIZE, b"z").unwrap();
        assert_eq!((read(&v, 0), read(&v, 7 * PAGE_SIZE)), (b'a', b'z'));
        // Bajtova velikost uvnitr stranky.
        v.set_size(7 * PAGE_SIZE + 10).unwrap();
        assert_eq!(v.size(), 7 * PAGE_SIZE + 10);
        assert_eq!(read(&v, 7 * PAGE_SIZE), b'z');
        let mut b = [0u8; 20];
        assert_eq!(v.read(7 * PAGE_SIZE, &mut b).err(), Some(Error::OutOfRange), "cteni za bajtovy konec");
        // Zmenseni: ramce za koncem pryc, slib zpet.
        v.set_size(PAGE_SIZE).unwrap();
        assert_eq!(v.resident_pages(), 1);
        assert_eq!(phys::committed_pages(), base, "jedina stranka ma ramec, zadny slib navic");
        // Znovu zvetsit: nova cast se cte jako nuly (stary obsah pryc).
        v.set_size(8 * PAGE_SIZE).unwrap();
        assert_eq!(read(&v, 7 * PAGE_SIZE), 0, "po zmenseni a zvetseni nuly");
        v.set_size(0).unwrap();
        assert_eq!(v.resident_pages(), 0);
    }
    assert_eq!(phys::committed_pages(), base, "slib se po zaniku vrati cely");
}

fn mapped_resize() {
    let p = Process::new("test-resize", Job::new_root()).unwrap();
    let space = p.address_space();
    let v = MemoryObject::new_resizable(2 * PAGE_SIZE, 16 * PAGE_SIZE, None).unwrap();
    // Mapovani smi pokryt celou kapacitu.
    let addr = space.map(v.clone(), 0, 16 * PAGE_SIZE, MAP_READ | MAP_WRITE, 0).unwrap();
    v.register_mapper(&p).unwrap();
    space.resolve_fault(addr + PAGE_SIZE, Access::Write).unwrap();
    space.copy_to_user(addr + PAGE_SIZE, b"m").unwrap();
    assert_eq!(read(&v, PAGE_SIZE), b'm');
    // Za koncem: vypadek se nevyresi (vyjimka BEYOND_END), jadro nekopiruje.
    assert_eq!(space.resolve_fault(addr + 4 * PAGE_SIZE, Access::Read).err(), Some(Error::OutOfRange));
    assert_eq!(space.resolve_fault(addr + 4 * PAGE_SIZE, Access::Write).err(), Some(Error::OutOfRange));
    assert!(space.copy_to_user(addr + 4 * PAGE_SIZE, b"x").is_err());
    // Zvetseni: stejne mapovani, pristup projde.
    v.set_size(8 * PAGE_SIZE).unwrap();
    space.resolve_fault(addr + 4 * PAGE_SIZE, Access::Write).unwrap();
    space.copy_to_user(addr + 4 * PAGE_SIZE, b"w").unwrap();
    assert_eq!(read(&v, 4 * PAGE_SIZE), b'w');
    // Zmenseni: stranka za koncem zmizi i z tabulek stranek mapperu.
    assert!(space.translate(VirtAddr::new(addr + 4 * PAGE_SIZE)).is_some());
    v.set_size(PAGE_SIZE).unwrap();
    assert!(space.translate(VirtAddr::new(addr + 4 * PAGE_SIZE)).is_none(), "zmenseni odmapovalo");
    assert!(space.translate(VirtAddr::new(addr + PAGE_SIZE)).is_none());
    assert_eq!(space.resolve_fault(addr + PAGE_SIZE, Access::Read).err(), Some(Error::OutOfRange));
    space.resolve_fault(addr, Access::Read).unwrap();
    assert_eq!(v.resident_pages(), 0, "stranka 0 nikdy nevznikla, ostatni uvolneny");
}

fn errors() {
    assert_eq!(MemoryObject::new_resizable(2 * PAGE_SIZE, PAGE_SIZE, None).err(), Some(Error::InvalidArgs));
    assert_eq!(MemoryObject::new_resizable(0, 0, None).err(), Some(Error::InvalidArgs));
    let v = MemoryObject::new_resizable(2 * PAGE_SIZE, 4 * PAGE_SIZE, None).unwrap();
    assert_eq!(v.set_size(5 * PAGE_SIZE).err(), Some(Error::OutOfRange), "nad kapacitu");
    // Pripnutou stranku (adresu zna ovladac) zmenseni neodstrani.
    v.expose_phys(PAGE_SIZE).unwrap();
    assert_eq!(v.set_size(PAGE_SIZE).err(), Some(Error::BadState));
    assert_eq!(v.size(), 2 * PAGE_SIZE);
    let plain = MemoryObject::new(PAGE_SIZE, None).unwrap();
    assert_eq!(plain.set_size(PAGE_SIZE).err(), Some(Error::NotSupported));
    assert_eq!(plain.capacity(), plain.size());
}
