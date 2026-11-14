//! Bezne chovani textovych poli jako na Windows / macOS / GNOME: slova
//! s Alt, prepis, nejvetsi delka, jen pro cteni, cislo, hledani, oko
//! hesla, Caps Lock, prstenec fokusu jen z klavesnice, blikani kurzoru,
//! automaticky posun pri tahu, kontextova nabidka, slucovani kroku zpet.

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::{Role, actions};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::{ALT, CAPS_LOCK, CTRL, SHIFT};

use crate::event::Clip;
use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Text(String),
    Ro(String),
    Max(String),
    Num(String),
    Search(String),
    Found(String),
    Pass(String),
}

struct A {
    text: String,
    ro: String,
    max: String,
    num: String,
    search: String,
    found: Vec<String>,
    pass: String,
    ui: Ui<M>,
}

const MS: u64 = 1_000_000;

impl A {
    fn new() -> Self {
        let mut a = A {
            text: "ahoj krásný světe".into(),
            ro: "jen ke čtení".into(),
            max: String::new(),
            num: "7".into(),
            search: String::new(),
            found: Vec::new(),
            pass: String::new(),
            ui: Ui::new(crate::Look::ui_fonts(), 420, 600),
        };
        a.rebuild();
        a
    }

    fn view(&self) -> Node<M> {
        column([
            text_input(&self.text).placeholder("Text").on_change(M::Text).key("t"),
            text_input(&self.ro).placeholder("Cteni").read_only().on_change(M::Ro).key("ro"),
            text_input(&self.max).placeholder("Max").max_len(5).on_change(M::Max).key("mx"),
            number_input(&self.num).placeholder("Cislo").range(0.0, 100.0).num_step(5.0).unit("%").on_change(M::Num).key("num"),
            search_input(&self.search).placeholder("Hledat").on_change(M::Search).on_search(M::Found).key("s"),
            password_input(&self.pass).placeholder("Heslo").on_change(M::Pass).key("p"),
            button("Dál").key("b"),
        ])
    }

    fn rebuild(&mut self) {
        let v = self.view();
        self.ui.rebuild("a".into(), v);
    }

    fn apply(&mut self, mut r: event::Response<M>) -> event::Response<M> {
        r.msgs.extend(self.ui.take_msgs());
        for m in &r.msgs {
            match m.clone() {
                M::Text(t) => self.text = t,
                M::Ro(t) => self.ro = t,
                M::Max(t) => self.max = t,
                M::Num(t) => self.num = t,
                M::Search(t) => self.search = t,
                M::Found(t) => self.found.push(t),
                M::Pass(t) => self.pass = t,
            }
        }
        self.rebuild();
        r
    }

    fn ev(&mut self, e: InputEvent, mods: u16, t: Option<u64>) -> event::Response<M> {
        let r = self.ui.event_at(e, mods, t);
        self.apply(r)
    }

    fn key(&mut self, code: u16, mods: u16) -> event::Response<M> {
        self.key_ch(code, None, mods)
    }

    fn key_ch(&mut self, code: u16, ch: Option<char>, mods: u16) -> event::Response<M> {
        let mut r = self.ev(InputEvent::Key { code, action: 1, ch }, mods, None);
        let r2 = self.ev(InputEvent::Key { code, action: 0, ch }, mods, None);
        r.msgs.extend(r2.msgs);
        r.unhandled_key = r.unhandled_key.or(r2.unhandled_key);
        r
    }

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.ev(InputEvent::Text(c), 0, None);
        }
    }

    fn idx(&self, k: &str) -> usize {
        self.ui.index_of(Key::ROOT.mix(1).mix(Key::from(k).0)).unwrap()
    }

    fn rect(&self, k: &str) -> Rect {
        self.ui.elems[self.idx(k)].rect
    }

    /// Klik doprostred pole (cas `t` ms).
    fn click(&mut self, k: &str, t: u64) {
        let r = self.rect(k);
        let (x, y) = (r.x + 6, r.y + r.h as i32 / 2);
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, Some(t * MS));
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, 0, Some(t * MS));
    }

    fn edit(&self, k: &str) -> &text::TextEdit {
        &self.ui.edits[&self.ui.elems[self.idx(k)].key]
    }

    fn focus_on(&mut self, k: &str) {
        self.ui.focus = Some(self.idx(k));
    }
}

/// Alt+sipky a Alt+Backspace po slovech (macOS), Ctrl stejne (Windows).
#[test]
fn alt_and_ctrl_move_by_words() {
    let mut a = A::new();
    a.focus_on("t");
    a.key(KEY_END, 0);
    a.key(KEY_LEFT, ALT);
    assert_eq!(a.edit("t").cursor(), "ahoj krásný ".len());
    a.key(KEY_LEFT, CTRL | SHIFT);
    assert_eq!(a.edit("t").selected_text(), "krásný ");
    a.key(KEY_END, 0);
    a.key(KEY_BACKSPACE, ALT);
    assert_eq!(a.text, "ahoj krásný ");
    a.key(KEY_HOME, 0);
    a.key(KEY_DELETE, CTRL);
    assert_eq!(a.text, "krásný ", "slovo i mezera za nim (Windows)");
}

/// Insert prepne prepis: psani nahradi znak za kurzorem, ne konec.
#[test]
fn insert_toggles_overwrite() {
    let mut a = A::new();
    a.focus_on("t");
    a.key(KEY_HOME, 0);
    a.key(KEY_INSERT, 0);
    assert!(a.edit("t").overwrite);
    a.type_str("AH");
    assert_eq!(a.text, "AHoj krásný světe");
    a.key(KEY_END, 0);
    a.type_str("!");
    assert_eq!(a.text, "AHoj krásný světe!", "na konci se pripisuje");
    a.key(KEY_INSERT, 0);
    a.key(KEY_HOME, 0);
    a.type_str("x");
    assert_eq!(a.text, "xAHoj krásný světe!");
}

/// Nejvetsi delka: psani navic se ignoruje, vlozeni se zkrati (grafemy).
#[test]
fn max_len_limits_typing_and_paste() {
    let mut a = A::new();
    a.focus_on("mx");
    a.type_str("Čaj");
    assert_eq!(a.key_ch(KEY_V, Some('v'), CTRL).clipboard, Some(Clip::Paste));
    let r = a.ui.paste("ovna kava");
    a.apply(r);
    assert_eq!(a.max, "Čajov");
    a.type_str("z");
    assert_eq!(a.max, "Čajov");
    // Nahrada vyberu se vejde.
    a.key(KEY_LEFT, SHIFT);
    a.type_str("é");
    assert_eq!(a.max, "Čajoé");
}

/// Jen pro cteni: vybrat a kopirovat ano, psani, mazani, vyjmuti,
/// vlozeni, zpet ani akce pristupnosti text nezmeni.
#[test]
fn read_only_selects_and_copies() {
    let mut a = A::new();
    a.focus_on("ro");
    a.key_ch(KEY_A, Some('a'), CTRL);
    let r = a.key_ch(KEY_C, Some('c'), CTRL);
    assert_eq!(r.clipboard, Some(Clip::Copy("jen ke čtení".into())));
    a.type_str("x");
    a.key(KEY_BACKSPACE, 0);
    a.key_ch(KEY_X, Some('x'), CTRL);
    assert_eq!(a.key_ch(KEY_V, Some('v'), CTRL).clipboard, None);
    a.key(KEY_INSERT, 0);
    assert!(!a.edit("ro").overwrite);
    assert_eq!(a.ro, "jen ke čtení");
    let id = a.ui.node_id(a.idx("ro"));
    let r = a.ui.a11y_action(id, actions::SET_VALUE, "jine");
    a.apply(r);
    assert_eq!(a.ro, "jen ke čtení");
}

/// Cislo: jen cislice, sipky krok (na mrizce kroku), PgUp/PgDn 10 kroku,
/// Home/End meze, krokovaci tlacitka mysi; meze se nepresahnou.
#[test]
fn number_filters_and_steps() {
    let mut a = A::new();
    a.focus_on("num");
    a.key(KEY_END, SHIFT);
    a.type_str("a-,");
    assert_eq!(a.num, "7", "pismena, minus (min 0) ani carka (krok 5) neprojdou");
    a.key(KEY_UP, 0);
    assert_eq!(a.num, "10");
    a.key(KEY_PAGEUP, 0);
    assert_eq!(a.num, "60");
    a.key(KEY_PAGEUP, 0);
    assert_eq!(a.num, "100", "horni mez");
    a.key(KEY_HOME, 0);
    assert_eq!(a.num, "0");
    a.key(KEY_DOWN, 0);
    assert_eq!(a.num, "0", "dolni mez");
    a.key(KEY_END, 0);
    assert_eq!(a.num, "100");
    // Krokovaci tlacitka: dolni pulka dolu.
    let i = a.idx("num");
    let (_, down) = a.ui.field_buttons(i).into_iter().find(|(b, _)| *b == crate::field::FieldBtn::Down).unwrap();
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: down.x + 2, y: down.y + 2 }, 0, Some(MS));
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: down.x + 2, y: down.y + 2 }, 0, Some(MS));
    assert_eq!(a.num, "95");
    // Jednotka a krokovani zuzi plochu textu, text je vpravo.
    assert!(a.ui.field_align(i) > 0);
    // Desetinny krok: carka projde a formatuje se cesky.
    let spec = fieldopts::NumberSpec { min: 0.0, max: 1.0, step: 0.25, unit: String::new() };
    assert!(spec.accepts(','));
    assert_eq!(spec.stepped("0,5", 1), "0,75");
    assert_eq!(spec.stepped("", 1), "0,25");
}

/// Hledani: filtr az po prodleve bez psani, Esc smaze (hned prazdne
/// hledani), druhe Esc nechava aplikaci, sipka dolu na dalsi prvek.
#[test]
fn search_escape_debounce_and_down() {
    let mut a = A::new();
    a.focus_on("s");
    a.ev(InputEvent::Text('k'), 0, Some(100 * MS));
    a.ev(InputEvent::Text('o'), 0, Some(120 * MS));
    assert_eq!(a.search, "ko");
    assert!(a.found.is_empty(), "jeste se nepsalo dost dlouho");
    let due = a.ui.next_deadline().unwrap();
    assert!(due >= 120 * MS + 150 * MS - MS && due <= 120 * MS + 150 * MS);
    a.ui.timer(due);
    let r = event::Response::default();
    a.apply(r);
    assert_eq!(a.found, ["ko"]);
    // Krizek je videt jen s textem.
    let i = a.idx("s");
    assert!(a.ui.field_buttons(i).iter().any(|(b, _)| *b == crate::field::FieldBtn::Clear));
    a.key(KEY_ESC, 0);
    assert_eq!(a.search, "");
    assert_eq!(a.found, ["ko", ""], "prazdne hledani hned");
    assert!(a.ui.field_buttons(i).is_empty());
    let r = a.key(KEY_ESC, 0);
    assert_eq!(r.unhandled_key.map(|k| k.0), Some(KEY_ESC), "druhe Esc pro aplikaci");
    a.key(KEY_DOWN, 0);
    assert_eq!(a.ui.focus, Some(a.idx("p")), "sipka dolu na dalsi prvek");
}

/// Oko hesla: zobrazi znaky (ne tecky), do schranky heslo ani tak nejde;
/// Caps Lock ukaze varovani pod polem.
#[test]
fn password_eye_and_caps_lock() {
    let mut a = A::new();
    a.click("p", 1);
    a.type_str("Tajne1");
    let i = a.idx("p");
    assert!(a.ui.field_masked(i));
    let (_, eye) = a.ui.field_buttons(i).into_iter().find(|(b, _)| *b == crate::field::FieldBtn::Eye).unwrap();
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: eye.x + 3, y: eye.y + 3 }, 0, Some(5 * MS));
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: eye.x + 3, y: eye.y + 3 }, 0, Some(5 * MS));
    assert!(!a.ui.field_masked(i), "oko zobrazi heslo");
    assert_eq!(a.pass, "Tajne1", "klik na oko text nezmeni");
    a.key_ch(KEY_A, Some('a'), CTRL);
    assert_eq!(a.key_ch(KEY_C, Some('c'), CTRL).clipboard, None);
    assert!(!a.ui.caps_warning(i));
    a.ev(InputEvent::Text('x'), CAPS_LOCK, None);
    assert!(a.ui.caps_warning(i));
    // Heslo nikdy do stromu pristupnosti, ani zobrazene.
    let n = a.ui.a11y().into_iter().find(|n| n.role == Role::PasswordInput).unwrap();
    assert!(n.value.is_empty());
}

/// Prstenec fokusu jen po klavesnici: klik ho neukaze, Tab ano.
#[test]
fn focus_ring_only_from_keyboard() {
    let mut a = A::new();
    a.click("t", 1);
    let i = a.idx("t");
    assert!(a.ui.state(i).focused);
    assert!(!a.ui.state(i).focus_visible);
    a.key(KEY_TAB, 0);
    let j = a.ui.focus.unwrap();
    assert!(a.ui.state(j).focus_visible);
    a.click("t", 2);
    assert!(!a.ui.state(i).focus_visible);
}

/// Kurzor: po cinnosti cela perioda videt, pak blika; po psani znovu
/// videt; po zastaveni blikani zustane videt a casovac nic neceka.
#[test]
fn caret_blinks_pauses_and_stops() {
    let mut a = A::new();
    let m = a.ui.look.theme.motion.clone();
    let half = m.caret_blink as u64 * MS;
    a.click("t", 1000);
    let t0 = 1000 * MS;
    assert!(a.ui.caret_on);
    assert_eq!(a.ui.next_deadline(), Some(t0 + 2 * half));
    assert!(a.ui.timer(t0 + 2 * half), "zhasne");
    assert!(!a.ui.caret_on);
    assert!(a.ui.timer(t0 + 3 * half), "rozsviti se");
    assert!(a.ui.caret_on);
    // Psani: hned videt a pocita se znovu.
    a.ui.timer(t0 + 4 * half);
    assert!(!a.ui.caret_on);
    a.ev(InputEvent::Text('x'), 0, Some(t0 + 4 * half + MS));
    assert!(a.ui.caret_on);
    // Po zastaveni videt a bez casovace.
    let stop = t0 + 4 * half + MS + m.caret_blink_stop as u64 * MS;
    a.ui.timer(stop);
    assert!(a.ui.caret_on);
    assert_eq!(a.ui.next_deadline(), None);
}

/// Tah s vyberem za pravy okraj: casovac posouva pole a rozsiruje vyber
/// i bez pohybu mysi.
#[test]
fn drag_past_edge_autoscrolls() {
    let mut a = A::new();
    a.text = "slovo ".repeat(30);
    a.rebuild();
    let r = a.rect("t");
    let y = r.y + r.h as i32 / 2;
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: r.x + 20, y }, 0, Some(MS));
    a.key(KEY_HOME, 0);
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: r.x + 20, y }, 0, Some(MS));
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: r.x + 20, y }, 0, Some(10 * MS));
    a.ev(InputEvent::PointerMove { x: r.right() as i32 + 30, y }, 0, Some(20 * MS));
    let key = a.ui.elems[a.idx("t")].key;
    let first = a.edit("t").cursor();
    let sx0 = a.ui.tscroll[&key].0;
    let mut t = 20 * MS;
    for _ in 0..5 {
        let d = a.ui.next_deadline().expect("posun ceka na casovac");
        t = t.max(d);
        a.ui.timer(t);
    }
    assert!(a.edit("t").cursor() > first, "vyber roste");
    assert!(a.ui.tscroll[&key].0 > sx0, "pole se posouva");
    a.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: r.right() as i32 + 30, y }, 0, Some(t));
    assert!(a.edit("t").selection().is_some());
}

/// Kontextova nabidka pole: polozky s duvody nedostupnosti, Vybrat vse
/// a Kopirovat z nabidky; prave tlacitko v textu presune kurzor.
#[test]
fn field_context_menu() {
    let mut a = A::new();
    let r = a.rect("t");
    let (x, y) = (r.x + 8, r.y + r.h as i32 / 2);
    a.ev(InputEvent::Button { code: BTN_RIGHT, pressed: true, x, y }, 0, Some(MS));
    a.ev(InputEvent::Button { code: BTN_RIGHT, pressed: false, x, y }, 0, Some(MS));
    assert!(a.ui.popup_open());
    let items: Vec<_> = a.ui.a11y().into_iter().filter(|n| n.role == Role::MenuItem).collect();
    let names: Vec<&str> = items.iter().map(|n| n.name.as_str()).collect();
    assert!(names.contains(&"Kopírovat (nic není vybráno)"), "{names:?}");
    assert!(names.contains(&"Zpět (není co vrátit)"), "{names:?}");
    let all = items.iter().find(|n| n.name == "Vybrat vše").unwrap().id;
    let r = a.ui.a11y_action(all, actions::ACTIVATE, "");
    a.apply(r);
    assert_eq!(a.edit("t").selected_text(), "ahoj krásný světe");
    // Klavesnice: Shift+F10 otevre nabidku u kurzoru, Kopirovat.
    a.key(KEY_F10, SHIFT);
    assert!(a.ui.popup_open());
    let copy = a.ui.a11y().into_iter().find(|n| n.role == Role::MenuItem && n.name == "Kopírovat").unwrap().id;
    let r = a.ui.a11y_action(copy, actions::ACTIVATE, "");
    assert_eq!(r.clipboard, Some(Clip::Copy("ahoj krásný světe".into())));
    // Heslo: Kopirovat nikdy.
    a.click("p", 50);
    a.type_str("abc");
    a.key_ch(KEY_A, Some('a'), CTRL);
    a.key(127, 0);
    let names: Vec<String> = a.ui.a11y().into_iter().filter(|n| n.role == Role::MenuItem).map(|n| n.name).collect();
    assert!(names.iter().any(|n| n == "Kopírovat (heslo nejde kopírovat)"), "{names:?}");
}

/// Zpet: psani po sobe je jeden krok, pohyb kurzoru krok ukonci.
#[test]
fn undo_merges_typing_until_cursor_moves() {
    let mut a = A::new();
    a.text = String::new();
    a.rebuild();
    a.focus_on("t");
    a.type_str("abc");
    a.key(KEY_LEFT, 0);
    a.type_str("x");
    assert_eq!(a.text, "abxc");
    a.key_ch(KEY_Z, Some('z'), CTRL);
    assert_eq!(a.text, "abc");
    a.key_ch(KEY_Z, Some('z'), CTRL);
    assert_eq!(a.text, "");
    a.key_ch(KEY_Z, Some('z'), CTRL | SHIFT);
    assert_eq!(a.text, "abc");
}
