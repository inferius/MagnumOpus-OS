//! Stabilni klice, id uzlu, jednotny stav, heslo, chyba, `skip_tab`.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Name(String),
    Pass(String),
    Go,
}

/// Formular: `extra` tlacitek vlozenych pred pole (data ze sluzby).
fn form(extra: usize, name: &str, pass: &str, err: &str) -> Node<M> {
    let mut ch: Vec<Node<M>> = (0..extra).map(|i| button(&alloc::format!("Nové {i}")).on_click(M::Go)).collect();
    ch.push(text_input(name).placeholder("Jméno").on_change(M::Name).key("jmeno"));
    ch.push(password_input(pass).placeholder("Heslo").on_change(M::Pass).error(err).key("heslo"));
    ch.push(button("Nápověda").on_click(M::Go).skip_tab(true).key("napoveda"));
    ch.push(button("Přihlásit").on_click(M::Go).key("ok"));
    column(ch)
}

fn ui(view: Node<M>) -> Ui<M> {
    let mut u = Ui::new(crate::Look::ui_fonts(), 400, 400);
    u.rebuild("f".into(), view);
    u
}

fn find(u: &Ui<M>, role: Role, name: &str) -> guiproto::a11y::Node {
    u.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap()
}

fn press(u: &mut Ui<M>, x: i32, y: i32, down: bool) -> event::Response<M> {
    u.event(InputEvent::Button { code: BTN_LEFT, pressed: down, x, y }, 0)
}

fn click(u: &mut Ui<M>, n: &guiproto::a11y::Node) -> Vec<M> {
    let (x, y) = (n.bounds.x + 5, n.bounds.y + 5);
    let mut m = press(u, x, y, true).msgs;
    m.extend(press(u, x, y, false).msgs);
    m
}

fn key(u: &mut Ui<M>, code: u16) -> event::Response<M> {
    let mut r = u.event(InputEvent::Key { code, action: 1, ch: None }, 0);
    let r2 = u.event(InputEvent::Key { code, action: 0, ch: None }, 0);
    r.msgs.extend(r2.msgs);
    r
}

fn pixels(u: &mut Ui<M>, r: Rect) -> Vec<u32> {
    let (w, h) = (u.w, u.h);
    let mut px = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut px, w, h, w).unwrap();
    u.draw(&mut c, 0);
    let mut out = Vec::new();
    for y in r.y..r.bottom() as i32 {
        for x in r.x..r.right() as i32 {
            out.push(c.get(x as u32, y as u32));
        }
    }
    out
}

/// Vlozeni prvku pred pole: fokus, kurzor i id uzlu zustanou u pole.
#[test]
fn focus_cursor_and_ids_follow_keys() {
    let mut u = ui(form(0, "Alena", "", ""));
    let f = find(&u, Role::TextInput, "Jméno");
    click(&mut u, &f);
    key(&mut u, KEY_HOME);
    key(&mut u, KEY_RIGHT);
    let ok_id = find(&u, Role::Button, "Přihlásit").id;
    u.rebuild("f".into(), form(3, "Alena", "", ""));
    let f2 = find(&u, Role::TextInput, "Jméno");
    assert_eq!(f2.id, f.id, "stabilni id uzlu");
    assert!(f2.state & state::FOCUSED != 0, "fokus zustal u pole");
    assert_ne!(f2.bounds.y, f.bounds.y, "pole se posunulo");
    assert_eq!(find(&u, Role::Button, "Přihlásit").id, ok_id);
    // Kurzor zustal za prvnim znakem: psani vlozi znak na pozici 1.
    let r = u.event(InputEvent::Text('x'), 0);
    assert_eq!(r.msgs, vec![M::Name("Axlena".into())]);
    // Zmizele prvky uvolni id, nove prvky nedostanou hned tatez.
    let gone: Vec<u32> = (0..3).map(|i| find(&u, Role::Button, &alloc::format!("Nové {i}")).id).collect();
    u.rebuild("f".into(), form(0, "Alena", "", ""));
    u.rebuild("f".into(), form(3, "Alena", "", ""));
    for i in 0..3 {
        assert!(!gone.contains(&find(&u, Role::Button, &alloc::format!("Nové {i}")).id));
    }
}

/// Stisk, prerozvrzeni (tlacitko se posune) a pusteni na novem miste:
/// stisk se prevede podle klice, akce probehne.
#[test]
fn press_follows_key_across_relayout() {
    let mut u = ui(form(0, "", "", ""));
    let b = find(&u, Role::Button, "Přihlásit").bounds;
    press(&mut u, b.x + 5, b.y + 5, true);
    assert!(find(&u, Role::Button, "Přihlásit").state & state::PRESSED != 0);
    u.rebuild("f".into(), form(2, "", "", ""));
    let b2 = find(&u, Role::Button, "Přihlásit").bounds;
    assert_ne!(b2.y, b.y);
    // Ukazatel uz neni nad tlacitkem: stisknuto zmizi, nad nim se vrati.
    assert!(find(&u, Role::Button, "Přihlásit").state & state::PRESSED == 0);
    u.event(InputEvent::PointerMove { x: b2.x + 5, y: b2.y + 5 }, 0);
    assert!(find(&u, Role::Button, "Přihlásit").state & state::PRESSED != 0);
    let r = press(&mut u, b2.x + 5, b2.y + 5, false);
    assert_eq!(r.msgs, vec![M::Go]);
}

/// Heslo: role `PasswordInput`, hodnota ani delka ve stromu ani
/// v trasovani, kresli se teckami (sirka znaku se neprozradi).
#[test]
fn password_field_hides_value() {
    let mut u = ui(form(0, "", "Tajne1", ""));
    let p = find(&u, Role::PasswordInput, "Heslo");
    assert!(p.value.is_empty());
    assert!(u.a11y().iter().all(|n| !n.value.contains("Tajne1")));
    // Klik na zacatek pole: kurzor na zacatek, psani vlozi znak tam.
    let (x, y) = (p.bounds.x + 2, p.bounds.y + 5);
    let mut trace = press(&mut u, x, y, true).trace;
    trace.extend(press(&mut u, x, y, false).trace);
    let t = u.event(InputEvent::Text('z'), 0);
    trace.extend(t.trace);
    assert!(trace.iter().all(|l| !l.contains("Tajne")), "{trace:?}");
    assert_eq!(t.msgs, vec![M::Pass("zTajne1".into())]);
    let draw = |v: &str| {
        let mut u = ui(form(0, "", v, ""));
        let b = find(&u, Role::PasswordInput, "Heslo").bounds;
        pixels(&mut u, b)
    };
    let a = draw("WWWWWW");
    assert_eq!(a, draw("iiiiii"), "tecky nezavisi na znacich");
    assert_ne!(a, draw("iiiii"));
}

/// Chyba: stav `INVALID`, text chyby jako podrizeny uzel, okraj `danger`.
#[test]
fn error_state_and_text() {
    let mut u = ui(form(0, "", "", "Heslo je příliš krátké"));
    let p = find(&u, Role::PasswordInput, "Heslo");
    assert!(p.state & state::INVALID != 0);
    let e = find(&u, Role::Label, "Heslo je příliš krátké");
    assert_eq!(e.parent, p.id);
    assert_eq!(e.id, error_node(p.id));
    assert!(find(&u, Role::TextInput, "Jméno").state & state::INVALID == 0);
    let mid = p.bounds.y + p.bounds.h as i32 / 2;
    let edge = pixels(&mut u, Rect::new(p.bounds.x, mid, 1, 1));
    assert_eq!(edge, vec![u.look.theme.color.danger]);
}

/// `skip_tab`: Tab prvek preskoci, klik mu fokus da.
#[test]
fn skip_tab_only_click_focus() {
    let mut u = ui(form(0, "", "", ""));
    let mut order = Vec::new();
    for _ in 0..4 {
        key(&mut u, KEY_TAB);
        order.push(u.a11y().into_iter().find(|n| n.state & state::FOCUSED != 0).unwrap().name);
    }
    assert_eq!(order, ["Heslo", "Přihlásit", "Jméno", "Heslo"]);
    let help = find(&u, Role::Button, "Nápověda");
    assert_eq!(click(&mut u, &help), vec![M::Go]);
    assert!(find(&u, Role::Button, "Nápověda").state & state::FOCUSED != 0);
}

/// Dva sourozenci se stejnym vlastnim klicem: ruzne klice i id.
#[test]
fn duplicate_keys_are_separated() {
    let u = ui(column([button::<M>("A").key("x"), button("B").key("x")]));
    let (a, b) = (find(&u, Role::Button, "A"), find(&u, Role::Button, "B"));
    assert_ne!(a.id, b.id);
    let k = |id: u32| u.elems[u.elem_ids.iter().position(|&i| i == id).unwrap()].key;
    assert_ne!(k(a.id), k(b.id));
}

/// Posun oblasti patri jejimu klici: oblast posunuta v okne posun drzi,
/// vrstvy (id oblasti) zustanou.
#[test]
fn scroll_offset_follows_key() {
    let view = |above: usize| -> Node<M> {
        let mut ch: Vec<Node<M>> = (0..above).map(|i| label(&alloc::format!("řádek {i}"))).collect();
        ch.push(scroll(list((0..30).map(|i| alloc::format!("{i}")), None)).height(100).key("seznam"));
        column(ch)
    };
    let mut u = ui(view(0));
    let a = u.scroll_areas()[0];
    u.event(InputEvent::PointerMove { x: a.rect.x + 5, y: a.rect.y + 5 }, 0);
    let r = u.event(InputEvent::Scroll { dx: 0, dy: -2 }, 0);
    assert_eq!(r.scrolls, vec![(a.id, 80)]);
    u.rebuild("f".into(), view(2));
    let b = u.scroll_areas()[0];
    assert_eq!((b.id, b.offset), (a.id, 80));
    assert_ne!(b.rect.y, a.rect.y);
    // Uzly obsahu lezi ve vrstve oblasti.
    let item = find(&u, Role::ListItem, "0");
    assert_eq!(item.layer, scroll_layers(b.id).1);
    assert_eq!(find(&u, Role::ScrollArea, "").actions, actions::SCROLL);
}

/// Klic je deterministicky (stejny na vsech architekturach).
#[test]
fn key_hash_is_fixed() {
    assert_eq!(Key::from("ok"), Key::from_bytes(b"ok"));
    assert_eq!(Key::from("").0, 0xcbf2_9ce4_8422_2325);
    assert_eq!(Key::from("a").0, 0xaf63_dc4c_8601_ec8c);
    assert_ne!(Key::from(1u64), Key::from(2u64));
}
