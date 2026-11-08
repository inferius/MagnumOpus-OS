//! Testy jadra taskd: registrace, prava, odber a navazani, akce,
//! uklid hotovych, neznamy proces v jobu.

use std::vec;

use taskproto::{Applied, TaskList};

use super::*;

fn user() -> Scope {
    Scope { identity: "uzivatel".into(), all: false, control: true }
}

fn admin() -> Scope {
    Scope { identity: "admin".into(), all: true, control: true }
}

fn summary() -> Scope {
    Scope { identity: "compd".into(), all: true, control: false }
}

/// Zpravy klientovi `c` jako odpovedi.
fn to_client(out: &[Out], c: ClientId) -> Vec<Reply> {
    out.iter()
        .filter_map(|o| match o {
            Out::Client(id, d) if *id == c => Some(Reply::decode(d).unwrap()),
            _ => None,
        })
        .collect()
}

fn to_producer(out: &[Out], p: ProducerId) -> Vec<Vec<u8>> {
    out.iter()
        .filter_map(|o| match o {
            Out::Producer(id, d) if *id == p => Some(d.clone()),
            _ => None,
        })
        .collect()
}

fn register(core: &mut Core, p: ProducerId, name: &str, job: Option<u64>, acts: u32, now: u64) -> (u64, Vec<Out>) {
    let m = Produce::Register {
        req: 7,
        parent: 0,
        name: name.into(),
        origin: Origin::User,
        who: "shell".into(),
        agent: false,
        job,
        attach: Some((1, 2)),
        actions: acts,
    };
    let out = core.producer_msg(p, &m.encode(), now);
    let reg = to_producer(&out, p);
    let (req, id) = taskproto::decode_registered(&reg[0]).unwrap();
    assert_eq!(req, 7);
    (id, out)
}

fn subscribe(core: &mut Core, c: ClientId, list: &mut TaskList) {
    for r in to_client(&core.client_msg(c, &Request::Subscribe { req: 1, since: None }.encode(), 0), c) {
        list.apply(&r);
    }
}

fn apply(list: &mut TaskList, out: &[Out], c: ClientId) {
    for r in to_client(out, c) {
        assert_ne!(list.apply(&r), Applied::Gap, "dira v poradovych cislech");
    }
}

#[test]
fn own_tasks_only_and_events_without_gaps() {
    let mut core = Core::new();
    let shell = core.add_producer("uzivatel");
    let sup = core.add_producer("system");
    let u = core.add_client(user());
    let a = core.add_client(admin());
    let (mut lu, mut la) = (TaskList::default(), TaskList::default());
    subscribe(&mut core, u, &mut lu);
    subscribe(&mut core, a, &mut la);
    let (t1, out) = register(&mut core, shell, "kopirovani", None, actions::CANCEL, 10);
    apply(&mut lu, &out, u);
    apply(&mut la, &out, a);
    let (t2, out) = register(&mut core, sup, "sluzba fsd", None, 0, 11);
    apply(&mut lu, &out, u);
    apply(&mut la, &out, a);
    // Uzivatel vidi jen svou, spravce obe; poradi bez der i u uzivatele.
    assert_eq!(lu.tasks.iter().map(|t| t.id).collect::<Vec<_>>(), vec![t1]);
    assert_eq!(la.tasks.len(), 2);
    let out = core.producer_msg(sup, &Produce::Update { id: t2, state: State::Running, progress: Some(40), eta_ms: None }.encode(), 12);
    apply(&mut lu, &out, u);
    apply(&mut la, &out, a);
    assert_eq!(lu.tasks.len(), 1);
    assert_eq!(la.tasks.iter().find(|t| t.id == t2).unwrap().progress, Some(40));
    // Cizi producent ulohu nezmeni.
    let out = core.producer_msg(sup, &Produce::Finish { id: t1, state: State::Cancelled }.encode(), 13);
    assert!(out.is_empty());
    assert_eq!(core.running(), (2, 0));
}

#[test]
fn actions_follow_scope_and_go_to_the_producer() {
    let mut core = Core::new();
    let shell = core.add_producer("uzivatel");
    let sup = core.add_producer("system");
    let u = core.add_client(user());
    let s = core.add_client(summary());
    let (t1, _) = register(&mut core, shell, "uloha", None, actions::CANCEL | actions::LOG, 1);
    let (t2, _) = register(&mut core, sup, "sluzba", None, actions::CANCEL, 1);
    // Vlastni: zruseni jde producentovi.
    let out = core.client_msg(u, &Request::Cancel { req: 3, id: t1 }.encode(), 2);
    assert_eq!(to_client(&out, u), vec![Reply::Result { req: 3, result: Ok(()) }]);
    assert_eq!(Control::decode(&to_producer(&out, shell)[0]).unwrap(), Control::Cancel(t1));
    // Cizi: jako neexistujici.
    let out = core.client_msg(u, &Request::Cancel { req: 4, id: t2 }.encode(), 2);
    assert_eq!(to_client(&out, u), vec![Reply::Result { req: 4, result: Err(Error::NotFound) }]);
    // Producent pozastaveni neumi.
    let out = core.client_msg(u, &Request::Pause { req: 5, id: t1 }.encode(), 2);
    assert_eq!(to_client(&out, u), vec![Reply::Result { req: 5, result: Err(Error::NotSupported) }]);
    // Shrnuti (kompozitor) vidi vse, ale neridi.
    let out = core.client_msg(s, &Request::Cancel { req: 6, id: t2 }.encode(), 2);
    assert_eq!(to_client(&out, s), vec![Reply::Result { req: 6, result: Err(Error::AccessDenied) }]);
    let snap = to_client(&core.client_msg(s, &Request::Subscribe { req: 1, since: None }.encode(), 2), s);
    let Reply::Snapshot { tasks, .. } = &snap[0] else { panic!() };
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|t| t.actions & actions::CANCEL == 0));
    // Hotovou ulohu zrusit nejde.
    core.producer_msg(shell, &Produce::Finish { id: t1, state: State::Done("ok".into()) }.encode(), 3);
    let out = core.client_msg(u, &Request::Cancel { req: 7, id: t1 }.encode(), 3);
    assert_eq!(to_client(&out, u), vec![Reply::Result { req: 7, result: Err(Error::BadState) }]);
    // Neplatna zprava: chyba, ne panika.
    let out = core.client_msg(u, &[1, 2, 3], 3);
    assert_eq!(to_client(&out, u), vec![Reply::Result { req: 0, result: Err(Error::InvalidArgs) }]);
}

#[test]
fn resubscribe_from_history_or_snapshot() {
    let mut core = Core::new();
    let shell = core.add_producer("uzivatel");
    let u = core.add_client(user());
    let mut l = TaskList::default();
    subscribe(&mut core, u, &mut l);
    let (t, out) = register(&mut core, shell, "a", None, 0, 1);
    apply(&mut l, &out, u);
    let seen = l.seq;
    // Klient vypadne; mezitim zmeny.
    core.remove_client(u);
    core.producer_msg(shell, &Produce::Update { id: t, state: State::Running, progress: Some(50), eta_ms: Some(1000) }.encode(), 2);
    core.producer_msg(shell, &Produce::Finish { id: t, state: State::Done("hotovo".into()) }.encode(), 3);
    let u2 = core.add_client(user());
    let out = core.client_msg(u2, &Request::Subscribe { req: 9, since: Some(seen) }.encode(), 4);
    apply(&mut l, &out, u2);
    let task = l.tasks.iter().find(|x| x.id == t).unwrap();
    assert_eq!(task.state, State::Done("hotovo".into()));
    assert_eq!(task.progress, Some(100));
    assert_eq!(task.ended, Some(3));
    // Prilis stare `since`: snimek.
    for i in 0..(HISTORY as u64 + 5) {
        register(&mut core, shell, "x", None, 0, 10 + i);
    }
    let out = core.client_msg(u2, &Request::Subscribe { req: 10, since: Some(seen) }.encode(), 9999);
    assert!(matches!(to_client(&out, u2)[0], Reply::Snapshot { req: 10, .. }));
}

#[test]
fn finished_tasks_are_kept_for_a_while_then_dropped() {
    let mut core = Core::new();
    let shell = core.add_producer("uzivatel");
    let a = core.add_client(admin());
    for i in 0..(KEEP_FINISHED as u64 + 20) {
        let (t, _) = register(&mut core, shell, "kratka", None, 0, i);
        core.producer_msg(shell, &Produce::Finish { id: t, state: State::Done(String::new()) }.encode(), i);
    }
    let snap = to_client(&core.client_msg(a, &Request::Subscribe { req: 1, since: None }.encode(), 200), a);
    let n: usize = snap.iter().map(|r| if let Reply::Snapshot { tasks, .. } = r { tasks.len() } else { 0 }).sum();
    assert_eq!(n, KEEP_FINISHED);
    // Po 30 minutach zmizi vsechny hotove; bezici zustane.
    let (live, _) = register(&mut core, shell, "dlouha", None, 0, 300);
    core.tick(300 + KEEP_FINISHED_NS + 1_000);
    let snap = to_client(&core.client_msg(a, &Request::Subscribe { req: 2, since: None }.encode(), 0), a);
    let Reply::Snapshot { tasks, .. } = &snap[0] else { panic!() };
    assert_eq!(tasks.iter().map(|t| t.id).collect::<Vec<_>>(), vec![live]);
}

#[test]
fn producer_gone_fails_its_tasks_and_log_lines() {
    let mut core = Core::new();
    let shell = core.add_producer("uzivatel");
    let u = core.add_client(user());
    let (t, _) = register(&mut core, shell, "a", None, actions::LOG, 1);
    for i in 0..(KEEP_LOG + 5) {
        core.producer_msg(shell, &Produce::LogLine { id: t, line: format!("radek {i}") }.encode(), 1);
    }
    let out = core.client_msg(u, &Request::Log { req: 2, id: t, lines: 3 }.encode(), 2);
    let r = to_client(&out, u);
    let Reply::LogLines { lines, .. } = &r[0] else { panic!("{r:?}") };
    assert_eq!(lines, &vec![format!("radek {}", KEEP_LOG + 2), format!("radek {}", KEEP_LOG + 3), format!("radek {}", KEEP_LOG + 4)]);
    core.remove_producer(shell, 5);
    let snap = to_client(&core.client_msg(u, &Request::Subscribe { req: 1, since: None }.encode(), 6), u);
    let Reply::Snapshot { tasks, .. } = &snap[0] else { panic!() };
    assert!(matches!(tasks[0].state, State::Failed { .. }));
    assert_eq!(tasks[0].ended, Some(5));
}

fn proc(pid: u64, job: u64, name: &str, cpu: u64, finished: bool) -> ProcEntry {
    ProcEntry {
        pid,
        job,
        cpu_ns: cpu,
        threads: 1,
        flags: if finished { proc_flags::FINISHED } else { 0 },
        name: ProcEntry::name_bytes(name),
    }
}

#[test]
fn unknown_process_in_job_appears_and_ends() {
    let mut core = Core::new();
    let init = core.add_producer("system");
    let a = core.add_client(admin());
    let u = core.add_client(user());
    let mut la = TaskList::default();
    let mut lu = TaskList::default();
    subscribe(&mut core, a, &mut la);
    subscribe(&mut core, u, &mut lu);
    // Aplikace v jobu 5 je registrovana: jeji procesy k ni patri (cas CPU, pamet).
    let (app, out) = register(&mut core, init, "Terminal", Some(5), 0, 1);
    apply(&mut la, &out, a);
    apply(&mut lu, &out, u);
    let procs = vec![proc(10, 5, "gui-term", 300, false), proc(11, 5, "pomocnik", 200, false), proc(20, 9, "skryty", 50, false)];
    let out = core.processes(&procs, &[(5, 4096)], 2);
    apply(&mut la, &out, a);
    apply(&mut lu, &out, u);
    let t = la.tasks.iter().find(|t| t.id == app).unwrap();
    assert_eq!((t.cpu_ns, t.memory), (Some(500), Some(4096)));
    // Proces v jobu 9 nikdo nehlasi: neznamy (vidi ho jen spravce).
    let unk = la.tasks.iter().find(|t| t.origin == Origin::Unknown).expect("neznamy proces chybi");
    assert!(unk.name.contains("skryty") && unk.name.contains("jobu 9"), "{}", unk.name);
    assert_eq!(unk.actions, 0);
    assert!(lu.tasks.iter().all(|t| t.origin != Origin::Unknown));
    // Opakovany vycet: zadna nova uloha, jen cas.
    let out = core.processes(&[proc(10, 5, "gui-term", 300, false), proc(20, 9, "skryty", 80, false)], &[], 3);
    apply(&mut la, &out, a);
    assert_eq!(la.tasks.iter().filter(|t| t.origin == Origin::Unknown).count(), 1);
    // Proces skoncil: uloha je hotova.
    let out = core.processes(&[proc(10, 5, "gui-term", 300, false), proc(20, 9, "skryty", 80, true)], &[], 4);
    apply(&mut la, &out, a);
    let unk = la.tasks.iter().find(|t| t.origin == Origin::Unknown).unwrap();
    assert!(unk.state.finished());
    // Aplikace v jobu 5 skoncila (zadny zivy proces ve sledovanem jobu).
    let out = core.processes(&[proc(10, 5, "gui-term", 300, true)], &[(5, 0), (9, 0)], 5);
    apply(&mut la, &out, a);
    assert!(la.tasks.iter().find(|t| t.id == app).unwrap().state.finished());
    // Producent se nesmi vydavat za neznamy proces.
    let m = Produce::Register { req: 1, parent: 0, name: "x".into(), origin: Origin::Unknown, who: "x".into(), agent: false, job: None, attach: None, actions: 0 };
    let out = core.producer_msg(init, &m.encode(), 5);
    apply(&mut la, &out, a);
    let (_, id) = taskproto::decode_registered(&to_producer(&out, init)[0]).unwrap();
    assert_eq!(la.tasks.iter().find(|t| t.id == id).unwrap().origin, Origin::App);
}
