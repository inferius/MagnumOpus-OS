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
    ("libos vm: rezervace, commit, protect, decommit, release", libos_vm_api),
    ("ochranna oblast pod zasobnikem vlakna a procesu", stack_guards),
    ("preteceni zasobniku skonci vypadkem v ochranne oblasti", stack_overflow),
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

fn libos_vm_api(_: &Startup) -> TestResult {
    use libos::vm;
    let pg = page();
    let base = vm::reserve(16 * pg, 0).map_err(|e| fail("reserve", e))?;
    vm::commit(base + 4 * pg, 4 * pg, RW).map_err(|e| fail("commit", e))?;
    // SAFETY: `base+4..base+8 stranek` je prave namapovane RW.
    unsafe {
        poke(base + 4 * pg, 77);
        check_eq!(peek(base + 4 * pg), 77, "data po commitu");
    }
    vm::protect(base + 4 * pg, 4 * pg, MAP_READ).map_err(|e| fail("protect", e))?;
    check_eq!(vm::query(base + 5 * pg).map_err(|e| fail("query", e))?.flags, MAP_READ, "prava");
    vm::decommit(base + 4 * pg, 4 * pg).map_err(|e| fail("decommit", e))?;
    let q = vm::query(base + 5 * pg).map_err(|e| fail("query", e))?;
    check_eq!(q.kind, region::RESERVED, "po decommitu rezervace");
    // Znovu commit: nova anonymni pamet = nuly.
    vm::commit(base + 4 * pg, pg, RW).map_err(|e| fail("commit znovu", e))?;
    // SAFETY: znovu namapovane RW.
    check_eq!(unsafe { peek(base + 4 * pg) }, 0, "nova pamet po decommitu");
    vm::release(base, 16 * pg).map_err(|e| fail("release", e))?;
    let a = vm::alloc(2 * pg, RW).map_err(|e| fail("alloc", e))?;
    // SAFETY: prave alokovane RW.
    unsafe { poke(a + pg, 5) };
    vm::release(a, 2 * pg).map_err(|e| fail("release alloc", e))?;
    check_eq!(vm::query(a).map_err(|e| fail("query", e))?.kind, region::FREE, "po release");
    Ok(())
}

/// Oblast tesne pod oblasti obsahujici `addr`.
fn region_below(q: impl Fn(u64) -> Result<libos::abi::vm::RegionInfo, Error>, addr: u64) -> Result<libos::abi::vm::RegionInfo, alloc::string::String> {
    let r = q(addr).map_err(|e| fail("dotaz", e))?;
    q(r.start - 1).map_err(|e| fail("dotaz pod", e))
}

fn stack_guards(_: &Startup) -> TestResult {
    // Vlakno: oblast pod jeho zasobnikem je ochranna rezervace.
    let w = libos::thread::spawn("guard", || {
        let local = 0u8;
        let here = core::hint::black_box(&local) as *const u8 as u64;
        let below = region_below(libos::vm::query, here);
        STATE.store(
            match below {
                Ok(r) if r.kind == region::RESERVED && r.attrs & region::ATTR_GUARD != 0 => 1,
                _ => 2,
            },
            core::sync::atomic::Ordering::SeqCst,
        );
    })
    .map_err(|e| fail("vlakno", e))?;
    w.join(Some(libos::time::deadline_after(core::time::Duration::from_secs(5))))
        .map_err(|e| fail("join", e))?;
    check_eq!(STATE.load(core::sync::atomic::Ordering::SeqCst), 1, "ochranna oblast pod zasobnikem vlakna");
    Ok(())
}

static STATE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn stack_overflow(s: &Startup) -> TestResult {
    let fs = libos::initfs::Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let (p, ch) = libos::spawn("preteceni", image, alloc::vec::Vec::new()).map_err(|e| fail("spusteni", e))?;
    ch.write(b"overflow", alloc::vec::Vec::new()).map_err(|e| fail("overflow", e))?;
    let code = p
        .wait_exit(Some(libos::time::deadline_after(core::time::Duration::from_secs(20))))
        .map_err(|e| fail("konec", e))?;
    // Vypadek stranky (vektor 14) v ochranne oblasti, ne jina chyba.
    check_eq!(code, libos::abi::process::EXIT_FAULT_BASE - 14, "kod konce po preteceni");
    Ok(())
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
