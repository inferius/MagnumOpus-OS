//! Testy protokolu uloh (na hostiteli: `cargo test -p taskproto`).

use crate::*;

pub fn sample(id: u64, agent: bool) -> Task {
    Task {
        id,
        parent: 0,
        name: "Kopírování 3 souborů".into(),
        origin: if agent { Origin::Agent } else { Origin::User },
        who: if agent { "asistent".into() } else { "michal".into() },
        agent,
        state: State::Running,
        progress: Some(40),
        eta_ms: Some(12_000),
        memory: Some(8 << 20),
        cpu_ns: None,
        started: 1_700_000_000_000_000_000,
        ended: None,
        attach: Some((3, 17)),
        actions: actions::CANCEL | actions::PAUSE | actions::LOG,
    }
}

fn all_bad_prefixes(enc: &[u8], dec: impl Fn(&[u8]) -> bool) {
    for n in 0..enc.len() {
        assert!(dec(&enc[..n]), "prefix {n} mel byt chyba");
    }
}

#[test]
fn requests_roundtrip() {
    for m in [
        Request::Hello { version: 1 },
        Request::Subscribe { req: 1, since: None },
        Request::Subscribe { req: 2, since: Some(99) },
        Request::Unsubscribe { req: 3 },
        Request::Cancel { req: 4, id: 7 },
        Request::Pause { req: 5, id: 7 },
        Request::Resume { req: 6, id: 7 },
        Request::Log { req: 7, id: 7, lines: 50 },
    ] {
        let e = m.encode();
        assert_eq!(Request::decode(&e), Ok(m));
        all_bad_prefixes(&e, |d| Request::decode(d).is_err());
    }
    // Log se omezi.
    let e = Request::Log { req: 1, id: 1, lines: u32::MAX }.encode();
    assert_eq!(Request::decode(&e), Ok(Request::Log { req: 1, id: 1, lines: MAX_LOG_LINES }));
}

#[test]
fn replies_roundtrip_all_states() {
    let mut t = sample(1, true);
    for s in [
        State::Running,
        State::Paused,
        State::Waiting("Přepsat soubor? [a/n]".into()),
        State::Cancelling,
        State::Done("3 soubory".into()),
        State::Failed { code: -8, message: "nenalezeno".into() },
        State::Cancelled,
    ] {
        t.state = s;
        t.progress = None;
        t.ended = Some(5);
        let m = Reply::Event { seq: 3, change: Change::Updated(t.clone()) };
        let e = m.encode();
        assert_eq!(Reply::decode(&e), Ok(m));
        all_bad_prefixes(&e, |d| Reply::decode(d).is_err());
    }
    for m in [
        Reply::Snapshot { req: 1, seq: 10, tasks: vec![sample(1, false), sample(2, true)], last: true },
        Reply::Event { seq: 11, change: Change::Added(sample(3, false)) },
        Reply::Event { seq: 12, change: Change::Removed(3) },
        Reply::Result { req: 2, result: Ok(()) },
        Reply::Result { req: 3, result: Err(Error::AccessDenied) },
        Reply::LogLines { req: 4, lines: vec!["řádek 1".into(), "řádek 2".into()] },
    ] {
        let e = m.encode();
        assert_eq!(Reply::decode(&e), Ok(m));
    }
}

#[test]
fn produce_and_control_roundtrip() {
    for m in [
        Produce::Register {
            req: 1,
            parent: 0,
            name: "scrub".into(),
            origin: Origin::Service,
            who: "fsd".into(),
            agent: false,
            job: Some(4),
            attach: None,
            actions: actions::CANCEL,
        },
        Produce::Update { id: 9, state: State::Running, progress: Some(55), eta_ms: None },
        Produce::Finish { id: 9, state: State::Done(String::new()) },
        Produce::LogLine { id: 9, line: "radek vystupu".into() },
    ] {
        let e = m.encode();
        assert_eq!(Produce::decode(&e), Ok(m));
    }
    assert_eq!(decode_registered(&encode_registered(1, 42)), Ok((1, 42)));
    for c in [Control::Cancel(1), Control::Pause(2), Control::Resume(3)] {
        assert_eq!(Control::decode(&c.encode()), Ok(c));
    }
}

#[test]
fn bad_messages_never_panic() {
    assert_eq!(Request::decode(&999u32.to_le_bytes()), Err(Error::NotSupported));
    // Obri pocet uloh v kratke zprave.
    let mut w = wire::Writer::op(op::SNAPSHOT);
    w.u32(1).u64(1).u8(1).u32(u32::MAX);
    assert!(Reply::decode(&w.finish()).is_err());
    // Neplatny stav, puvod, bool.
    let mut e = Reply::Event { seq: 1, change: Change::Added(sample(1, false)) }.encode();
    let pos = e.len() - 4 - 1 - 16 - 1;
    e[pos] = 9;
    assert!(Reply::decode(&e).is_err());
    let mut x = 7u64;
    for _ in 0..2000 {
        let n = (x % 120) as usize;
        let d: Vec<u8> = (0..n).map(|i| (x.wrapping_mul(i as u64 + 31) >> 13) as u8).collect();
        let _ = Reply::decode(&d);
        let _ = Request::decode(&d);
        let _ = Produce::decode(&d);
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
}

#[test]
fn task_list_snapshot_events_and_gaps() {
    let mut l = TaskList::default();
    let snap1 = Reply::Snapshot { req: 1, seq: 5, tasks: vec![sample(1, false)], last: false };
    let snap2 = Reply::Snapshot { req: 1, seq: 5, tasks: vec![sample(2, true)], last: true };
    assert_eq!(l.apply(&snap1), Applied::Ok);
    assert_eq!(l.apply(&snap2), Applied::Ok);
    assert_eq!(l.tasks.len(), 2, "snimek z vic casti");
    assert_eq!(l.running(), (2, 1));
    let mut done = sample(1, false);
    done.state = State::Done("ok".into());
    assert_eq!(l.apply(&Reply::Event { seq: 6, change: Change::Updated(done) }), Applied::Ok);
    assert_eq!(l.running(), (1, 1));
    // Stara udalost ignorovana, mezera hlasena.
    assert_eq!(l.apply(&Reply::Event { seq: 6, change: Change::Removed(1) }), Applied::Ignored);
    assert_eq!(l.apply(&Reply::Event { seq: 9, change: Change::Removed(1) }), Applied::Gap);
    assert_eq!(l.apply(&Reply::Event { seq: 7, change: Change::Removed(1) }), Applied::Ok);
    assert_eq!(l.tasks.len(), 1);
    // Novy snimek nahradi vse.
    assert_eq!(l.apply(&Reply::Snapshot { req: 2, seq: 20, tasks: vec![], last: true }), Applied::Ok);
    assert!(l.tasks.is_empty());
    assert_eq!(l.seq, 20);
}
