//! Prihlasovaci a zamykaci obrazovka.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use inputproto::code::*;

use super::*;
use crate::greeter::{Account, Method, Reply, Request};

fn accounts() -> Vec<Account> {
    vec![
        Account { uid: 1000, name: "alice".into(), display: "Alice".into(), methods: 1 | 2, flags: 0 },
        Account { uid: 1001, name: "bob".into(), display: "Bob".into(), methods: 1, flags: 0 },
    ]
}

/// Obrazovka prihlaseni s dvema ucty.
fn login() -> T {
    let mut t = T::new();
    t.c.set_login(true, t.now);
    assert_eq!(t.c.take_login_requests(), vec![Request::List]);
    t.c.login_reply(Reply::Accounts(accounts()), t.now);
    assert_eq!(t.c.take_login_requests(), vec![Request::Phrase(1000)]);
    t.c.login_reply(Reply::Phrase(1000, "modra konvice".into()), t.now);
    t
}

#[test]
fn bez_kanalu_login_obrazovka_neni() {
    let t = T::new();
    assert!(!t.c.greeter_visible());
}

#[test]
fn prihlaseni_pinem_vychozi_metoda() {
    let mut t = login();
    assert!(t.c.greeter_visible());
    t.now += dialog::INPUT_DELAY;
    // Alice ma PIN: vychozi metoda PIN, jine znaky nez cislice se zahodi.
    text(&mut t, "12a34");
    t.key(KEY_ENTER, false);
    let r = t.c.take_login_requests();
    assert_eq!(r, vec![Request::Auth { uid: 1000, method: Method::Pin, secret: String::from("1234") }]);
    // Behem overeni se dalsi pokus neposle.
    text(&mut t, "9");
    t.key(KEY_ENTER, false);
    assert!(t.c.take_login_requests().is_empty());
    t.c.login_reply(Reply::Ok, t.now);
    t.c.session_opened(1, 1000, "alice", t.now);
    assert!(!t.c.greeter_visible(), "po prihlaseni plocha");
}

#[test]
fn umely_vstup_se_ignoruje() {
    let mut t = login();
    t.now += dialog::INPUT_DELAY;
    for ch in "1234".chars() {
        let r = inputproto::Record::new(Event::Text(ch), 0, flags::SYNTHETIC, t.now);
        t.c.input(&[r], t.now);
    }
    t.key(KEY_ENTER, true);
    assert!(t.c.take_login_requests().is_empty());
}

#[test]
fn vyber_uctu_a_metody() {
    let mut t = login();
    t.key(KEY_RIGHT, false);
    assert_eq!(t.c.take_login_requests(), vec![Request::Phrase(1001)]);
    // Bob ma jen heslo; Tab zustane na hesle.
    t.key(KEY_TAB, false);
    text(&mut t, "tajne heslo");
    t.key(KEY_ENTER, false);
    assert_eq!(t.c.take_login_requests(), vec![Request::Auth { uid: 1001, method: Method::Password, secret: "tajne heslo".into() }]);
}

#[test]
fn zpomaleni_po_chybe() {
    let mut t = login();
    t.now += dialog::INPUT_DELAY;
    text(&mut t, "0000");
    t.key(KEY_ENTER, false);
    t.c.take_login_requests();
    t.c.login_reply(Reply::Denied { retry_after_ms: 30_000, pin_left: 2 }, t.now);
    assert!(t.c.greeter.msg.as_ref().is_some_and(|m| m.1 && m.0.contains("Zbývá pokusů: 2")));
    // V dobe cekani se nic neposle.
    text(&mut t, "1111");
    t.key(KEY_ENTER, false);
    assert!(t.c.take_login_requests().is_empty());
    assert!(t.c.next_wakeup(t.now).is_some(), "odpocet probouzi snimek");
    t.now += 31_000_000_000;
    t.key(KEY_ENTER, false);
    assert_eq!(t.c.take_login_requests().len(), 1);
}

#[test]
fn zamceni_a_odemceni_jen_stejny_ucet() {
    let mut t = login();
    t.c.session_opened(7, 1001, "bob", t.now);
    assert!(!t.c.greeter_visible());
    // Bezpecna obrazovka nabidne zamceni.
    t.c.secure_attention(t.now);
    t.key(KEY_RIGHT, false);
    t.key(KEY_ENTER, false);
    assert_eq!(t.c.take_login_requests(), vec![Request::Lock { session: 7 }]);
    t.c.session_locked(7, true, t.now);
    assert!(t.c.greeter_visible());
    t.c.take_login_requests();
    t.now += dialog::INPUT_DELAY;
    // Vyber uctu na zamcene obrazovce neni: odemyka Bob.
    t.key(KEY_LEFT, false);
    text(&mut t, "bobovo heslo");
    t.key(KEY_ENTER, false);
    assert_eq!(t.c.take_login_requests(), vec![Request::Unlock { session: 7, method: Method::Password, secret: "bobovo heslo".into() }]);
    t.c.login_reply(Reply::Ok, t.now);
    t.c.session_locked(7, false, t.now);
    assert!(!t.c.greeter_visible());
}

#[test]
fn obrazovka_skryje_okna() {
    let mut t = login();
    let cl = t.client("app");
    t.window(cl, Mode::Standard, 0xFF00_FF00);
    t.frame();
    let r = t.rect_of(cl);
    let (x, y) = (r.x + 10, r.y + 10);
    assert_ne!(t.px(x, y), 0xFF00_FF00, "obsah okna pod obrazovkou neni videt");
    t.c.session_opened(1, 1000, "alice", t.now);
    t.frame();
    assert_eq!(t.px(x, y), 0xFF00_FF00, "po prihlaseni okno");
}

#[test]
fn odhlaseni_vrati_prihlaseni() {
    let mut t = login();
    t.c.session_opened(1, 1000, "alice", t.now);
    t.c.take_login_requests();
    t.c.session_closed(1, t.now);
    assert!(t.c.greeter_visible());
    assert_eq!(t.c.take_login_requests(), vec![Request::List]);
}

#[test]
fn snimek_obrazovky() {
    // Pro kontrolu okem: `target/comp-greeter.png`, `target/comp-greeter-zamceno.png`.
    let mut t = login();
    t.c.set_clock(Some(("14:05".into(), "so 7. 11.".into())));
    t.now += dialog::INPUT_DELAY;
    text(&mut t, "12");
    t.frame();
    save_png("comp-greeter.png", &t.frame, W, H);
    t.c.session_opened(3, 1000, "alice", t.now);
    t.c.session_locked(3, true, t.now);
    t.c.login_reply(Reply::Denied { retry_after_ms: 30_000, pin_left: 4 }, t.now);
    t.frame();
    save_png("comp-greeter-zamceno.png", &t.frame, W, H);
}

#[test]
fn dialog_zvyseni_prav() {
    let mut t = T::new();
    t.c.dialog_request(1, DialogRequest::Phrase { text: "modrá konvice".into() }, t.now);
    t.c.dialog_request(
        1,
        DialogRequest::Ask {
            id: 9,
            kind: DialogKind::Elevate,
            app: "Nastavení".into(),
            text: "Přidat účet bob (Bob)\nPotvrzuje: Alice (smí spravovat tento počítač)".into(),
            options: vec!["Povolit změnu".into(), "Zrušit".into()],
        },
        t.now,
    );
    t.frame();
    save_png("comp-elevate.png", &t.frame, W, H);
    t.now += dialog::INPUT_DELAY;
    text(&mut t, "heslo");
    // Zaskrtavatko je za tlacitky (Tab dvakrat), Enter ho prepne.
    t.key(KEY_TAB, false);
    t.key(KEY_TAB, false);
    t.key(KEY_ENTER, false);
    // Zpet na "Povolit změnu" a potvrdit.
    t.key(KEY_TAB, false);
    t.key(KEY_ENTER, false);
    let out = t.out();
    assert!(out.contains(&Out::Dialog(1, DialogReply::Answer { id: 9, choice: 2, text: "heslo".into() })), "{out:?}");
}

#[test]
fn dialog_zvyseni_prav_esc_zrusi() {
    let mut t = T::new();
    t.c.dialog_request(
        1,
        DialogRequest::Ask { id: 4, kind: DialogKind::Elevate, app: "X".into(), text: "?".into(), options: vec!["Povolit změnu".into(), "Zrušit".into()] },
        t.now,
    );
    t.key(KEY_ESC, false);
    assert!(t.out().contains(&Out::Dialog(1, DialogReply::Answer { id: 4, choice: 1, text: String::new() })));
}

#[test]
fn tajemstvi_neni_v_trasovani_ani_snimku() {
    let mut t = login();
    t.now += dialog::INPUT_DELAY;
    t.c.take_trace();
    text(&mut t, "Zx9qW7");
    let trace = t.c.take_trace();
    assert!(!trace.is_empty() && trace.iter().all(|l| l.contains("vstup do zabezpeceneho pole")), "{trace:?}");
    let snap = t.c.snapshot_lines("t1");
    let all = snap.join("\n");
    for bad in ["Zx9qW7", "Zx9", "6 znak"] {
        assert!(!all.contains(bad), "snimek obsahuje {bad}: {all}");
    }
    assert!(all.contains("pole zabezpecene"), "{all}");
    // Obrazek snimku zakryje pole (sluzba vyplni tyto obdelniky).
    let f = t.c.secure_fields();
    assert_eq!(f, vec![t.c.greeter_layout().field]);
    // Dialog s heslem: totez, Debug heslo nevypise.
    t.c.session_opened(1, 1000, "alice", t.now);
    t.c.dialog_request(
        1,
        DialogRequest::Ask { id: 1, kind: DialogKind::Elevate, app: "X".into(), text: "?".into(), options: vec!["Povolit změnu".into(), "Zrušit".into()] },
        t.now,
    );
    t.now += dialog::INPUT_DELAY;
    t.c.take_trace();
    text(&mut t, "Qp4Kd");
    let trace = t.c.take_trace();
    assert!(trace.iter().all(|l| !l.contains("Qp4Kd") && l.contains("zabezpeceneho pole")), "{trace:?}");
    assert!(!t.c.snapshot_lines("t2").join("\n").contains("Qp4Kd"));
    assert_eq!(t.c.secure_fields().len(), 1);
    let d = alloc::format!("{:?}", t.c.dialogs.active().unwrap());
    assert!(!d.contains("Qp4Kd"), "{d}");
}
