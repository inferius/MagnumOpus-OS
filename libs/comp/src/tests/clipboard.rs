//! Schranka: jen na fyzicke gesto okna s fokusem, liny prenos, kopie po
//! skonceni vlastnika, odmitnuti ze zabezpeceneho pole.

use alloc::vec;
use alloc::vec::Vec;

use clipproto::{ClientMsg as CMsg, Denied, ServerMsg as SMsg, TEXT};

use super::*;

fn clip(t: &mut T, c: ClientId, m: CMsg) {
    t.msg(c, ClientMsg::Clipboard { data: m.encode() });
}

/// Zpravy schranky pro klienta `c` z vystupu.
fn got(out: &[Out], c: ClientId) -> Vec<SMsg> {
    client_msgs(out, c)
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Clipboard { data } => Some(SMsg::decode(&data).unwrap()),
            _ => None,
        })
        .collect()
}

/// Fyzicky klik do okna klienta (fokus a gesto).
fn press(t: &mut T, c: ClientId) {
    let r = t.rect_of(c);
    t.click(r.x + 20, r.y + 60);
}

#[test]
fn copy_and_paste_need_gesture_and_focus() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    // Nabidka bez gesta: odmitnuta (zadna zadost o kopii).
    clip(&mut t, a, CMsg::Offer { formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty());
    // Klik do a: fokus a gesto; nabidka projde, kompozitor si hned vyzada
    // text pro kopii.
    press(&mut t, a);
    t.out();
    clip(&mut t, a, CMsg::Offer { formats: vec![TEXT.into()] });
    let out = t.out();
    let m = got(&out, a);
    let SMsg::Send { serial: keep, ref format } = m[0] else { panic!("{m:?}") };
    assert_eq!(format, TEXT);
    assert!(m.contains(&SMsg::Changed { formats: vec![TEXT.into()] }));
    clip(&mut t, a, CMsg::Data { serial: keep, data: "Čaj".as_bytes().to_vec() });
    // b bez fokusu nic neprecte.
    clip(&mut t, b, CMsg::Request { serial: 5, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 5, why: Denied::NoGesture }]);
    // Klik do b: vlozeni jde line pres vlastnika a.
    press(&mut t, b);
    let out = t.out();
    assert!(got(&out, b).contains(&SMsg::Changed { formats: vec![TEXT.into()] }), "nove okno s fokusem vi o obsahu");
    clip(&mut t, b, CMsg::Request { serial: 6, format: TEXT.into() });
    let m = got(&t.out(), a);
    let SMsg::Send { serial: s, .. } = m[0] else { panic!("{m:?}") };
    clip(&mut t, a, CMsg::Data { serial: s, data: b"novy".to_vec() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Deliver { serial: 6, format: TEXT.into(), data: b"novy".to_vec() }]);
    // Jedno gesto = jedno vlozeni.
    clip(&mut t, b, CMsg::Request { serial: 7, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 7, why: Denied::NoGesture }]);
    // Gesto vyprsi.
    press(&mut t, b);
    t.out();
    t.now += crate::clipboard::GESTURE + 1;
    clip(&mut t, b, CMsg::Request { serial: 8, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 8, why: Denied::NoGesture }]);
    // Cizi klient nemuze podstrcit data za vlastnika.
    clip(&mut t, b, CMsg::Data { serial: s, data: b"x".to_vec() });
    assert!(t.out().is_empty());
}

/// Vlastnik skonci: zustane kopie textu; cekajici vlozeni selze.
#[test]
fn content_survives_owner_and_pending_paste_fails() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    press(&mut t, a);
    clip(&mut t, a, CMsg::Offer { formats: vec![TEXT.into(), "text/html".into()] });
    let m = got(&t.out(), a);
    let SMsg::Send { serial: keep, .. } = m[0] else { panic!() };
    clip(&mut t, a, CMsg::Data { serial: keep, data: b"kopie".to_vec() });
    // b pozada o HTML, a skonci driv, nez odpovi.
    press(&mut t, b);
    t.out();
    clip(&mut t, b, CMsg::Request { serial: 1, format: "text/html".into() });
    t.out();
    t.c.remove_client(a);
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 1, why: Denied::Unavailable }]);
    // Text zustal (kopie), HTML ne.
    press(&mut t, b);
    t.out();
    clip(&mut t, b, CMsg::Request { serial: 2, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Deliver { serial: 2, format: TEXT.into(), data: b"kopie".to_vec() }]);
    press(&mut t, b);
    t.out();
    clip(&mut t, b, CMsg::Request { serial: 3, format: "text/html".into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 3, why: Denied::Empty }]);
}

/// Umely vstup gesto neda; nabidka z fokusu v poli s heslem se odmitne.
#[test]
fn synthetic_input_and_secure_field_refused() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.key(inputproto::code::KEY_C, true);
    t.out();
    clip(&mut t, a, CMsg::Offer { formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "umely vstup neni gesto");
    // Fokus v poli s heslem.
    let mut n = Node::new(2, 0, Role::PasswordInput, "Heslo");
    n.state = guiproto::a11y::state::FOCUSED;
    t.msg(a, ClientMsg::A11yUpdate { window: 1, nodes: vec![Node::new(1, 0, Role::Window, "Okno"), n] });
    t.commit(a, 2, 0, 0, 0);
    press(&mut t, a);
    t.out();
    clip(&mut t, a, CMsg::Offer { formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "heslo do schranky ne");
    assert!(t.c.take_trace().iter().any(|l| l.contains("zabezpecenem poli")));
    // Vadna zprava: chyba, ne panika.
    t.msg(a, ClientMsg::Clipboard { data: vec![9, 9] });
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { .. })));
}
