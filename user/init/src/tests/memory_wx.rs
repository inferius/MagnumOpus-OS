//! Pecet (`VMO_SEAL`) a W^X s opravnenim JIT pres syscally: politika jobu
//! ciloveho procesu, audit v `JOB_PROCESSES`.

use libos::abi::job::{jit, proc_flags};
use libos::abi::vm::{MAP_EXECUTE, MAP_READ, MAP_WRITE};
use libos::job::Job;
use libos::{Error, Process, Startup, Vmo};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("W^X: bez JIT spustit jen zapecetenou pamet", sealed_only),
    ("JIT: politika jobu a audit v prehledu procesu", jit_job),
];

const RX: u32 = MAP_READ | MAP_EXECUTE;

fn page() -> u64 {
    libos::vm::page_size()
}

fn sealed_only(s: &Startup) -> TestResult {
    let parent = libos::job::current().map_err(|e| fail("vlastni job", e))?;
    let job = Job::create(&parent).map_err(|e| fail("job", e))?;
    let p = Process::create(&job, "wx-bez-jit").map_err(|e| fail("proces", e))?;
    let vmo = Vmo::create(page()).map_err(|e| fail("objekt", e))?;
    vmo.write(0, &[0xC3]).map_err(|e| fail("zapis", e))?;
    check_eq!(p.map(&vmo, 0, page(), RX, 0).err(), Some(Error::AccessDenied), "nezapecetena pamet bez JIT");
    // Zapisovatelne mapovani (tady ve vlastnim procesu) pecet nepusti.
    let own = s.process.map(&vmo, 0, page(), MAP_READ | MAP_WRITE, 0).map_err(|e| fail("RW", e))?;
    check_eq!(vmo.seal().err(), Some(Error::BadState), "pecet s RW mapovanim");
    s.process.unmap(own, page()).map_err(|e| fail("odmapovani", e))?;
    vmo.seal().map_err(|e| fail("pecet", e))?;
    p.map(&vmo, 0, page(), RX, 0).map_err(|e| fail("zapecetena pamet RX", e))?;
    check_eq!(vmo.write(0, &[0]).err(), Some(Error::AccessDenied), "zapis po peceti");
    check_eq!(
        s.process.map(&vmo, 0, page(), MAP_READ | MAP_WRITE, 0).err(),
        Some(Error::AccessDenied),
        "mapovani pro zapis po peceti"
    );
    let list = job.processes().map_err(|e| fail("JOB_PROCESSES", e))?;
    check!(list.iter().all(|e| e.flags & proc_flags::JIT_USED == 0), "zapecetena pamet neni JIT");
    Ok(())
}

fn jit_job(_: &Startup) -> TestResult {
    let parent = libos::job::current().map_err(|e| fail("vlastni job", e))?;
    let job = Job::create(&parent).map_err(|e| fail("job", e))?;
    job.set_jit(jit::RUN).map_err(|e| fail("JIT pro job", e))?;
    let p = Process::create(&job, "wx-jit").map_err(|e| fail("proces", e))?;
    let vmo = Vmo::create(page()).map_err(|e| fail("objekt", e))?;
    let at = p.map(&vmo, 0, page(), MAP_READ | MAP_WRITE, 0).map_err(|e| fail("RW", e))?;
    p.protect(at, page(), RX).map_err(|e| fail("RW -> RX s JIT", e))?;
    check_eq!(p.protect(at, page(), RX | MAP_WRITE).err(), Some(Error::InvalidArgs), "W+X ani s JIT");
    let list = job.processes().map_err(|e| fail("JOB_PROCESSES", e))?;
    check!(
        list.iter().any(|e| e.name() == "wx-jit" && e.flags & proc_flags::JIT_USED != 0),
        "audit JIT v prehledu"
    );
    // Job bez GRANT neudeli JIT podrizenemu; odebrani plati hned.
    let nested = Job::create(&job).map_err(|e| fail("vnoreny job", e))?;
    check_eq!(nested.set_jit(jit::RUN).err(), Some(Error::AccessDenied), "udeleni bez GRANT");
    job.set_jit(0).map_err(|e| fail("odebrani", e))?;
    check_eq!(p.protect(at, page(), RX).err(), Some(Error::AccessDenied), "po odebrani JIT");
    Ok(())
}
