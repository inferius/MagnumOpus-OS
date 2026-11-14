//! Kopie pri zapisu pres syscally (`VMO_CLONE`) a sdileny kod programu.

use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::initfs::Initfs;
use libos::{Error, Mapping, Startup, Vmo, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("VMO_CLONE: kopie pri zapisu v obou smerech", clone_isolation),
    ("sdileny kod: dve instance programu z jedne sablony", shared_code),
];

fn page() -> u64 {
    libos::vm::page_size()
}

fn clone_isolation(_: &Startup) -> TestResult {
    let pg = page();
    let src = Vmo::create(4 * pg).map_err(|e| fail("objekt", e))?;
    src.write(0, &1u64.to_le_bytes()).map_err(|e| fail("zapis", e))?;
    src.write(pg, &2u64.to_le_bytes()).map_err(|e| fail("zapis", e))?;
    let clone = src.clone_range(0, 4 * pg).map_err(|e| fail("VMO_CLONE", e))?;
    let a = Mapping::new(&src, 0, 4 * pg, MAP_READ | MAP_WRITE).map_err(|e| fail("mapovani zdroje", e))?;
    let b = Mapping::new(&clone, 0, 4 * pg, MAP_READ | MAP_WRITE).map_err(|e| fail("mapovani klonu", e))?;
    check_eq!((a.read::<u64>(0), b.read::<u64>(0)), (1, 1), "klon vidi data zdroje");
    // Zapis v klonu se zdroje netyka a naopak.
    b.write::<u64>(0, 10);
    a.write::<u64>(pg, 20);
    check_eq!((a.read::<u64>(0), b.read::<u64>(0)), (1, 10), "zapis v klonu");
    check_eq!((a.read::<u64>(pg), b.read::<u64>(pg)), (20, 2), "zapis ve zdroji");
    // Zapis jadrem (VMO_WRITE) do sdilene stranky take kopiruje.
    let c2 = clone.clone_range(pg, pg).map_err(|e| fail("klon klonu", e))?;
    c2.write(0, &7u64.to_le_bytes()).map_err(|e| fail("VMO_WRITE", e))?;
    check_eq!(b.read::<u64>(pg), 2, "klon po zapisu do klonu klonu");
    // Meze.
    check_eq!(src.clone_range(0, 8 * pg).err(), Some(Error::OutOfRange), "pres konec");
    check_eq!(src.clone_range(1, pg).err(), Some(Error::InvalidArgs), "nezarovnany offset");
    Ok(())
}

fn shared_code(s: &Startup) -> TestResult {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let mut procs = Vec::new();
    let mut counts = Vec::new();
    for i in 0..2 {
        let (p, ch) = libos::spawn(if i == 0 { "echo-sdil-1" } else { "echo-sdil-2" }, image, Vec::new())
            .map_err(|e| fail("spusteni", e))?;
        ch.write(b"ping:x", Vec::new()).map_err(|e| fail("ping", e))?;
        let reply = ch
            .read(Some(time::deadline_after(Duration::from_secs(5))))
            .map_err(|e| fail("odpoved", e))?;
        check_eq!(reply.data, b"pong:x".to_vec(), "odpoved instance");
        counts.push(libos::spawn::shared_images());
        procs.push((p, ch));
    }
    check!(counts[0] >= 1, "program z initfs ma sablonu");
    check_eq!(counts[1], counts[0], "druha instance pouzila stejnou sablonu");
    for (p, ch) in procs {
        ch.write(b"exit:0", Vec::new()).map_err(|e| fail("exit", e))?;
        check_eq!(p.wait_exit(Some(time::deadline_after(Duration::from_secs(5)))), Ok(0), "konec instance");
    }
    Ok(())
}
