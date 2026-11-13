//! Relace okna (karty): prehrani po davkach (nejdriv viditelna cast,
//! starsi bloky se vlozi pred novejsi), prepnuti na jinou relaci, seznam.

use crate::proto::{ClientMsg, ServerMsg};
use crate::session::Service;
use crate::tests::{TestEnv, TestLink};
use crate::theme::Theme;
use crate::ui::App;

fn start(a: &mut App, id: u64, line: &str) {
    a.on_msg(ServerMsg::BlockStart { session: 1, block: id, line: line.into() });
    a.on_msg(ServerMsg::BlockEnd { session: 1, block: id, ok: true, duration: 1 });
}

#[test]
fn replay_puts_older_blocks_before_newer() {
    let mut a = App::new(Theme::dark(), 80, 30);
    // Viditelna cast (posledni bloky), pak Attached, pak starsi po davkach
    // od nejnovejsich.
    for id in 20..32 {
        start(&mut a, id, &format!("b{id}"));
    }
    a.on_msg(ServerMsg::Attached { session: 1, cwd: "/".into(), blocks: 31 });
    for chunk in (1..20u64).collect::<Vec<_>>().rchunks(16) {
        for &id in chunk {
            start(&mut a, id, &format!("b{id}"));
        }
    }
    let ids: Vec<u64> = a.blocks.iter().map(|b| b.id).collect();
    assert_eq!(ids, (1..32).collect::<Vec<u64>>());
    // Novy blok jde na konec.
    start(&mut a, 40, "novy");
    assert_eq!(a.blocks.last().unwrap().id, 40);
}

#[test]
fn replay_drops_oldest_when_window_is_full() {
    let mut a = App::new(Theme::dark(), 80, 30);
    for id in 1000..1100 {
        start(&mut a, id, "x");
    }
    // Okno drzi 100 bloku: starsi prehrany blok se uz nevejde.
    start(&mut a, 5, "stary");
    assert_eq!(a.blocks.len(), 100);
    assert_eq!(a.blocks[0].id, 1000);
}

#[test]
fn switch_detaches_and_attaches_other_session() {
    let mut a = App::new(Theme::dark(), 80, 30);
    a.on_msg(ServerMsg::Attached { session: 1, cwd: "/".into(), blocks: 0 });
    start(&mut a, 3, "ls");
    a.on_msg(ServerMsg::SessionList { sessions: vec![(2, 4, "/tmp".into()), (1, 1, "/".into())] });
    let tabs: Vec<u64> = a.session_tabs().iter().map(|s| s.id).collect();
    assert_eq!(tabs, vec![1, 2]);
    a.take_out();
    a.switch_session(2);
    assert_eq!(
        a.take_out(),
        vec![ClientMsg::Detach { session: 1 }, ClientMsg::Attach { session: 2 }, ClientMsg::Sessions]
    );
    assert!(a.blocks.is_empty(), "bloky predchozi relace okno zapomene");
    // Stejna relace = nic; nova relace = Attach 0.
    a.switch_session(2);
    assert!(a.take_out().is_empty());
    a.switch_session(0);
    assert_eq!(a.take_out()[1], ClientMsg::Attach { session: 0 });
}

#[test]
fn service_replays_visible_blocks_first() {
    let mut svc = Service::new("/home");
    let (mut link, mut f, mut env) = (TestLink::default(), shlang::vfs::mem::MemFs::new(), TestEnv::default());
    svc.handle(ClientMsg::Attach { session: 0 }, &mut link, &mut f, &mut env);
    let id = svc.sessions[0].id;
    for i in 0..30 {
        svc.handle(ClientMsg::Run { session: id, line: format!("echo {i}") }, &mut link, &mut f, &mut env);
    }
    svc.handle(ClientMsg::Detach { session: id }, &mut link, &mut f, &mut env);
    let mut l2 = TestLink::default();
    svc.handle(ClientMsg::Attach { session: id }, &mut l2, &mut f, &mut env);
    let attached = l2.sent.iter().position(|m| matches!(m, ServerMsg::Attached { .. })).expect("Attached");
    let before = l2.sent[..attached].iter().filter(|m| matches!(m, ServerMsg::BlockStart { .. })).count();
    let after: Vec<u64> = l2.sent[attached..]
        .iter()
        .filter_map(|m| match m {
            ServerMsg::BlockStart { block, .. } => Some(*block),
            _ => None,
        })
        .collect();
    assert_eq!(before, 12, "viditelna cast pred Attached");
    assert_eq!(after.len(), 18);
    // Prvni davka po Attached jsou nejnovejsi ze starsich bloku.
    assert!(after[0] > after[after.len() - 1], "{after:?}");
    // Okno z toho slozi bloky ve spravnem poradi.
    let mut a = App::new(Theme::dark(), 80, 30);
    for m in l2.sent {
        a.on_msg(m);
    }
    let ids: Vec<u64> = a.blocks.iter().map(|b| b.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
    assert_eq!(ids.len(), 30);
}
