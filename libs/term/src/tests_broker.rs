//! Testy prostrednika termd, ridicich zprav, registru a pripojeni
//! programu v nahledu.

use shlang::host::ExecNeeds;

use crate::broker::{Broker, Out, id_base, owner};
use crate::ctl::{RegMsg, Registry, SessMsg};
use crate::perm::{MountSpec, mounts};
use crate::proto::{ClientMsg, ServerMsg};

#[test]
fn control_messages_roundtrip() {
    for m in [
        SessMsg::Init { session: 7, tag: "ab-7".into(), home: "/home".into() },
        SessMsg::Adopt,
        SessMsg::Client(ClientMsg::Run { session: 7, line: "ls".into() }),
    ] {
        assert_eq!(SessMsg::decode(&m.encode()), Ok(m));
    }
    for m in [
        RegMsg::Park { session: 3, tag: "x-3".into() },
        RegMsg::Unpark { session: 3 },
        RegMsg::Claim,
        RegMsg::Parked { sessions: vec![(1, "a".into()), (2, "b".into())] },
    ] {
        assert_eq!(RegMsg::decode(&m.encode()), Ok(m));
    }
    assert!(SessMsg::decode(&[]).is_err());
    assert!(RegMsg::decode(&[9]).is_err());
    // Bomba: obri pocet relaci bez dat.
    let mut bomb = vec![4u8];
    bomb.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(RegMsg::decode(&bomb).is_err());
}

#[test]
fn registry_keeps_spare_ends_until_claimed() {
    let mut r: Registry<&str> = Registry::default();
    assert!(r.handle(RegMsg::Park { session: 1, tag: "t-1".into() }, vec!["h1"]).is_none());
    r.handle(RegMsg::Park { session: 2, tag: "t-2".into() }, vec!["h2"]);
    // Novy konec tehoz cisla nahradi stary.
    r.handle(RegMsg::Park { session: 1, tag: "t-1".into() }, vec!["h1b"]);
    // Park bez handlu nic neulozi.
    r.handle(RegMsg::Park { session: 9, tag: "t-9".into() }, vec![]);
    r.handle(RegMsg::Unpark { session: 2 }, vec![]);
    let (reply, hs) = r.handle(RegMsg::Claim, vec![]).unwrap();
    assert_eq!(reply, RegMsg::Parked { sessions: vec![(1, "t-1".into())] });
    assert_eq!(hs, vec!["h1b"]);
    assert!(r.is_empty(), "vydane konce registr uz nedrzi");
}

#[test]
fn broker_routes_by_session_and_prompt() {
    let mut b = Broker::new(0xab);
    let out = b.from_window(ClientMsg::Attach { session: 0 });
    assert_eq!(
        out,
        vec![Out::Spawn { session: 1, tag: "ab-1".into() }, Out::Session(1, ClientMsg::Attach { session: 1 })]
    );
    let run = ClientMsg::Run { session: 1, line: "ls".into() };
    assert_eq!(b.from_window(run.clone()), vec![Out::Session(1, run)]);
    // Odpoved na vyzvu jde relaci, ktera vyzvu poslala.
    let prompt = id_base(1) + 4;
    assert_eq!(owner(prompt), 1);
    let ans = ClientMsg::Answer { prompt, yes: true };
    assert_eq!(b.from_window(ans.clone()), vec![Out::Session(1, ans)]);
    // Neznama relace: Fail oknu.
    assert!(matches!(&b.from_window(ClientMsg::Interrupt { session: 5 })[..], [Out::Window(ServerMsg::Fail { .. })]));
    // Stav pro Sessions bez dotazu na relaci.
    b.from_session(1, ServerMsg::Attached { session: 1, cwd: "/home".into(), blocks: 0 });
    b.from_session(1, ServerMsg::BlockStart { session: 1, block: id_base(1), line: "ls".into() });
    assert_eq!(
        b.from_window(ClientMsg::Sessions),
        vec![Out::Window(ServerMsg::SessionList { sessions: vec![(1, 1, "/home".into())] })]
    );
    // Okno pryc: relace se odpoji, ale bezi dal.
    assert_eq!(b.window_closed(), vec![Out::Session(1, ClientMsg::Detach { session: 1 })]);
    assert_eq!(b.routes.len(), 1);
    // Relace spadla: odpojena = bez zpravy, cesta zmizi.
    assert_eq!(b.session_ended(1), None);
    assert!(b.routes.is_empty());
}

#[test]
fn adopted_sessions_keep_numbers() {
    let mut b = Broker::new(1);
    b.adopt(4, "old-4".into());
    let out = b.from_window(ClientMsg::Attach { session: 0 });
    assert!(matches!(&out[0], Out::Spawn { session: 5, .. }), "nova relace za prevzatymi");
    assert_eq!(b.from_window(ClientMsg::Attach { session: 4 }), vec![Out::Session(4, ClientMsg::Attach { session: 4 })]);
}

#[test]
fn preview_program_mounts_come_from_clones() {
    let spec = |n: &str, t: &str, ro| MountSpec { name: n.into(), target: t.into(), read_only: ro };
    // Omezeny program: zapis do klonu, zapis mimo klony jen pro cteni.
    let n = ExecNeeds {
        read: vec!["/data".into()],
        write: vec!["/home/out".into(), "/srv".into()],
        aside: vec![("/home".into(), "/tmp/.aside/x-2".into()), ("/".into(), "/tmp/.aside/x-1".into())],
        ..Default::default()
    };
    assert_eq!(
        mounts(&n),
        vec![
            spec("/data", "/tmp/.aside/x-1/data", true),
            spec("/home/out", "/tmp/.aside/x-2/out", false),
            spec("/srv", "/tmp/.aside/x-1/srv", false),
        ]
    );
    // Plna prava: koren z klonu a vnoreny dataset s vlastnim klonem.
    let all = ExecNeeds { all: true, aside: n.aside.clone(), ..Default::default() };
    assert_eq!(
        mounts(&all),
        vec![spec("/", "/tmp/.aside/x-1", false), spec("/home", "/tmp/.aside/x-2", false)]
    );
    // Bez klonu pro zapisovanou cestu: jen cteni.
    let only_home = ExecNeeds {
        write: vec!["/srv".into()],
        aside: vec![("/home".into(), "/tmp/.aside/x-2".into())],
        ..Default::default()
    };
    assert_eq!(mounts(&only_home), vec![spec("/srv", "/srv", true)]);
}

#[test]
fn single_session_service_uses_its_number() {
    use crate::session::Service;
    let mut svc = Service::single("/home", 3);
    let mut link = super::tests::TestLink::default();
    let mut f = shlang::vfs::mem::MemFs::new();
    f.put("/home/a.txt", b"a");
    let mut env = super::tests::TestEnv::default();
    svc.handle(ClientMsg::Attach { session: 3 }, &mut link, &mut f, &mut env);
    assert!(matches!(link.sent.last(), Some(ServerMsg::Attached { session: 3, .. })));
    svc.handle(ClientMsg::Run { session: 3, line: "echo x".into() }, &mut link, &mut f, &mut env);
    let block = link.sent.iter().find_map(|m| match m {
        ServerMsg::BlockStart { block, .. } => Some(*block),
        _ => None,
    });
    assert_eq!(block.map(owner), Some(3), "cisla bloku v rozsahu relace");
}
