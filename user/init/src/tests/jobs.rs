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
    ("job: vycet procesu s casem CPU (prehled uloh)", list_processes),
    ("job: JOB_KILL ukonci podstrom a job uzavre", kill_subtree),
    ("job: JOB_KILL vlastniho jobu ukonci i volajiciho", kill_own_job),
    ("job: JOB_KILL behem spousteni nic neproklouzne", kill_races_spawn),
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

fn list_processes(s: &Startup) -> TestResult {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let job = limited_job(UNLIMITED, UNLIMITED)?;
    let nested = Job::create(&job).map_err(|e| fail("vnoreny job", e))?;
    let jid = job.info().map_err(|e| fail("JOB_INFO", e))?.id;
    let nid = nested.info().map_err(|e| fail("JOB_INFO", e))?.id;
    check!(jid != 0 && nid != jid, "cisla jobu");
    check!(job.processes().map_err(|e| fail("JOB_PROCESSES", e))?.is_empty(), "prazdny job");
    let (p, ch) = libos::spawn_in(&nested, "echo-vycet", image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    // Trochu prace, aby mel proces cas na CPU.
    ch.write(b"alloc:65536", Vec::new()).map_err(|e| fail("zapis", e))?;
    ch.read(deadline()).map_err(|e| fail("odpoved", e))?;
    // Vycet nadrazeneho jobu obsahuje i proces vnoreneho. Cas CPU jadro
    // pripise az pri prepnuti vlakna: potomek muze po odpovedi jeste
    // bezet (nebo cekat na prepnuti), proto chvili pockat, nez se ukaze.
    let until = libos::time::deadline_after(core::time::Duration::from_secs(1));
    let mut list = job.processes().map_err(|e| fail("JOB_PROCESSES", e))?;
    while list.first().is_some_and(|e| e.cpu_ns == 0) && libos::time::now() < until {
        libos::time::sleep(core::time::Duration::from_millis(5));
        list = job.processes().map_err(|e| fail("JOB_PROCESSES", e))?;
    }
    check_eq!(list.len(), 1, "jeden proces v podstromu");
    let e = list[0];
    check_eq!(e.name(), "echo-vycet", "jmeno procesu");
    check_eq!(e.job, nid, "job procesu = vnoreny");
    check!(e.threads >= 1 && e.flags & libos::abi::job::proc_flags::FINISHED == 0, "zivy proces");
    check!(e.cpu_ns > 0, "cas na CPU");
    // Bez prava READ nejde.
    let blind = job.duplicate(Some(Rights::BASIC)).map_err(|e| fail("dup", e))?;
    check_eq!(blind.processes().err(), Some(libos::Error::AccessDenied), "bez READ");
    drop(ch);
    check_eq!(p.wait_exit(deadline()), Ok(0), "konec potomka");
    drop(p);
    let after = job.processes().map_err(|e| fail("JOB_PROCESSES", e))?;
    check!(after.iter().all(|e| e.flags & libos::abi::job::proc_flags::FINISHED != 0), "po konci jen skoncene");
    Ok(())
}

/// Proces `echo` v jobu `job`, ktery ceka navzdy.
fn sleeper(s: &Startup, job: &Job, name: &str) -> Result<(Process, libos::Channel), alloc::string::String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let (p, ch) = libos::spawn_in(job, name, image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    ch.write(b"sleep", Vec::new()).map_err(|e| fail("sleep", e))?;
    Ok((p, ch))
}

fn kill_subtree(s: &Startup) -> TestResult {
    let job = limited_job(UNLIMITED, UNLIMITED)?;
    let nested = Job::create(&job).map_err(|e| fail("vnoreny job", e))?;
    let (a, _ca) = sleeper(s, &job, "kill-a")?;
    let (b, _cb) = sleeper(s, &nested, "kill-b")?;
    // Bez prava MANAGE nejde.
    let weak = job.duplicate(Some(Rights::BASIC | Rights::READ)).map_err(|e| fail("dup", e))?;
    check_eq!(weak.kill(1).err(), Some(Error::AccessDenied), "JOB_KILL bez MANAGE");
    job.kill(77).map_err(|e| fail("JOB_KILL", e))?;
    check_eq!(a.wait_exit(deadline()), Ok(77), "proces jobu");
    check_eq!(b.wait_exit(deadline()), Ok(77), "proces vnoreneho jobu");
    // Job je uzavreny: novy proces ani podjob, ani ve vnorenem.
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    check_eq!(libos::spawn_in(&job, "pozde", image, Vec::new()).err(), Some(Error::BadState), "proces po JOB_KILL");
    check_eq!(libos::spawn_in(&nested, "pozde2", image, Vec::new()).err(), Some(Error::BadState), "proces ve vnorenem po JOB_KILL");
    check_eq!(Job::create(&job).err(), Some(Error::BadState), "podjob po JOB_KILL");
    check_eq!(Job::create(&nested).err(), Some(Error::BadState), "podjob vnoreneho po JOB_KILL");
    // Sourozenec (jiny job pod stejnym rodicem) zustava.
    let other = limited_job(UNLIMITED, UNLIMITED)?;
    let (c, cc) = sleeper(s, &other, "kill-c")?;
    check_eq!(c.wait_exit(Some(time::deadline_after(Duration::from_millis(100)))).err(), Some(Error::TimedOut), "jiny job bezi");
    drop(cc);
    c.kill().map_err(|e| fail("uklid", e))?;
    Ok(())
}

fn kill_own_job(s: &Startup) -> TestResult {
    // Definice: volajici uvnitr zabijeneho jobu skonci jako ostatni
    // (syscall se mu vrati, vlakno skonci pri navratu do userspace).
    let job = limited_job(UNLIMITED, UNLIMITED)?;
    let (peer, _pc) = sleeper(s, &job, "kill-soused")?;
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let (p, ch) = libos::spawn_in(&job, "kill-sam", image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    ch.write(b"killjob:9", Vec::new()).map_err(|e| fail("killjob", e))?;
    check_eq!(p.wait_exit(deadline()), Ok(9), "volajici skoncil s kodem");
    check_eq!(peer.wait_exit(deadline()), Ok(9), "soused v jobu");
    check!(!matches!(ch.read(Some(time::deadline_after(Duration::from_millis(50)))), Ok(m) if m.data == b"prezil"), "volajici neprezil");
    Ok(())
}

fn kill_races_spawn(s: &Startup) -> TestResult {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use alloc::sync::Arc;
    let job = limited_job(UNLIMITED, UNLIMITED)?;
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image: Vec<u8> = fs.find("echo").ok_or("echo neni v initfs")?.to_vec();
    let spawned = Arc::new(spin::Mutex::new(Vec::<Process>::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let refused = Arc::new(AtomicU32::new(0));
    // Vlakno spousti procesy a podjoby co nejrychleji, dokud ho job
    // neodmitne; hlavni vlakno mezitim job zabije.
    let spawner = {
        let job = job.duplicate(None).map_err(|e| fail("dup", e))?;
        let (spawned, stop, refused) = (spawned.clone(), stop.clone(), refused.clone());
        libos::thread::spawn("spoustec", move || {
            let mut n = 0u32;
            while !stop.load(Ordering::Relaxed) && n < 200 {
                n += 1;
                let target = if n % 2 == 0 { Job::create(&job).ok() } else { None };
                let r = libos::spawn_in(target.as_ref().unwrap_or(&job), "zavod", &image, Vec::new());
                match r {
                    Ok((p, ch)) => {
                        let _ = ch.write(b"sleep", Vec::new());
                        core::mem::forget(ch);
                        spawned.lock().push(p);
                    }
                    Err(_) => {
                        refused.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
            }
        })
        .map_err(|e| fail("vlakno", e))?
    };
    // Pockat, az bezi aspon par procesu, pak zabit.
    let until = time::deadline_after(Duration::from_secs(5));
    while spawned.lock().len() < 3 && time::now() < until {
        time::sleep(Duration::from_millis(2));
    }
    job.kill(55).map_err(|e| fail("JOB_KILL", e))?;
    let _ = spawner.join(Some(time::deadline_after(Duration::from_secs(10))));
    stop.store(true, Ordering::Relaxed);
    check!(refused.load(Ordering::Relaxed) >= 1 || spawned.lock().len() >= 200, "spoustec narazil na uzavreny job");
    // Kazdy proces, ktery vznikl, skoncil (zadny neproklouzl).
    for p in spawned.lock().iter() {
        check_eq!(p.wait_exit(deadline()), Ok(55), "proces vznikly pred nebo behem JOB_KILL");
    }
    let left = job.processes().map_err(|e| fail("JOB_PROCESSES", e))?;
    check!(left.iter().all(|e| e.flags & libos::abi::job::proc_flags::FINISHED != 0), "v jobu nic nebezi");
    check_eq!(Job::create(&job).err(), Some(Error::BadState), "podjob po JOB_KILL");
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
