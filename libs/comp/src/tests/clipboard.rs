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

/// Cislo gesta posledniho fyzickeho stisku doruceneho klientovi `c`.
fn serial_in(out: &[Out], c: ClientId) -> u32 {
    client_msgs(out, c)
        .iter()
        .rev()
        .find_map(|m| match m {
            ServerMsg::Input { records, .. } => records.iter().rev().find(|r| r.serial != 0).map(|r| r.serial),
            _ => None,
        })
        .unwrap_or(0)
}

/// Fyzicky klik do okna klienta (fokus a gesto): cislo gesta a vystup.
fn press(t: &mut T, c: ClientId) -> (u32, Vec<Out>) {
    let r = t.rect_of(c);
    t.click(r.x + 20, r.y + 60);
    let out = t.out();
    (serial_in(&out, c), out)
}

#[test]
fn copy_and_paste_need_gesture_and_focus() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    // Nabidka bez gesta: odmitnuta (zadna zadost o kopii).
    clip(&mut t, a, CMsg::Offer { gesture: 0, formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty());
    // Klik do a: fokus a gesto; nabidka s jeho cislem projde, kompozitor
    // si hned vyzada text pro kopii.
    let (g, _) = press(&mut t, a);
    assert_ne!(g, 0, "fyzicky stisk nese cislo gesta");
    clip(&mut t, a, CMsg::Offer { gesture: g, formats: vec![TEXT.into()] });
    let out = t.out();
    let m = got(&out, a);
    let SMsg::Send { serial: keep, ref format } = m[0] else { panic!("{m:?}") };
    assert_eq!(format, TEXT);
    assert!(m.contains(&SMsg::Changed { formats: vec![TEXT.into()] }));
    clip(&mut t, a, CMsg::Data { serial: keep, data: "Čaj".as_bytes().to_vec() });
    // b bez fokusu nic neprecte (ani s cislem okna a).
    clip(&mut t, b, CMsg::Request { gesture: g, serial: 5, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 5, why: Denied::NoGesture }]);
    // Klik do b: vlozeni jde line pres vlastnika a.
    let (g, out) = press(&mut t, b);
    assert!(got(&out, b).contains(&SMsg::Changed { formats: vec![TEXT.into()] }), "nove okno s fokusem vi o obsahu");
    clip(&mut t, b, CMsg::Request { gesture: g, serial: 6, format: TEXT.into() });
    let m = got(&t.out(), a);
    let SMsg::Send { serial: s, .. } = m[0] else { panic!("{m:?}") };
    clip(&mut t, a, CMsg::Data { serial: s, data: b"novy".to_vec() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Deliver { serial: 6, format: TEXT.into(), data: b"novy".to_vec() }]);
    // Jedno gesto = jedno vlozeni (znovu pouzite cislo).
    clip(&mut t, b, CMsg::Request { gesture: g, serial: 7, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 7, why: Denied::NoGesture }]);
    // Cizi klient nemuze podstrcit data za vlastnika.
    clip(&mut t, b, CMsg::Data { serial: s, data: b"x".to_vec() });
    assert!(t.out().is_empty());
}

/// Pomale zpracovani (emulace): gesto plati i po sekundach, ale jen do
/// zalozni meze; rychle psani po Ctrl+C gesto nezrusi.
#[test]
fn gesture_survives_slow_app_until_backstop() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let (g, _) = press(&mut t, a);
    // Dalsi stisky (psani) pred zpracovanim Ctrl+C.
    press(&mut t, a);
    press(&mut t, a);
    t.now += 2_000_000_000;
    clip(&mut t, a, CMsg::Offer { gesture: g, formats: vec![TEXT.into()] });
    assert!(!got(&t.out(), a).is_empty(), "2 s po stisku projde");
    // Nad zalozni mez ne.
    let (g, _) = press(&mut t, a);
    t.now += crate::clipboard::GESTURE_MAX + 1;
    clip(&mut t, a, CMsg::Offer { gesture: g, formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "po 10 s uz ne");
    // Jen z poslednich GESTURES stisku okna.
    let (old, _) = press(&mut t, a);
    for _ in 0..crate::clipboard::GESTURES {
        press(&mut t, a);
    }
    clip(&mut t, a, CMsg::Offer { gesture: old, formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "prilis stary stisk");
}

/// Podvrzene a cizi cislo gesta neprojde.
#[test]
fn forged_and_foreign_gesture_refused() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    let (gb, _) = press(&mut t, b);
    let (ga, _) = press(&mut t, a);
    // Podvrzene cislo (nikdy nedorucene).
    clip(&mut t, a, CMsg::Offer { gesture: ga.wrapping_add(1000), formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "podvrzene cislo");
    // Cislo dorucene jinemu oknu (b), a ma fokus.
    clip(&mut t, a, CMsg::Offer { gesture: gb, formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "cizi cislo");
    // Vlastni platne cislo projde, podruhe uz ne.
    clip(&mut t, a, CMsg::Offer { gesture: ga, formats: vec![TEXT.into()] });
    assert!(!got(&t.out(), a).is_empty());
    clip(&mut t, a, CMsg::Request { gesture: ga, serial: 1, format: TEXT.into() });
    assert_eq!(got(&t.out(), a), vec![SMsg::Denied { serial: 1, why: Denied::NoGesture }], "znovu pouzite cislo");
}

/// Vlastnik skonci: zustane kopie textu; cekajici vlozeni selze.
#[test]
fn content_survives_owner_and_pending_paste_fails() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    let b = t.client("b");
    t.window(b, Mode::Standard, RED);
    let (g, _) = press(&mut t, a);
    clip(&mut t, a, CMsg::Offer { gesture: g, formats: vec![TEXT.into(), "text/html".into()] });
    let m = got(&t.out(), a);
    let SMsg::Send { serial: keep, .. } = m[0] else { panic!() };
    clip(&mut t, a, CMsg::Data { serial: keep, data: b"kopie".to_vec() });
    // Kompozitor si vyzadal i HTML (kopie po skonceni vlastnika); a na
    // tu zadost neodpovi.
    // b pozada o HTML, a skonci driv, nez odpovi.
    let (g, _) = press(&mut t, b);
    clip(&mut t, b, CMsg::Request { gesture: g, serial: 1, format: "text/html".into() });
    t.out();
    t.c.remove_client(a);
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 1, why: Denied::Unavailable }]);
    // Text zustal (kopie), HTML ne.
    let (g, _) = press(&mut t, b);
    clip(&mut t, b, CMsg::Request { gesture: g, serial: 2, format: TEXT.into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Deliver { serial: 2, format: TEXT.into(), data: b"kopie".to_vec() }]);
    let (g, _) = press(&mut t, b);
    clip(&mut t, b, CMsg::Request { gesture: g, serial: 3, format: "text/html".into() });
    assert_eq!(got(&t.out(), b), vec![SMsg::Denied { serial: 3, why: Denied::Empty }]);
}

/// Umely vstup gesto neda; nabidka z fokusu v poli s heslem se odmitne.
#[test]
fn synthetic_input_and_secure_field_refused() {
    let mut t = T::new();
    let a = t.client("a");
    t.window(a, Mode::Standard, RED);
    t.key(inputproto::code::KEY_C, true);
    let out = t.out();
    assert_eq!(serial_in(&out, a), 0, "umely vstup nema cislo gesta");
    clip(&mut t, a, CMsg::Offer { gesture: 1, formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "umely vstup neni gesto");
    // Fokus v poli s heslem.
    let mut n = Node::new(2, 0, Role::PasswordInput, "Heslo");
    n.state = guiproto::a11y::state::FOCUSED;
    t.msg(a, ClientMsg::A11yUpdate { window: 1, nodes: vec![Node::new(1, 0, Role::Window, "Okno"), n] });
    t.commit(a, 2, 0, 0, 0);
    let (g, _) = press(&mut t, a);
    clip(&mut t, a, CMsg::Offer { gesture: g, formats: vec![TEXT.into()] });
    assert!(got(&t.out(), a).is_empty(), "heslo do schranky ne");
    assert!(t.c.take_trace().iter().any(|l| l.contains("zabezpecenem poli")));
    // Vadna zprava: chyba, ne panika.
    t.msg(a, ClientMsg::Clipboard { data: vec![9, 9] });
    assert!(client_msgs(&t.out(), a).iter().any(|m| matches!(m, ServerMsg::Error { .. })));
}

/// Odmitnuta schranka jde do logu sluzby s identitou aplikace (bez obsahu).
#[test]
fn denied_clipboard_is_logged_with_identity() {
    let mut t = T::new();
    let a = t.client("org.winehq.wine");
    t.window(a, Mode::Standard, RED);
    t.frame();
    t.c.take_logs();
    clip(&mut t, a, CMsg::Request { gesture: 0, serial: 1, format: TEXT.into() });
    clip(&mut t, a, CMsg::Offer { gesture: 0, formats: vec![TEXT.into()] });
    let logs = t.c.take_logs();
    assert!(logs.iter().any(|l| l.contains("vlozeni pro org.winehq.wine odmitnuto")), "{logs:?}");
    assert!(logs.iter().any(|l| l.contains("kopirovani pro org.winehq.wine odmitnuto")), "{logs:?}");
}
