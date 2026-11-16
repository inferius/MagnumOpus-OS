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

#[test]
fn avatar_pro_prechod_startovni_obrazovky() {
    let mut t = T::new();
    let before = t.c.greeter_avatar(false);
    let mut t2 = login();
    let l = t2.c.greeter_layout();
    let a = l.avatar;
    // Stejna poloha bez uctu (pri pripojeni displeje) i s ucty.
    assert_eq!(before, (a.x as f32 + 44.0, a.y as f32 + 44.0, 88.0));
    t.c.session_opened(1, 1000, "alice", t.now);
    assert!(t.c.greeter_avatar(true).1 > before.1, "zamcena obrazovka ma avatar niz (hodiny)");
    t2.frame();
}

#[test]
fn bez_uctu_zprava_a_vypnuti_misto_formulare() {
    let mut t = T::new();
    t.c.set_login(true, t.now);
    t.c.take_login_requests();
    // Dokud seznam uctu neprisel: ani formular, ani zprava.
    assert!(t.c.greeter_visible() && !t.c.greeter.has_form() && !t.c.greeter.no_account());
    t.c.login_reply(Reply::Accounts(Vec::new()), t.now);
    let l = t.c.greeter_layout();
    assert!(!l.form && l.methods.is_empty() && l.power.len() == 2, "zadny formular, Vypnout a Restartovat");
    assert!(t.c.secure_fields().is_empty(), "zadne pole s tajemstvim");
    assert!(t.c.snapshot_lines("t").join("\n").contains("bez uctu"));
    t.now += dialog::INPUT_DELAY;
    // Psani ani Enter s prazdnym heslem nic neposle.
    text(&mut t, "heslo");
    assert!(t.c.greeter.field.is_empty());
    assert!(t.c.take_login_requests().is_empty());
    t.frame();
    save_png("comp-greeter-bez-uctu.png", &t.frame, W, H);
    // Tab na Restartovat, Enter = potvrzovaci dialog systemu.
    t.key(KEY_TAB, false);
    assert_eq!(t.c.greeter.power_sel, 1);
    t.key(KEY_ENTER, false);
    assert!(t.c.dialogs.active().is_some_and(|d| d.owner == crate::dialog::Owner::Power(true)), "dialog restartu");
    // Klik na Vypnout (po zavreni dialogu).
    t.key(KEY_ESC, false);
    let (_, r) = t.c.greeter_layout().power[0];
    t.click(r.x + 5, r.y + 5);
    assert!(t.c.dialogs.active().is_some_and(|d| d.owner == crate::dialog::Owner::Power(false)), "dialog vypnuti");
}

#[test]
fn cip_metody_jen_pri_vic_metodach() {
    let mut t = T::new();
    t.c.set_login(true, t.now);
    let one = vec![Account { uid: 1001, name: "bob".into(), display: "Bob".into(), methods: 1, flags: 0 }];
    t.c.login_reply(Reply::Accounts(one), t.now);
    let l = t.c.greeter_layout();
    assert!(l.form && l.methods.is_empty(), "jedna metoda: zadny cip");
    t.now += dialog::INPUT_DELAY;
    t.key(KEY_TAB, false);
    assert_eq!(t.c.greeter.method, None, "Tab s jednou metodou nic nemeni");
    // Vic metod (heslo, PIN, klic): cip pro kazdou.
    let more = vec![Account { uid: 1000, name: "alice".into(), display: "Alice".into(), methods: 1 | 2 | 4, flags: 0 }];
    t.c.login_reply(Reply::Accounts(more), t.now);
    let l = t.c.greeter_layout();
    let ms: Vec<Method> = l.methods.iter().map(|(m, _)| *m).collect();
    assert_eq!(ms, vec![Method::Password, Method::Pin, Method::Key]);
    assert_eq!(t.c.secure_fields(), vec![l.field]);
}

/// Relativni jas WCAG (sRGB -> linearni).
fn luminance(p: u32) -> f32 {
    let lin = |c: u32| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { libm::powf((c + 0.055) / 1.055, 2.4) }
    };
    0.2126 * lin(p >> 16 & 0xFF) + 0.7152 * lin(p >> 8 & 0xFF) + 0.0722 * lin(p & 0xFF)
}

fn ratio(a: u32, b: u32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Kontrast textu `text` stylu `s` vystredeneho na radku `y` proti
/// pozadi hned vedle nej: nejkontrastnejsi pixel textu (jadro glyfu) proti
/// prumeru pozadi vlevo a vpravo od textu.
fn text_contrast(t: &mut T, y: i32, text: &str, s: theme::TextStyle) -> f32 {
    let tw = t.c.style.width(text, s) as i32;
    let x0 = (W as i32 - tw) / 2;
    let mut best = 1.0f32;
    for yy in y..y + s.line as i32 {
        let bg = [t.px(x0 - 12, yy), t.px(x0 + tw + 12, yy)];
        for x in x0..x0 + tw {
            let p = t.px(x, yy);
            best = best.max(ratio(p, bg[0]).min(ratio(p, bg[1])));
        }
    }
    best
}

/// O24: hodiny, datum a jmeno na zamcene obrazovce jsou citelne ve
/// svetlem, tmavem i kontrastnim rezimu (kontrast z pixelu >= 4,5 : 1).
/// Snimky `target/comp-zamek-<rezim>.png`.
#[test]
fn zamek_citelny_ve_vsech_rezimech() {
    use theme::{Pref, Scheme};
    for (scheme, contrast, name) in [(Scheme::Light, false, "svetly"), (Scheme::Dark, false, "tmavy"), (Scheme::Dark, true, "kontrast")] {
        let mut t = login();
        // Volba vzhledu systemu i uzivatele, bez prechodu (Omezit pohyb).
        let p = Pref { scheme, contrast, reduce_motion: true };
        t.c.set_appearance_system(p, t.now);
        t.c.set_appearance_user(1000, p, t.now);
        t.c.set_clock(Some(("14:05".into(), "so 7. 11.".into())));
        t.c.session_opened(3, 1000, "alice", t.now);
        t.c.session_locked(3, true, t.now);
        t.frame();
        t.frame();
        save_png(&alloc::format!("comp-zamek-{name}.png"), &t.frame, W, H);
        let l = t.c.greeter_layout();
        let th = t.c.style.theme.clone();
        for (what, y, text, s) in [
            ("hodiny", l.clock_y, "14:05", th.text.clock),
            ("datum", l.clock_y + th.text.clock.line as i32, "so 7. 11.", th.text.title),
            ("jmeno", l.name_y, "Alice", th.text.title),
        ] {
            let c = text_contrast(&mut t, y, text, s);
            assert!(c >= 4.5, "{name}: {what} kontrast {c:.2} : 1");
        }
    }
}

/// Enter s prazdnym polem nic neposle, ale rekne proc (O25); psani zpravu
/// zase smaze.
#[test]
fn prazdne_heslo_rekne_proc() {
    let mut t = login();
    t.now += dialog::INPUT_DELAY;
    t.key(KEY_ENTER, false);
    assert!(t.c.take_login_requests().is_empty());
    assert_eq!(t.c.greeter.msg, Some((String::from("Zadejte PIN."), true)));
    text(&mut t, "1");
    t.key(KEY_ENTER, false);
    assert_eq!(t.c.take_login_requests().len(), 1);
}
