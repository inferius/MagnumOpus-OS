//! Joby: rozpocty pameti jadra a uzivatelske pameti, dedeni limitu.

use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::job::{CHILD_RIGHTS, Job, UNLIMITED, pool};
use libos::{Error, Event, Process, Rights, Startup, Thread, Vmo, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("job: rozpocet sleduje pamet procesu", own_budget),
    ("job: limit uzivatelske pameti potomka", user_limit),
    ("job: limit pameti jadra (zasobnik vlakna)", kernel_limit),
    ("job: potomek si limit nezvedne", child_cannot_raise),
    ("job: program v omezenem jobu", spawned_in_limited_job),
    ("handly: 140 000 handlu v procesu", many_handles),
];

const KIB: u64 = 1024;

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(5)))
}

fn limited_job(user: u64, kernel: u64) -> Result<Job, alloc::string::String> {
    let parent = libos::job::current().map_err(|e| fail("vlastni job", e))?;
    let job = Job::create(&parent).map_err(|e| fail("JOB_CREATE", e))?;
    job.set_limit(pool::USER, user)
        .map_err(|e| fail("limit uzivatelske pameti", e))?;
    job.set_limit(pool::KERNEL, kernel)
        .map_err(|e| fail("limit pameti jadra", e))?;
    Ok(job)
}

fn own_budget(_: &Startup) -> TestResult {
    // Vlastni job bez limitu: spotreba presne podle toho, co v nem vznikne.
    let job = limited_job(UNLIMITED, UNLIMITED)?;
    let empty = job.info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!((empty.user.used, empty.kernel.used), (0, 0), "prazdny job");
    check_eq!(empty.user.available, UNLIMITED, "bez limitu");
    let p = Process::create(&job, "rozpocet").map_err(|e| fail("proces", e))?;
    let with_process = job.info().map_err(|e| fail("JOB_INFO", e))?;
    check!(with_process.kernel.used > 0, "proces se nestrhl z pameti jadra");
    let vmo = Vmo::create_for(1024 * KIB, &p).map_err(|e| fail("vytvoreni", e))?;
    let during = job.info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(during.user.used, 1024 * KIB, "stranky objektu");
    check!(
        during.kernel.used > with_process.kernel.used,
        "objekt se nestrhl z pameti jadra"
    );
    // Rodic (init) vidi spotrebu potomka i ve svem jobu.
    let mine = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check!(mine.user.used >= 1024 * KIB, "predek nevidi spotrebu potomka");
    drop(vmo);
    let after = job.info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(after.user.used, 0, "uzivatelska pamet po uvolneni");
    check_eq!(after.kernel.used, with_process.kernel.used, "pamet jadra po uvolneni");
    drop(p);
    check_eq!(job.info().map_err(|e| fail("JOB_INFO", e))?.kernel.used, 0, "po zaniku procesu");
    Ok(())
}

fn user_limit(_: &Startup) -> TestResult {
    let job = limited_job(256 * KIB, UNLIMITED)?;
    let p = Process::create(&job, "limit").map_err(|e| fail("proces v jobu", e))?;
    check_eq!(
        Vmo::create_for(512 * KIB, &p).err(),
        Some(Error::NoResources),
        "objekt nad limit jobu"
    );
    let ok = Vmo::create_for(128 * KIB, &p).map_err(|e| fail("objekt pod limit", e))?;
    let info = job.info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(info.user.used, 128 * KIB, "spotreba jobu");
    check_eq!(info.user.limit, 256 * KIB, "limit jobu");
    check_eq!(info.user.available, 128 * KIB, "zbyva");
    // Podrizeny job je omezeny limitem predka, i kdyz vlastni limit nema.
    let nested = Job::create(&job).map_err(|e| fail("vnoreny job", e))?;
    let q = Process::create(&nested, "vnoreny").map_err(|e| fail("vnoreny proces", e))?;
    check_eq!(
        Vmo::create_for(192 * KIB, &q).err(),
        Some(Error::NoResources),
        "limit predka plati pro vnoreny job"
    );
    drop(ok);
    check!(Vmo::create_for(192 * KIB, &q).is_ok(), "po uvolneni se misto vrati");
    Ok(())
}

fn kernel_limit(_: &Startup) -> TestResult {
    // Na proces staci; zasobnik jadra vlakna (64 KiB) uz ne.
    let job = limited_job(UNLIMITED, 32 * KIB)?;
    let p = Process::create(&job, "bez-vlaken").map_err(|e| fail("proces", e))?;
    check_eq!(
        Thread::create(&p, "vlakno").err(),
        Some(Error::NoResources),
        "vlakno nad rozpocet jadra"
    );
    job.set_limit(pool::KERNEL, UNLIMITED)
        .map_err(|e| fail("zruseni limitu", e))?;
    check!(Thread::create(&p, "vlakno").is_ok(), "vlakno bez limitu");
    Ok(())
}

fn child_cannot_raise(_: &Startup) -> TestResult {
    let job = limited_job(64 * KIB, UNLIMITED)?;
    let child_view = job
        .duplicate(Some(CHILD_RIGHTS))
        .map_err(|e| fail("duplikace", e))?;
    check_eq!(
        child_view.set_limit(pool::USER, UNLIMITED).err(),
        Some(Error::AccessDenied),
        "zmena limitu bez WRITE"
    );
    check!(child_view.info().is_ok(), "stav rozpoctu jde cist");
    // Vlastni podrizeny job potomek zalozit muze a limit si v nem zvedne,
    // ale porad plati limit predka.
    let sub = Job::create(&child_view).map_err(|e| fail("podrizeny job", e))?;
    sub.set_limit(pool::USER, UNLIMITED)
        .map_err(|e| fail("limit v podrizenem jobu", e))?;
    let p = Process::create(&sub, "obchazi").map_err(|e| fail("proces", e))?;
    check_eq!(
        Vmo::create_for(128 * KIB, &p).err(),
        Some(Error::NoResources),
        "limit predka nejde obejit"
    );
    check_eq!(
        Job::create(&job.duplicate(Some(Rights::BASIC | Rights::READ)).map_err(|e| fail("dup", e))?).err(),
        Some(Error::AccessDenied),
        "job bez MANAGE"
    );
    Ok(())
}

fn spawned_in_limited_job(s: &Startup) -> TestResult {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let job = limited_job(8 * 1024 * KIB, UNLIMITED)?;
    let (p, ch) = libos::spawn_in(&job, "echo-limit", image, Vec::new())
        .map_err(|e| fail("spusteni", e))?;
    // Pamet programu (segmenty, zasobnik, heap) jde na ucet jobu potomka.
    let used = job.info().map_err(|e| fail("JOB_INFO", e))?.user.used;
    check!(used >= 256 * KIB, "pamet programu se nestrhla z jobu potomka");
    let ask = |msg: &str| -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        ch.write(msg.as_bytes(), Vec::new())
            .map_err(|e| fail("zapis", e))?;
        Ok(ch.read(deadline()).map_err(|e| fail("odpoved", e))?.data)
    };
    check_eq!(ask("alloc:65536")?, b"ok".to_vec(), "mala alokace v potomkovi");
    check_eq!(
        ask("alloc:16777216")?,
        b"err:NO_RESOURCES".to_vec(),
        "alokace nad limit v potomkovi"
    );
    drop(ch);
    check_eq!(p.wait_exit(deadline()), Ok(0), "konec potomka");
    drop(p);
    let info = job.info().map_err(|e| fail("JOB_INFO", e))?;
    check_eq!(info.user.used, 0, "uzivatelska pamet po konci potomka");
    check_eq!(info.kernel.used, 0, "pamet jadra po konci potomka");
    Ok(())
}

fn many_handles(_: &Startup) -> TestResult {
    // Drive pevny limit 65 536 handlu na proces; ted jen rozpocet jobu.
    let ev = Event::create().map_err(|e| fail("event", e))?;
    let n = 140_000;
    let mut held = Vec::new();
    held.try_reserve(n).map_err(|_| "pamet pro seznam handlu")?;
    for _ in 0..n {
        held.push(ev.handle().duplicate(None).map_err(|e| fail("duplikace", e))?);
    }
    check_eq!(held.len(), n, "pocet handlu");
    Ok(())
}
