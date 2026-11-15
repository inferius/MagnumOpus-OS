//! Prehled uloh (`taskd`) v systemu: kanaly razi init, producent hlasi
//! ulohu a dostane zruseni, rozsah klienta (vlastni / vse), neznamy
//! proces v jobu, scrub z fsd a sluzby ze supervisoru.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::job::Job;
use libos::{Channel, Startup, time};
use taskproto::{Control, Origin, Produce, Reply, Request, State, Task, TaskList, actions};

use super::{Test, TestResult, devices, fail};
use crate::tasks;
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("ulohy: producent, rozsah klienta, zruseni", produce_and_scope),
    ("ulohy: neznamy proces v jobu", unknown_process),
    ("ulohy: scrub z fsd a sluzby ze supervisoru", fsd_and_services),
    ("ulohy: 100 soucasnych producentu (smycka pres port)", many_producers),
];

const TIMEOUT: Duration = Duration::from_secs(20);

/// Klient s odberem; `pump` dohani zpravy do seznamu.
struct Watch {
    ch: Channel,
    list: TaskList,
    results: Vec<Reply>,
}

impl Watch {
    fn new(identity: &str, all: bool) -> Result<Self, String> {
        let ch = tasks::client(identity, all, true).map_err(|e| fail("kanal klienta", e))?;
        ch.write(&Request::Subscribe { req: 1, since: None }.encode(), Vec::new()).map_err(|e| fail("odber", e))?;
        Ok(Self { ch, list: TaskList::default(), results: Vec::new() })
    }

    fn pump(&mut self) {
        while let Ok(m) = self.ch.try_read() {
            if let Ok(r) = Reply::decode(&m.data) {
                match r {
                    Reply::Result { .. } | Reply::LogLines { .. } => self.results.push(r),
                    r => {
                        self.list.apply(&r);
                    }
                }
            }
        }
    }

    /// Pocka, az seznam splni `f`.
    fn until(&mut self, what: &str, f: impl Fn(&[Task]) -> bool) -> Result<(), String> {
        let end = time::deadline_after(TIMEOUT);
        loop {
            self.pump();
            if f(&self.list.tasks) {
                return Ok(());
            }
            if time::now() > end {
                let names: Vec<(String, &str)> = self.list.tasks.iter().map(|t| (t.name.clone(), t.state.name())).collect();
                return Err(alloc::format!("{what}: ulohy {names:?}"));
            }
            time::sleep(Duration::from_millis(50));
        }
    }

    fn result(&mut self, req: u32) -> Result<taskproto::Result<()>, String> {
        let end = time::deadline_after(TIMEOUT);
        loop {
            self.pump();
            if let Some(i) = self.results.iter().position(|r| matches!(r, Reply::Result { req: q, .. } if *q == req)) {
                let Reply::Result { result, .. } = self.results.remove(i) else { unreachable!() };
                return Ok(result);
            }
            check!(time::now() < end, "odpoved taskd neprisla");
            time::sleep(Duration::from_millis(20));
        }
    }
}

fn produce_and_scope(_: &Startup) -> TestResult {
    let p = tasks::producer("test-ulohy").map_err(|e| fail("kanal producenta", e))?;
    let reg = Produce::Register {
        req: 5,
        parent: 0,
        name: "Testovaci uloha".into(),
        origin: Origin::Script,
        who: "test".into(),
        agent: true,
        job: None,
        attach: None,
        actions: actions::CANCEL | actions::LOG,
    };
    p.write(&reg.encode(), Vec::new()).map_err(|e| fail("registrace", e))?;
    let m = p.read(Some(time::deadline_after(TIMEOUT))).map_err(|e| fail("REGISTERED", e))?;
    let (req, id) = taskproto::decode_registered(&m.data).map_err(|e| fail("REGISTERED", e))?;
    check_eq!(req, 5, "cislo pozadavku");
    p.write(&Produce::Update { id, state: State::Running, progress: Some(42), eta_ms: Some(1000) }.encode(), Vec::new())
        .map_err(|e| fail("prubeh", e))?;
    p.write(&Produce::LogLine { id, line: "radek vystupu".into() }.encode(), Vec::new()).map_err(|e| fail("log", e))?;
    // Spravce (vse) ulohu vidi i s prubehem a oznacenim agenta.
    let mut admin = Watch::new("spravce", true)?;
    admin.until("uloha u spravce", |ts| ts.iter().any(|t| t.id == id && t.progress == Some(42) && t.agent))?;
    // Uzivatel cizi ulohu nevidi a nezrusi.
    let mut user = Watch::new("uzivatel", false)?;
    user.until("snimek uzivatele", |_| true)?;
    user.pump();
    check!(!user.list.tasks.iter().any(|t| t.id == id), "uzivatel vidi cizi ulohu");
    user.ch.write(&Request::Cancel { req: 9, id }.encode(), Vec::new()).map_err(|e| fail("cancel", e))?;
    check_eq!(user.result(9)?, Err(libos::Error::NotFound), "cizi uloha jako neexistujici");
    // Spravce zrusi: producent dostane CONTROL.
    admin.ch.write(&Request::Cancel { req: 10, id }.encode(), Vec::new()).map_err(|e| fail("cancel", e))?;
    check_eq!(admin.result(10)?, Ok(()), "zruseni spravcem");
    let m = p.read(Some(time::deadline_after(TIMEOUT))).map_err(|e| fail("CONTROL", e))?;
    check_eq!(Control::decode(&m.data), Ok(Control::Cancel(id)), "zruseni u producenta");
    // Vystup ulohy.
    admin.ch.write(&Request::Log { req: 11, id, lines: 5 }.encode(), Vec::new()).map_err(|e| fail("log", e))?;
    let end = time::deadline_after(TIMEOUT);
    loop {
        admin.pump();
        if let Some(Reply::LogLines { lines, .. }) = admin.results.iter().find(|r| matches!(r, Reply::LogLines { req: 11, .. })) {
            check_eq!(lines.clone(), alloc::vec![String::from("radek vystupu")], "vystup ulohy");
            break;
        }
        check!(time::now() < end, "vystup ulohy neprisel");
        time::sleep(Duration::from_millis(20));
    }
    // Producent skonci: jeho bezici uloha skonci chybou.
    drop(p);
    admin.until("konec producenta", |ts| ts.iter().any(|t| t.id == id && matches!(t.state, State::Failed { .. })))?;
    Ok(())
}

/// Vic kanalu nez limit `OBJECT_WAIT_MANY` (64): taskd ceka pres port.
/// 100 producentu, kazdy s ulohou; registrace v obracenem poradi, spravce
/// vidi vsechny; konec pulky producentu ukonci jejich ulohy.
fn many_producers(_: &Startup) -> TestResult {
    const N: usize = 100;
    let mut ps = Vec::new();
    for i in 0..N {
        ps.push(tasks::producer(&alloc::format!("test-mnoho-{i}")).map_err(|e| fail("kanal producenta", e))?);
    }
    for (i, p) in ps.iter().enumerate().rev() {
        let reg = Produce::Register {
            req: i as u32,
            parent: 0,
            name: alloc::format!("Hromadna uloha {i}"),
            origin: Origin::Script,
            who: "test".into(),
            agent: false,
            job: None,
            attach: None,
            actions: actions::CANCEL,
        };
        p.write(&reg.encode(), Vec::new()).map_err(|e| fail("registrace", e))?;
    }
    let mut ids = alloc::vec![0u64; N];
    for (i, p) in ps.iter().enumerate().rev() {
        let m = p.read(Some(time::deadline_after(TIMEOUT))).map_err(|e| fail(&alloc::format!("REGISTERED {i}"), e))?;
        let (req, id) = taskproto::decode_registered(&m.data).map_err(|e| fail("REGISTERED", e))?;
        check_eq!(req, i as u32, "cislo pozadavku");
        ids[i] = id;
        p.write(&Produce::Update { id, state: State::Running, progress: None, eta_ms: None }.encode(), Vec::new())
            .map_err(|e| fail("prubeh", e))?;
    }
    let mut admin = Watch::new("spravce", true)?;
    admin.until("vsechny ulohy u spravce", |ts| ids.iter().all(|id| ts.iter().any(|t| t.id == *id)))?;
    // Zavrit kazdeho druheho producenta: jeho bezici uloha skonci chybou.
    let keep: Vec<Channel> = ps.into_iter().enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, p)| p).collect();
    admin.until("konec pulky producentu", |ts| {
        ids.iter().step_by(2).all(|id| ts.iter().any(|t| t.id == *id && matches!(t.state, State::Failed { .. })))
    })?;
    drop(keep);
    Ok(())
}

fn unknown_process(s: &Startup) -> TestResult {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    let parent = libos::job::current().map_err(|e| fail("vlastni job", e))?;
    let job = Job::create(&parent).map_err(|e| fail("JOB_CREATE", e))?;
    let jid = job.info().map_err(|e| fail("JOB_INFO", e))?.id;
    let (p, ch) = libos::spawn_in(&job, "echo-neznamy", image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    tasks::watch(&job).map_err(|e| fail("watch", e))?;
    let mut admin = Watch::new("spravce", true)?;
    let name = alloc::format!("neznámý proces echo-neznamy v jobu {jid}");
    admin.until("neznamy proces", |ts| ts.iter().any(|t| t.origin == Origin::Unknown && t.name == name && !t.state.finished()))?;
    let mut user = Watch::new("uzivatel", false)?;
    user.until("snimek uzivatele", |_| true)?;
    user.pump();
    check!(!user.list.tasks.iter().any(|t| t.origin == Origin::Unknown), "uzivatel vidi systemovy neznamy proces");
    drop(ch);
    check_eq!(p.wait_exit(Some(time::deadline_after(TIMEOUT))), Ok(0), "konec procesu");
    admin.until("konec neznameho procesu", |ts| ts.iter().any(|t| t.name == name && t.state.finished()))?;
    Ok(())
}

fn fsd_and_services(_: &Startup) -> TestResult {
    let f = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    let mut admin = Watch::new("spravce", true)?;
    f.scrub_start().map_err(|e| fail("scrub_start", e))?;
    admin.until("scrub z fsd skoncil", |ts| {
        ts.iter().any(|t| t.name.starts_with("Scrub") && t.who == "fsd" && matches!(t.state, State::Done(_)))
    })?;
    // Sluzby ze supervisoru (v testovacim behu kolo ridi test).
    {
        let sup = crate::supervisor::SUPERVISOR.lock();
        tasks::sync(&sup);
    }
    let end = time::deadline_after(TIMEOUT);
    loop {
        {
            let sup = crate::supervisor::SUPERVISOR.lock();
            tasks::sync(&sup);
        }
        admin.pump();
        if admin.list.tasks.iter().any(|t| t.name == "Služba fsd" && t.origin == Origin::Service && t.state == State::Running) {
            break;
        }
        check!(time::now() < end, "sluzba fsd v prehledu uloh chybi");
        time::sleep(Duration::from_millis(100));
    }
    Ok(())
}
