//! Limity na identitu (aplikaci), ne na spojeni: procesy, ktere dedi
//! relaci okna (role `Gui`, napr. Wine), je neobejdou. Odvolani GUI
//! identity najde vsechna jeji spojeni.

use super::*;
use crate::scene::{MAX_IDENTITY_CLIENTS, MAX_WINDOWS};

fn create(t: &mut T, cl: ClientId, window: u32) {
    t.msg(cl, ClientMsg::WindowCreate { window, mode: Mode::Standard, w: 100, h: 80, title: "Okno".into() });
}

fn no_resources(out: &[Out], cl: ClientId, op: u32) -> bool {
    client_msgs(out, cl).iter().any(|m| matches!(m, ServerMsg::Error { op: o, code: guiproto::Error::NoResources } if *o == op))
}

#[test]
fn window_limit_counts_all_connections_of_identity() {
    let mut t = T::new();
    // Dve spojeni stejne identity (rodic a dite s roli Gui).
    let a = t.client("wine");
    let b = t.client("wine");
    let half = MAX_WINDOWS / 2;
    for w in 1..=half as u32 {
        create(&mut t, a, w);
    }
    for w in 1..=(MAX_WINDOWS - half) as u32 {
        create(&mut t, b, w);
    }
    t.out();
    assert_eq!(t.c.identity_usage("wine").windows, MAX_WINDOWS);
    // Dalsi okno uz nedostane zadne spojeni identity.
    create(&mut t, a, 100);
    create(&mut t, b, 100);
    let out = t.out();
    assert!(no_resources(&out, a, guiproto::window::op::WINDOW_CREATE));
    assert!(no_resources(&out, b, guiproto::window::op::WINDOW_CREATE));
    // Jina identita ma svuj vlastni limit.
    let c = t.client("jina");
    create(&mut t, c, 1);
    let out = t.out();
    assert!(!client_msgs(&out, c).iter().any(|m| matches!(m, ServerMsg::Error { .. })));
}

#[test]
fn identity_client_limit_and_revoke_list() {
    let mut t = T::new();
    let mut ids = Vec::new();
    for _ in 0..MAX_IDENTITY_CLIENTS {
        assert!(t.c.can_add_client(&info("wine", "wine")));
        ids.push(t.client("wine"));
    }
    assert!(!t.c.can_add_client(&info("wine", "wine")), "fork-bomba spojeni neprojde");
    assert!(t.c.can_add_client(&info("jina", "jina")));
    let other = t.client("jina");
    let mut of = t.c.clients_of("wine");
    of.sort();
    assert_eq!(of, ids);
    assert_eq!(t.c.clients_of("jina"), vec![other]);
    // Po zavreni spojeni (odvolani) misto zase je.
    for id in ids {
        t.c.remove_client(id);
    }
    assert_eq!(t.c.identity_usage("wine").clients, 0);
    assert!(t.c.can_add_client(&info("wine", "wine")));
}
