//! Testy pecete (`VMO_SEAL`) a W^X s opravnenim JIT (politika jobu).

use abi::job::jit;
use abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE};

use crate::arch::paging::PAGE_SIZE;
use crate::error::Error;
use crate::object::job::Job;
use crate::object::process::Process;
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[
    ("pecet: obsah se uz nezmeni", seal),
    ("JIT: politika jobu, udelovani a odebrani", jit_policy),
    ("W^X: spusteni nezapecetene pameti jen s JIT", protect_exec),
];

const RX: u32 = MAP_READ | MAP_EXECUTE;
const RW: u32 = MAP_READ | MAP_WRITE;

fn seal() {
    let p = Process::new("test-seal", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(2 * PAGE_SIZE, None).unwrap();
    vmo.write(0, b"kod").unwrap();
    // Zapisovatelne mapovani pecet nepusti (i kdyz je ted jen pro cteni).
    let rw = space.map(vmo.clone(), 0, 2 * PAGE_SIZE, RW, 0).unwrap();
    vmo.register_mapper(&p).unwrap();
    space.protect(rw, 2 * PAGE_SIZE, MAP_READ, false).unwrap();
    assert_eq!(vmo.seal(), Err(Error::BadState));
    assert!(!vmo.is_sealed());
    space.unmap(rw, 2 * PAGE_SIZE, 0).unwrap();
    vmo.seal().unwrap();
    vmo.seal().unwrap();
    assert_eq!(vmo.write(0, b"x"), Err(Error::AccessDenied));
    assert_eq!(space.map(vmo.clone(), 0, PAGE_SIZE, RW, 0), Err(Error::AccessDenied));
    assert_eq!(vmo.expose_phys(0), Err(Error::AccessDenied));
    let ro = space.map_with_max(vmo.clone(), 0, PAGE_SIZE, MAP_READ, 0, RW | MAP_EXECUTE).unwrap();
    // Nejvyssi prava zapeceteneho mapovani nikdy nezahrnou zapis.
    assert_eq!(space.query(ro).unwrap().max_flags, RX);
    // Klon zapeceteneho objektu je obycejny (soukroma kopie).
    let c = vmo.clone_cow(0, 2 * PAGE_SIZE, None).unwrap();
    c.write(0, b"y").unwrap();
    let mut b = [0u8; 3];
    vmo.read(0, &mut b).unwrap();
    assert_eq!(&b, b"kod");
    let dma = MemoryObject::new_contiguous_dma(PAGE_SIZE, None, None).unwrap();
    assert_eq!(dma.seal(), Err(Error::NotSupported));
}

fn jit_policy() {
    let root = Job::new_root();
    assert!(!root.may_jit(), "koren JIT jen udeluje");
    let a = Job::new_child(&root).unwrap();
    assert!(!a.may_jit(), "novy job JIT nema");
    let b = Job::new_child(&a).unwrap();
    assert_eq!(b.set_jit(jit::RUN), Err(Error::AccessDenied), "rodic neudeluje");
    a.set_jit(jit::GRANT).unwrap();
    assert!(!a.may_jit(), "GRANT neni RUN");
    b.set_jit(jit::RUN).unwrap();
    assert!(b.may_jit());
    // Odebrani u predka plati hned i pro potomky.
    a.set_jit(0).unwrap();
    assert!(!b.may_jit());
    assert_eq!(b.set_jit(8), Err(Error::InvalidArgs));
    b.set_jit(0).unwrap();
}

fn protect_exec() {
    let p = Process::new("test-wx", Job::new_root()).unwrap();
    let space = p.address_space();
    let vmo = MemoryObject::new(PAGE_SIZE, None).unwrap();
    // JIT: RW mapovani s nejvyssimi pravy RWX, prepinani RW <-> RX.
    let at = space.map_with_max(vmo.clone(), 0, PAGE_SIZE, RW, 0, RW | MAP_EXECUTE).unwrap();
    assert_eq!(space.protect(at, PAGE_SIZE, RX, false), Err(Error::AccessDenied), "bez JIT");
    assert_eq!(space.protect(at, PAGE_SIZE, RX, true), Ok(true), "s JIT (audit)");
    assert_eq!(space.protect(at, PAGE_SIZE, RW | MAP_EXECUTE, true), Err(Error::InvalidArgs), "W+X nikdy");
    assert_eq!(space.protect(at, PAGE_SIZE, RW, true), Ok(false));
    // Zapecetene: spusteni bez JIT, bez auditu.
    let sealed = MemoryObject::new(PAGE_SIZE, None).unwrap();
    sealed.seal().unwrap();
    let s = space.map_with_max(sealed, 0, PAGE_SIZE, MAP_READ, 0, RX).unwrap();
    assert_eq!(space.protect(s, PAGE_SIZE, RX, false), Ok(false));
}
