//! Oblasti adresniho prostoru pres syscally: rezervace (jako Wine),
//! mapovani do rezervace, decommit, castecne odmapovani, VM_PROTECT,
//! VM_QUERY.

use libos::abi::vm::{
    MAP_EXECUTE, MAP_FIXED, MAP_IN_RESERVATION, MAP_READ, MAP_WRITE, RESERVE_GUARD, region,
};
use libos::{Error, Startup, Vmo};

use super::{Test, TestResult, fail};
use crate::check_eq;

pub const TESTS: &[Test] = &[
    ("oblasti: rezervace 64 GiB a mapovani do ni (Wine)", reserve_and_commit),
    ("oblasti: castecne odmapovani a VM_PROTECT", partial_and_protect),
    ("oblasti: VM_QUERY a chyby argumentu", query_and_errors),
];

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
const RW: u32 = MAP_READ | MAP_WRITE;

fn page() -> u64 {
    libos::vm::page_size()
}

/// SAFETY volajiciho: `addr` je namapovany pro cteni.
unsafe fn peek(addr: u64) -> u64 {
    // SAFETY: predano volajicim.
    unsafe { core::ptr::read_volatile(addr as *const u64) }
}

/// SAFETY volajiciho: `addr` je namapovany pro zapis.
unsafe fn poke(addr: u64, v: u64) {
    // SAFETY: predano volajicim.
    unsafe { core::ptr::write_volatile(addr as *mut u64, v) }
}

fn reserve_and_commit(s: &Startup) -> TestResult {
    let p = &s.process;
    // Cenu rezervace (jen zaznam) overuje self-test jadra; job initu
    // zahrnuje i bezici sluzby, tady by mereni nebylo spolehlive.
    let base = p.reserve(0, 64 * GIB, 0).map_err(|e| fail("rezervace 64 GiB", e))?;
    // Commit kousku uprostred na pevne adrese.
    let vmo = Vmo::create(2 * MIB).map_err(|e| fail("objekt", e))?;
    let at = base + 10 * GIB;
    check_eq!(
        p.map(&vmo, 0, 2 * MIB, RW | MAP_FIXED, at).err(),
        Some(Error::BadState),
        "bez MAP_IN_RESERVATION je rezervace obsazena"
    );
    p.map(&vmo, 0, 2 * MIB, RW | MAP_FIXED | MAP_IN_RESERVATION, at)
        .map_err(|e| fail("mapovani do rezervace", e))?;
    // SAFETY: `at..at+2 MiB` je prave namapovane RW.
    unsafe {
        poke(at, 0xABCD);
        poke(at + MIB, 0x1234);
        check_eq!(peek(at) + peek(at + MIB), 0xABCD + 0x1234, "data v rezervaci");
    }
    // Decommit: zpet rezervace. Data zustala v objektu, nove mapovani
    // tehoz objektu je ukaze.
    p.decommit(at, 2 * MIB).map_err(|e| fail("decommit", e))?;
    check_eq!(p.query(at).map_err(|e| fail("dotaz", e))?.kind, region::RESERVED, "po decommitu");
    p.map(&vmo, 0, 2 * MIB, MAP_READ | MAP_FIXED | MAP_IN_RESERVATION, at)
        .map_err(|e| fail("znovu do rezervace", e))?;
    // SAFETY: znovu namapovane pro cteni.
    check_eq!(unsafe { peek(at) }, 0xABCD, "data objektu po novem mapovani");
    p.unmap(base, 64 * GIB).map_err(|e| fail("zruseni cele rezervace", e))?;
    check_eq!(p.query(at).map_err(|e| fail("dotaz", e))?.kind, region::FREE, "po zruseni");
    Ok(())
}

fn partial_and_protect(s: &Startup) -> TestResult {
    let p = &s.process;
    let pg = page();
    let vmo = Vmo::create(4 * pg).map_err(|e| fail("objekt", e))?;
    let addr = p.map(&vmo, 0, 4 * pg, RW, 0).map_err(|e| fail("mapovani", e))?;
    for i in 0..4 {
        // SAFETY: `addr..addr+4 stranky` je namapovane RW.
        unsafe { poke(addr + i * pg, i + 1) };
    }
    // Vyriznout stranku 1; okolni zustanou s puvodnimi daty.
    p.unmap(addr + pg, pg).map_err(|e| fail("castecne odmapovani", e))?;
    // SAFETY: stranky 0, 2 a 3 zustaly namapovane.
    unsafe {
        check_eq!(peek(addr) + peek(addr + 2 * pg) + peek(addr + 3 * pg), 1 + 3 + 4, "zbyle stranky");
    }
    check_eq!(p.query(addr + pg).map_err(|e| fail("dotaz", e))?.kind, region::FREE, "dira");
    // Stranka 2 jen pro cteni: cteni jde, mapovani pres dotaz ukaze prava.
    p.protect(addr + 2 * pg, pg, MAP_READ).map_err(|e| fail("VM_PROTECT", e))?;
    let q = p.query(addr + 2 * pg).map_err(|e| fail("dotaz", e))?;
    check_eq!((q.flags, q.max_flags), (MAP_READ, RW), "prava po VM_PROTECT");
    // SAFETY: stranka 2 je namapovana pro cteni.
    check_eq!(unsafe { peek(addr + 2 * pg) }, 3, "cteni po VM_PROTECT");
    check_eq!(p.protect(addr + 2 * pg, pg, MAP_READ | MAP_EXECUTE).err(), Some(Error::AccessDenied), "nad max. prava");
    check_eq!(p.protect(addr + 2 * pg, pg, RW | MAP_EXECUTE).err(), Some(Error::InvalidArgs), "W+X");
    check_eq!(p.protect(addr, 4 * pg, MAP_READ).err(), Some(Error::NotFound), "pres diru");
    p.protect(addr + 2 * pg, pg, RW).map_err(|e| fail("zpet RW", e))?;
    // SAFETY: stranka 2 je znovu RW.
    unsafe {
        poke(addr + 2 * pg, 33);
        check_eq!(peek(addr + 2 * pg), 33, "zapis po navratu prav");
    }
    p.unmap(addr, pg).map_err(|e| fail("uklid", e))?;
    p.unmap(addr + 2 * pg, 2 * pg).map_err(|e| fail("uklid", e))?;
    Ok(())
}

fn query_and_errors(s: &Startup) -> TestResult {
    let p = &s.process;
    let pg = page();
    let g = p.reserve(0, 4 * pg, RESERVE_GUARD).map_err(|e| fail("ochranna rezervace", e))?;
    let q = p.query(g + pg).map_err(|e| fail("dotaz", e))?;
    check_eq!((q.start, q.len, q.kind, q.attrs), (g, 4 * pg, region::RESERVED, region::ATTR_GUARD), "ochranna rezervace");
    check_eq!(p.reserve(g, pg, MAP_FIXED).err(), Some(Error::BadState), "rezervace pres rezervaci");
    check_eq!(p.reserve(0, 0, 0).err(), Some(Error::InvalidArgs), "nulova delka");
    check_eq!(p.reserve(0, pg, 1 << 12).err(), Some(Error::InvalidArgs), "neznamy priznak");
    check_eq!(p.query(0x10).err(), Some(Error::InvalidArgs), "dotaz pod USER_MIN");
    p.unmap(g, 4 * pg).map_err(|e| fail("zruseni", e))?;
    Ok(())
}
