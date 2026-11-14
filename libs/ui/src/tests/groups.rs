//! Bezne chovani ostatnich prvku: tlacitko (akce pri pusteni, zruseni
//! odjetim a Esc), skupina radio (jedna zastavka Tab, sipky), zavreny
//! vyber (hledani psanim, Home/End, Tab v otevrenem vyberu zvoli),
//! posuvnik (kolecko jen s fokusem), zalozky (Ctrl+PgUp/PgDn), seznam
//! (Home/End, hledani psanim), nabidka (podnabidka, mnemonika).

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::Role;
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::{CTRL, SHIFT};

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Click,
    Radio(usize),
    Pick(usize),
    Vol(i32),
    Tab(usize),
    Row(usize),
    Menu(&'static str),
}

struct A {
    radio: usize,
    pick: usize,
    vol: i32,
    tab: usize,
    row: Option<usize>,
    log: Vec<M>,
    ui: Ui<M>,
}

const FRUIT: [&str; 6] = ["Banán", "Borůvka", "Citron", "Hruška", "Jablko", "Švestka"];

impl A {
    fn new() -> Self {
        let mut a = A { radio: 1, pick: 0, vol: 50, tab: 0, row: None, log: Vec::new(), ui: Ui::new(crate::Look::ui_fonts(), 420, 900) };
        a.rebuild();
        a
    }

    fn view(&self) -> Node<M> {
        let menu = alloc::vec![
            menu_item("&Kopírovat", M::Menu("kopirovat")),
            submenu("&Seřadit", alloc::vec![menu_item("Podle &názvu", M::Menu("nazev")), menu_item("Podle &velikosti", M::Menu("velikost"))]),
            menu_item("&Vložit", M::Menu("vlozit")),
            menu_item("Vy&mazat", M::Menu("vymazat")),
        ];
        column([
            button("Uložit").on_click(M::Click).key("b").context_menu(menu),
            row([radio("Malé", self.radio == 0).on_click(M::Radio(0)), radio("Střední", self.radio == 1).on_click(M::Radio(1)), radio("Velké", self.radio == 2).on_click(M::Radio(2))]).key("rg"),
            select(FRUIT.iter().map(|s| String::from(*s)), self.pick).on_pick(M::Pick).key("sel"),
            slider(self.vol, 0, 100).on_value(M::Vol).key("sl"),
            tabs(["Obecné", "Síť", "Zvuk"].into_iter().map(String::from), self.tab).on_pick(M::Tab).key("tb"),
            list(FRUIT.iter().map(|s| String::from(*s)), self.row).on_select(M::Row).key("ls"),
            button("Konec").key("b2"),
        ])
    }

    fn rebuild(&mut self) {
        let v = self.view();
        self.ui.rebuild("a".into(), v);
    }

    fn apply(&mut self, r: event::Response<M>) -> event::Response<M> {
        for m in &r.msgs {
            self.log.push(m.clone());
            match *m {
                M::Radio(k) => self.radio = k,
                M::Pick(k) => self.pick = k,
                M::Vol(v) => self.vol = v,
                M::Tab(k) => self.tab = k,
                M::Row(k) => self.row = Some(k),
                _ => {}
            }
        }
        self.rebuild();
        r
    }

    fn ev(&mut self, e: InputEvent, mods: u16, t: u64) -> event::Response<M> {
        let r = self.ui.event_at(e, mods, Some(t * 1_000_000));
        self.apply(r)
    }

    fn down(&mut self, code: u16, ch: Option<char>, mods: u16, t: u64) -> event::Response<M> {
        self.ev(InputEvent::Key { code, action: 1, ch }, mods, t)
    }

    fn up(&mut self, code: u16, mods: u16, t: u64) -> event::Response<M> {
        self.ev(InputEvent::Key { code, action: 0, ch: None }, mods, t)
    }

    fn key(&mut self, code: u16, mods: u16) {
        self.down(code, None, mods, 1);
        self.up(code, mods, 1);
    }

    fn ch(&mut self, c: char, t: u64) {
        self.down(KEY_A, Some(c), 0, t);
        self.up(KEY_A, 0, t);
    }

    fn idx(&self, k: &str) -> usize {
        self.ui.index_of(Key::ROOT.mix(1).mix(Key::from(k).0)).unwrap()
    }

    fn center(&self, i: usize) -> (i32, i32) {
        let r = self.ui.elems[i].rect;
        (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2)
    }

    fn radios(&self) -> Vec<usize> {
        (0..self.ui.elems.len()).filter(|&i| matches!(self.ui.elems[i].kind, layout::ElemKind::Radio { .. })).collect()
    }
}

/// Tlacitko: akce az pri pusteni; odjeti se stisknutym tlacitkem mysi
/// a pusteni mimo akci zrusi; Enter i mezernik pri pusteni, Esc pri
/// drzene klavese stisk zrusi.
#[test]
fn button_acts_on_release_and_cancels() {
    let mut a = A::new();
    let b = a.idx("b");
    let (x, y) = a.center(b);
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, 1);
    assert!(a.log.is_empty(), "stisk jeste nic");
    a.ev(InputEvent::PointerMove { x: x + 500, y }, 0, 2);
    assert!(!a.ui.state(b).pressed, "odjeti zrusi stav stisku");
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: x + 500, y }, 0, 3);
    assert!(a.log.is_empty(), "pusteni mimo = zadna akce");
    a.ui.focus = Some(b);
    a.down(KEY_SPACE, Some(' '), 0, 4);
    assert!(a.log.is_empty() && a.ui.state(b).pressed);
    a.up(KEY_SPACE, 0, 5);
    assert_eq!(a.log, [M::Click]);
    a.down(KEY_ENTER, None, 0, 6);
    a.down(KEY_ESC, None, 0, 7);
    a.up(KEY_ENTER, 0, 8);
    assert_eq!(a.log, [M::Click], "Esc zrusil drzeny Enter");
}

/// Skupina radio je jedna zastavka Tab (vybrana volba); sipky vybiraji
/// dokola a nesou fokus.
#[test]
fn radio_group_single_tab_stop_and_arrows() {
    let mut a = A::new();
    let rs = a.radios();
    a.ui.focus = Some(a.idx("b"));
    a.key(KEY_TAB, 0);
    assert_eq!(a.ui.focus, Some(rs[1]), "Tab na vybranou volbu");
    a.key(KEY_DOWN, 0);
    assert_eq!((a.radio, a.ui.focus), (2, Some(rs[2])));
    a.key(KEY_RIGHT, 0);
    assert_eq!((a.radio, a.ui.focus), (0, Some(rs[0])), "dokola");
    a.key(KEY_LEFT, 0);
    assert_eq!(a.radio, 2);
    a.key(KEY_TAB, 0);
    assert_eq!(a.ui.focus, Some(a.idx("sel")), "Tab ze skupiny ven, ne na dalsi volbu");
    a.key(KEY_TAB, SHIFT);
    assert_eq!(a.ui.focus, Some(rs[2]), "Shift+Tab zpet na vybranou");
}

/// Zavreny vyber: psani skoci na volbu (opakovane pismeno dal), slovo
/// najde presne; Home/End; otevreny vyber Tab zvoli aktivni polozku.
#[test]
fn select_typeahead_and_tab_picks() {
    let mut a = A::new();
    a.ui.focus = Some(a.idx("sel"));
    a.ch('b', 100);
    assert_eq!(a.pick, 1, "druhe na B (prvni je vybrane)");
    a.ch('b', 3000);
    assert_eq!(a.pick, 0, "dokola");
    a.ch('h', 5000);
    assert_eq!(a.pick, 3);
    a.ch('š', 7000);
    assert_eq!(a.pick, 5, "diakritika");
    a.key(KEY_HOME, 0);
    assert_eq!(a.pick, 0);
    a.key(KEY_END, 0);
    assert_eq!(a.pick, 5);
    // Otevrit, sipkou nahoru a Tab: zvoleno a fokus dal.
    a.key(KEY_F4, 0);
    assert!(a.ui.popup_open());
    a.key(KEY_UP, 0);
    a.key(KEY_TAB, 0);
    assert!(!a.ui.popup_open());
    assert_eq!(a.pick, 4);
    assert_eq!(a.ui.focus, Some(a.idx("sl")));
}

/// Kolecko meni posuvnik jen s fokusem (jinak posouva stranku).
#[test]
fn slider_wheel_needs_focus() {
    let mut a = A::new();
    let s = a.idx("sl");
    let (x, y) = a.center(s);
    a.ev(InputEvent::PointerMove { x, y }, 0, 1);
    a.ui.focus = Some(a.idx("b"));
    a.ev(InputEvent::Scroll { dx: 0, dy: 1 }, 0, 2);
    assert_eq!(a.vol, 50, "bez fokusu nic");
    a.ui.focus = Some(s);
    a.ev(InputEvent::Scroll { dx: 0, dy: 1 }, 0, 3);
    assert_eq!(a.vol, 51);
    a.key(KEY_PAGEUP, 0);
    assert_eq!(a.vol, 61);
    a.key(KEY_HOME, 0);
    assert_eq!(a.vol, 0);
}

/// Zalozky: Ctrl+PgDn / Ctrl+PgUp jako Ctrl+Tab / Ctrl+Shift+Tab.
#[test]
fn tabs_ctrl_page_keys() {
    let mut a = A::new();
    a.ui.focus = Some(a.idx("b"));
    a.key(KEY_PAGEDOWN, CTRL);
    assert_eq!(a.tab, 1);
    a.key(KEY_PAGEUP, CTRL);
    a.key(KEY_PAGEUP, CTRL);
    assert_eq!(a.tab, 2, "dokola");
    a.key(KEY_TAB, CTRL);
    assert_eq!(a.tab, 0);
}

/// Seznam: Home/End, PgDn, hledani psanim.
#[test]
fn list_home_end_and_typeahead() {
    let mut a = A::new();
    a.ui.focus = Some(a.idx("ls"));
    a.key(KEY_END, 0);
    assert_eq!(a.row, Some(5));
    a.key(KEY_HOME, 0);
    assert_eq!(a.row, Some(0));
    a.ch('c', 100);
    assert_eq!(a.row, Some(2));
    a.ch('j', 3000);
    a.ch('a', 3100);
    assert_eq!(a.row, Some(4), "slovo 'ja'");
    a.key(KEY_DOWN, 0);
    assert_eq!(a.row, Some(5));
}

/// Nabidka: mnemonika vybere polozku, podnabidka sipkou vpravo (prvni
/// polozka aktivni), vlevo zpet, Enter v podnabidce vybere a zavre vse;
/// mysi najeti otevre podnabidku a najeti na rodice ji zavre.
#[test]
fn menu_submenu_and_mnemonics() {
    let mut a = A::new();
    a.ui.focus = Some(a.idx("b"));
    a.key(KEY_F10, SHIFT);
    assert!(a.ui.popup_open());
    // Text polozky bez & (ctecka i kresleni).
    let names: Vec<String> = a.ui.a11y().into_iter().filter(|n| n.role == Role::MenuItem).map(|n| n.name).collect();
    assert_eq!(names, ["Kopírovat", "Seřadit", "Vložit", "Vymazat"]);
    a.ch('m', 10);
    assert_eq!(a.log, [M::Menu("vymazat")], "mnemonika uprostred slova");
    assert!(!a.ui.popup_open());
    a.key(KEY_F10, SHIFT);
    a.ch('s', 20);
    assert_eq!(a.ui.popup_stack.len(), 1, "mnemonika podnabidky ji otevre");
    let first: Vec<String> = a.ui.a11y().into_iter().filter(|n| n.role == Role::MenuItem).map(|n| n.name).collect();
    assert_eq!(first, ["Podle názvu", "Podle velikosti"]);
    a.key(KEY_LEFT, 0);
    assert!(a.ui.popup_stack.is_empty() && a.ui.popup_open(), "vlevo zpet do rodice");
    a.key(KEY_RIGHT, 0);
    assert_eq!(a.ui.popup_stack.len(), 1);
    a.key(KEY_DOWN, 0);
    a.key(KEY_ENTER, 0);
    assert_eq!(a.log.last(), Some(&M::Menu("velikost")));
    assert!(!a.ui.popup_open() && a.ui.popup_stack.is_empty());
    // Mys: najeti na podnabidku ji otevre, najeti na rodice zavre.
    a.key(KEY_F10, SHIFT);
    let root = a.ui.popup.as_ref().unwrap().rect;
    let pad = a.ui.look.theme.space[2] as i32;
    let row = a.ui.look.control_h() as i32;
    let (x, y) = (root.x + 20, root.y + pad + row + row / 2);
    a.ev(InputEvent::PointerMove { x, y }, 0, 30);
    assert_eq!(a.ui.popup_stack.len(), 1, "najeti otevre podnabidku");
    let sub = a.ui.popup.as_ref().unwrap().rect;
    assert!(sub.x >= root.right() as i32 - pad - 1, "vpravo od rodice");
    a.ev(InputEvent::PointerMove { x, y: root.y + pad + row / 2 }, 0, 31);
    assert!(a.ui.popup_stack.is_empty(), "najeti na jinou polozku rodice zavre podnabidku");
    a.key(KEY_ESC, 0);
    assert!(!a.ui.popup_open());
}
