//! Testy mechanismu pro userspace na urovni jadra: adresni prostor procesu,
//! objekty pameti, kanaly. (Uplne testy pres syscally bezi v procesu init.)

use alloc::vec;
use alloc::vec::Vec;

use abi::vm::{MAP_EXECUTE, MAP_FIXED, MAP_READ, MAP_WRITE};

use crate::arch::paging::PAGE_SIZE;

use crate::error::Error;
use crate::mm::VirtAddr;
use crate::object::channel::{Channel, Message};
use crate::object::event::Event;
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;
use crate::object::{KernelObject, Rights, Signals};

pub const TESTS: &[(&str, fn())] = &[
    (
        "adresni prostor: mapovani, kopie, odmapovani",
        address_space,
    ),
    ("adresni prostor: kontroly argumentu", address_space_checks),
    ("kanal: zpravy, handly, uzavreni", channel),
];

fn address_space() {
    let p = Process::new("test-as", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    let addr = space
        .map(vmo.clone(), 0, 2 * PAGE_SIZE, MAP_READ | MAP_WRITE, 0)
        .unwrap();
    assert_eq!(space.mapping_count(), 1);
    assert!(space.translate(VirtAddr::new(addr)).is_some());

    // Zapis pres hranici stranky se objevi v objektu pameti.
    let data: Vec<u8> = (0..100).collect();
    space.copy_to_user(addr + PAGE_SIZE - 50, &data).unwrap();
    let mut back = vec![0u8; 100];
    vmo.read(PAGE_SIZE - 50, &mut back).unwrap();
    assert_eq!(back, data);
    let mut via_user = vec![0u8; 100];
    space
        .copy_from_user(addr + PAGE_SIZE - 50, &mut via_user)
        .unwrap();
    assert_eq!(via_user, data);

    // Kopie pres konec mapovani se odmitne cela.
    assert_eq!(
        space.copy_to_user(addr + 2 * PAGE_SIZE - 10, &data),
        Err(Error::BadAddress)
    );

    space.unmap(addr, 2 * PAGE_SIZE).unwrap();
    assert_eq!(space.mapping_count(), 0);
    assert!(space.translate(VirtAddr::new(addr)).is_none());
}

fn address_space_checks() {
    let p = Process::new("test-as2", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(PAGE_SIZE, None).unwrap();
    let wx = MAP_READ | MAP_WRITE | MAP_EXECUTE;
    assert_eq!(
        space.map(vmo.clone(), 0, PAGE_SIZE, wx, 0),
        Err(Error::InvalidArgs)
    );
    assert_eq!(
        space.map(vmo.clone(), 0, PAGE_SIZE, MAP_WRITE, 0),
        Err(Error::InvalidArgs)
    );
    assert_eq!(
        space.map(vmo.clone(), 0, 2 * PAGE_SIZE, MAP_READ, 0),
        Err(Error::OutOfRange)
    );
    // Jadro do horni poloviny nikdy nemapuje pamet procesu.
    let kernel_addr = 0xFFFF_8000_0000_0000;
    assert_eq!(
        space.map(vmo.clone(), 0, PAGE_SIZE, MAP_READ | MAP_FIXED, kernel_addr),
        Err(Error::InvalidArgs)
    );
    let fixed = 0x50_0000;
    space
        .map(vmo.clone(), 0, PAGE_SIZE, MAP_READ | MAP_FIXED, fixed)
        .unwrap();
    assert_eq!(
        space.map(vmo.clone(), 0, PAGE_SIZE, MAP_READ | MAP_FIXED, fixed),
        Err(Error::BadState)
    );
    // Zapis do mapovani jen pro cteni.
    assert_eq!(space.copy_to_user(fixed, &[1]), Err(Error::BadAddress));
}

fn channel() {
    let (a, b) = Channel::create();
    let event = Event::new();
    let msg = Message::new(b"ahoj".to_vec(), vec![(event.clone(), Rights::WAIT)]);
    a.write(msg).unwrap();
    assert!(b.core().signals().contains(Signals::READABLE));
    assert_eq!(b.read(2, 1).err(), Some(Error::BufferTooSmall));
    let m = b.read(100, 1).unwrap();
    assert_eq!(m.data, b"ahoj");
    assert_eq!(m.handles.len(), 1);
    assert!(!b.core().signals().contains(Signals::READABLE));
    assert_eq!(b.read(100, 1).err(), Some(Error::ShouldWait));
    drop(a);
    assert!(b.core().signals().contains(Signals::PEER_CLOSED));
    assert_eq!(b.read(100, 1).err(), Some(Error::PeerClosed));
    let empty = Message::new(Vec::new(), Vec::new());
    assert_eq!(b.write(empty).err(), Some(Error::PeerClosed));
}
