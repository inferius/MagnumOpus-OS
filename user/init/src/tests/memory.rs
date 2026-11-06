//! Objekty pameti a mapovani.

use libos::abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE, PAGE_SIZE as PAGE};
use libos::{Error, Rights, Startup, Vmo, sys};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("zapis a cteni objektu pameti", vmo_roundtrip),
    ("cteni mimo objekt -> OUT_OF_RANGE", vmo_out_of_range),
    (
        "mapovani: zapis pres pamet je videt v objektu",
        map_and_access,
    ),
    ("W^X: zapis+spusteni -> INVALID_ARGS", map_wx),
    ("mapovani bez prava MAP -> ACCESS_DENIED", map_without_right),
    ("neplatny ukazatel -> BAD_ADDRESS", bad_pointer),
    ("odmapovani a nesouvisly rozsah", unmap),
    ("heap roste podle potreby (24 MiB)", heap_grows),
];

fn vmo_roundtrip(_: &Startup) -> TestResult {
    let vmo = Vmo::create(3 * PAGE).map_err(|e| fail("vytvoreni", e))?;
    let data = b"Prilis zlutoucky kun pres hranici stranky";
    vmo.write(PAGE - 10, data).map_err(|e| fail("zapis", e))?;
    let mut back = [0u8; 41];
    vmo.read(PAGE - 10, &mut back)
        .map_err(|e| fail("cteni", e))?;
    check_eq!(&back[..], &data[..], "data po zapisu pres hranici stranky");
    Ok(())
}

fn vmo_out_of_range(_: &Startup) -> TestResult {
    let vmo = Vmo::create(PAGE).map_err(|e| fail("vytvoreni", e))?;
    let mut buf = [0u8; 16];
    check_eq!(
        vmo.read(PAGE - 8, &mut buf),
        Err(Error::OutOfRange),
        "cteni pres konec"
    );
    check_eq!(
        sys::vmo_create(0),
        Err(Error::InvalidArgs),
        "objekt velikosti 0"
    );
    Ok(())
}

fn map_and_access(s: &Startup) -> TestResult {
    let vmo = Vmo::create(2 * PAGE).map_err(|e| fail("vytvoreni", e))?;
    let addr = s
        .process
        .map(&vmo, 0, 2 * PAGE, MAP_READ | MAP_WRITE, 0)
        .map_err(|e| fail("mapovani", e))?;
    // SAFETY: rozsah je prave namapovany pro cteni a zapis.
    unsafe { core::ptr::write_volatile((addr + PAGE + 8) as *mut u64, 0x1234_5678_9abc_def0) };
    let mut back = [0u8; 8];
    vmo.read(PAGE + 8, &mut back)
        .map_err(|e| fail("cteni", e))?;
    check_eq!(
        u64::from_le_bytes(back),
        0x1234_5678_9abc_def0,
        "hodnota zapsana pres mapovani"
    );
    s.process
        .unmap(addr, 2 * PAGE)
        .map_err(|e| fail("odmapovani", e))?;
    Ok(())
}

fn map_wx(s: &Startup) -> TestResult {
    let vmo = Vmo::create(PAGE).map_err(|e| fail("vytvoreni", e))?;
    let r = s
        .process
        .map(&vmo, 0, PAGE, MAP_READ | MAP_WRITE | MAP_EXECUTE, 0);
    check_eq!(r, Err(Error::InvalidArgs), "mapovani W+X");
    Ok(())
}

fn map_without_right(s: &Startup) -> TestResult {
    let vmo = Vmo::create(PAGE).map_err(|e| fail("vytvoreni", e))?;
    let no_map = vmo
        .handle()
        .duplicate(Some(Rights::READ | Rights::WRITE))
        .map_err(|e| fail("dup", e))?;
    let no_map = Vmo::from_handle(no_map);
    check_eq!(
        s.process.map(&no_map, 0, PAGE, MAP_READ, 0),
        Err(Error::AccessDenied),
        "mapovani bez MAP"
    );
    Ok(())
}

fn bad_pointer(_: &Startup) -> TestResult {
    let vmo = Vmo::create(PAGE).map_err(|e| fail("vytvoreni", e))?;
    // Adresa 0x1000 neni namapovana (programy zacinaji na 4 MiB).
    // SAFETY: ukazatel predavame jadru, ktere ho overi; sami ho nepouzijeme.
    let buf = unsafe { core::slice::from_raw_parts_mut(0x1000 as *mut u8, 16) };
    check_eq!(
        vmo.read(0, buf),
        Err(Error::BadAddress),
        "cteni do nenamapovane pameti"
    );
    // Zapis do pameti jen pro cteni (vlastni kod programu).
    let code = bad_pointer as *const () as u64;
    // SAFETY: viz vyse.
    let ro = unsafe { core::slice::from_raw_parts_mut(code as *mut u8, 16) };
    check_eq!(
        vmo.read(0, ro),
        Err(Error::BadAddress),
        "cteni do kodu programu"
    );
    Ok(())
}

fn unmap(s: &Startup) -> TestResult {
    let vmo = Vmo::create(PAGE).map_err(|e| fail("vytvoreni", e))?;
    let addr = s
        .process
        .map(&vmo, 0, PAGE, MAP_READ, 0)
        .map_err(|e| fail("mapovani", e))?;
    check_eq!(
        s.process.unmap(addr + PAGE, PAGE),
        Err(Error::NotFound),
        "odmapovani nenamapovaneho"
    );
    s.process
        .unmap(addr, PAGE)
        .map_err(|e| fail("odmapovani", e))?;
    check_eq!(
        s.process.unmap(addr, PAGE),
        Err(Error::NotFound),
        "dvojite odmapovani"
    );
    Ok(())
}

fn heap_grows(_: &Startup) -> TestResult {
    // Vic nez pocatecni heap (4 MiB): libos musi pridat pamet.
    let mut blocks: alloc::vec::Vec<alloc::vec::Vec<u8>> = alloc::vec::Vec::new();
    for i in 0..6u8 {
        blocks.push(alloc::vec![i; 4 * 1024 * 1024]);
    }
    for (i, b) in blocks.iter().enumerate() {
        check!(b.iter().all(|&x| x == i as u8), "obsah alokace se zmenil");
    }
    drop(blocks);
    // Po uvolneni jde pamet znovu pouzit.
    let big = alloc::vec![7u8; 20 * 1024 * 1024];
    check_eq!(big[20 * 1024 * 1024 - 1], 7, "posledni bajt");
    Ok(())
}
