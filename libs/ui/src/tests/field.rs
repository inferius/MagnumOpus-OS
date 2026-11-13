//! Textove pole a viceradkove pole: mys, klavesy, schranka, posun.

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::a11y::Role;
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::{CTRL, SHIFT};

use crate::event::Clip;
use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Text(String),
    Note(String),
    Pass(String),
    Send,
}

/// Aplikace s polem, viceradkovym polem a heslem; `upper` = aplikace
/// hodnotu pole upravi (velka pismena).
struct F {
    text: String,
    note: String,
    pass: String,
    sent: u32,
    upper: bool,
    ui: Ui<M>,
}

impl F {
    fn new(text: &str, note: &str) -> Self {
        let mut f = F { text: text.into(), note: note.into(), pass: String::new(), sent: 0, upper: false, ui: Ui::new(crate::Look::ui_fonts(), 400, 400) };
        f.rebuild();
        f
    }

    fn view(&self) -> Node<M> {
        column([
            text_input(&self.text).placeholder("Název").on_change(M::Text).on_submit(M::Send).key("t"),
            text_area(&self.note).placeholder("Poznámka").rows(3).on_change(M::Note).on_submit(M::Send).key("n"),
            password_input(&self.pass).placeholder("Heslo").on_change(M::Pass).key("p"),
        ])
    }

    fn rebuild(&mut self) {
        let v = self.view();
        self.ui.rebuild("f".into(), v);
    }

    fn apply(&mut self, r: event::Response<M>) -> event::Response<M> {
        for m in &r.msgs {
            match m.clone() {
                M::Text(t) => self.text = if self.upper { t.to_uppercase() } else { t },
                M::Note(t) => self.note = t,
                M::Pass(t) => self.pass = t,
                M::Send => self.sent += 1,
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
        r
    }

    fn type_str(&mut self, s: &str) {
        for c in s.chars() {
            self.ev(InputEvent::Text(c), 0, None);
        }
    }

    fn bounds(&self, role: Role, name: &str) -> Rect {
        self.ui.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap().bounds
    }

    fn button(&mut self, x: i32, y: i32, down: bool, t: u64, mods: u16) {
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: down, x, y }, mods, Some(t));
    }

    fn click(&mut self, x: i32, y: i32, t: u64) {
        self.button(x, y, true, t, 0);
        self.button(x, y, false, t, 0);
    }

    fn edit(&self, key: &str) -> &text::TextEdit {
        &self.ui.edits[&self.ui.elems[self.ui.index_of_key(key)].key]
    }
}

impl<M2: Clone> Ui<M2> {
    /// Index prvku s vlastnim klicem `k` (koren sloupce + klic).
    fn index_of_key(&self, k: &str) -> usize {
        let want = Key::ROOT.mix(1).mix(Key::from(k).0);
        self.index_of(want).unwrap()
    }
}

/// Tah mysi vybere text, Ctrl+C ho da do schranky, Ctrl+X vyjme,
/// Ctrl+V pozada o vlozeni a `paste` vlozi na kurzor.
#[test]
fn drag_select_copy_cut_paste() {
    let mut f = F::new("ahoj světe", "");
    let b = f.bounds(Role::TextInput, "Název");
    let y = b.y + b.h as i32 / 2;
    let x0 = b.x + 2;
    f.button(x0, y, true, 1, 0);
    f.ev(InputEvent::PointerMove { x: b.right() as i32 - 4, y }, 0, None);
    f.button(b.right() as i32 - 4, y, false, 2, 0);
    assert_eq!(f.edit("t").selected_text(), "ahoj světe");
    let r = f.key_ch(KEY_C, Some('c'), CTRL);
    assert_eq!(r.clipboard, Some(Clip::Copy("ahoj světe".into())));
    // Shift+Home: vyber od kurzoru (konec) na zacatek; Ctrl+X vyjme.
    f.key(KEY_END, 0);
    f.key(KEY_LEFT, CTRL | SHIFT);
    assert_eq!(f.edit("t").selected_text(), "světe");
    let r = f.key_ch(KEY_X, Some('x'), CTRL);
    assert_eq!(r.clipboard, Some(Clip::Copy("světe".into())));
    assert_eq!(f.text, "ahoj ");
    let r = f.key_ch(KEY_V, Some('v'), CTRL);
    assert_eq!(r.clipboard, Some(Clip::Paste));
    let r = f.ui.paste("Praho\nnad");
    f.apply(r);
    assert_eq!(f.text, "ahoj Praho nad", "jednoradkove pole: radek na mezeru");
    // Vlozeni bez pozadavku nic.
    let r = f.ui.paste("x");
    assert!(r.msgs.is_empty());
}

/// Ctrl+Z na ceskem rozlozeni (QWERTZ: znak `z` je na poloze KEY_Y).
#[test]
fn undo_redo_by_layout_letter() {
    let mut f = F::new("", "");
    let b = f.bounds(Role::TextInput, "Název");
    f.click(b.x + 4, b.y + 4, 1);
    f.type_str("Čaj s mlékem");
    f.key_ch(KEY_Y, Some('z'), CTRL);
    assert_eq!(f.text, "Čaj s ");
    f.key_ch(KEY_Y, Some('z'), CTRL);
    assert_eq!(f.text, "Čaj s");
    f.key_ch(KEY_Y, Some('Z'), CTRL | SHIFT);
    assert_eq!(f.text, "Čaj s ");
    // Ctrl+A a psani nahradi vse.
    f.key_ch(KEY_A, Some('a'), CTRL);
    f.type_str("x");
    assert_eq!(f.text, "x");
}

/// Dvojklik vybere slovo, trojklik radek; pomaly druhy klik ne.
#[test]
fn double_and_triple_click() {
    let mut f = F::new("Žluťoučký kůň úpěl", "");
    let b = f.bounds(Role::TextInput, "Název");
    let (x, y) = (b.x + 20, b.y + b.h as i32 / 2);
    f.click(x, y, 1_000_000_000);
    f.click(x, y, 1_100_000_000);
    assert_eq!(f.edit("t").selected_text(), "Žluťoučký");
    f.click(x, y, 1_200_000_000);
    assert_eq!(f.edit("t").selected_text(), "Žluťoučký kůň úpěl");
    // Po prodleve zase jen kurzor.
    f.click(x, y, 3_000_000_000);
    assert!(f.edit("t").selection().is_none());
}

/// Heslo: kopirovani ani vyjmuti nic nedaji do schranky, dvojklik vybere
/// vse (slova by prozradila mezery), vlozeni funguje.
#[test]
fn password_never_copies() {
    let mut f = F::new("", "");
    let b = f.bounds(Role::PasswordInput, "Heslo");
    f.click(b.x + 4, b.y + 4, 1);
    f.type_str("tajne heslo");
    f.key_ch(KEY_A, Some('a'), CTRL);
    assert_eq!(f.key_ch(KEY_C, Some('c'), CTRL).clipboard, None);
    assert_eq!(f.key_ch(KEY_X, Some('x'), CTRL).clipboard, None);
    assert_eq!(f.key(KEY_INSERT, CTRL).clipboard, None);
    assert_eq!(f.pass, "tajne heslo", "vyjmuti hesla nic nesmaze");
    // Ctrl+sipky: zacatek a konec, ne po slovech.
    f.key(KEY_LEFT, CTRL);
    assert_eq!(f.edit("p").cursor(), 0);
    f.key(KEY_RIGHT, CTRL);
    assert_eq!(f.edit("p").cursor(), "tajne heslo".len());
    f.click(b.x + 4, b.y + 4, 5_000_000_000);
    f.click(b.x + 4, b.y + 4, 5_100_000_000);
    assert_eq!(f.edit("p").selected_text(), "tajne heslo");
    assert_eq!(f.key(KEY_INSERT, SHIFT).clipboard, Some(Clip::Paste));
}

/// Viceradkove pole: Enter vlozi radek, Ctrl+Enter odesle, dlouhy text
/// se zalomi, nahoru/dolu drzi polohu x, kolecko posune obsah.
#[test]
fn text_area_lines_and_scroll() {
    let mut f = F::new("", "");
    let b = f.bounds(Role::TextInput, "Poznámka");
    f.click(b.x + 4, b.y + 4, 1);
    f.type_str("první");
    f.key(KEY_ENTER, 0);
    f.type_str("druhý řádek");
    assert_eq!(f.note, "první\ndruhý řádek");
    f.key(KEY_ENTER, CTRL);
    assert_eq!(f.sent, 1);
    // Nahoru: z konce druheho radku na konec prvniho (kratsiho), dolu zpet
    // na puvodni x.
    let end = f.edit("n").cursor();
    f.key(KEY_UP, 0);
    assert_eq!(f.edit("n").cursor(), "první".len());
    f.key(KEY_DOWN, 0);
    assert_eq!(f.edit("n").cursor(), end);
    // Dlouhy odstavec se zalomi podle slov.
    let long = "slovo ".repeat(40);
    f.note = long.clone();
    f.rebuild();
    let i = f.ui.index_of_key("n");
    let lines = f.ui.field_lines(i);
    assert!(lines.len() > 3, "{lines:?}");
    for &(a, e) in &lines[..lines.len() - 1] {
        assert!(long[a..e].ends_with(' '), "zalomeni za slovem: {:?}", &long[a..e]);
    }
    // Ctrl+End: kurzor na konci, obsah posunuty dolu.
    f.key(KEY_END, CTRL);
    let key = f.ui.elems[i].key;
    let sy = f.ui.tscroll[&key].1;
    assert!(sy > 0);
    // Kolecko nahoru nad polem posune obsah, ne okno.
    f.ev(InputEvent::PointerMove { x: b.x + 10, y: b.y + 10 }, 0, None);
    let r = f.ev(InputEvent::Scroll { dx: 0, dy: 1 }, 0, None);
    assert!(r.scrolls.is_empty());
    assert!(f.ui.tscroll[&key].1 < sy);
}

/// Jednoradkove pole s dlouhym textem: End posune obsah, Home vrati.
#[test]
fn single_line_scrolls_to_caret() {
    let mut f = F::new(&"dlouhý text ".repeat(20), "");
    let b = f.bounds(Role::TextInput, "Název");
    f.click(b.x + 4, b.y + 4, 1);
    let i = f.ui.index_of_key("t");
    let key = f.ui.elems[i].key;
    f.key(KEY_END, 0);
    assert!(f.ui.tscroll[&key].0 > 0);
    f.key(KEY_HOME, 0);
    assert_eq!(f.ui.tscroll[&key].0, 0);
}

/// Aplikace hodnotu upravi (velka pismena): pole ji prevezme, kurzor
/// zustane na konci.
#[test]
fn app_transforms_value() {
    let mut f = F::new("", "");
    f.upper = true;
    let b = f.bounds(Role::TextInput, "Název");
    f.click(b.x + 4, b.y + 4, 1);
    f.type_str("abč");
    assert_eq!(f.text, "ABČ");
    assert_eq!(f.edit("t").text(), "ABČ");
    assert_eq!(f.edit("t").cursor(), "ABČ".len());
    let v: Vec<_> = f.ui.a11y().into_iter().filter(|n| n.value == "ABČ").collect();
    assert_eq!(v.len(), 1);
}

/// Vlastni preklad klaves (`key_map`): pole pise podle vybraneho rozlozeni
/// (QWERTZ: poloha Y = z), znak od systemu ignoruje, editace zustava.
#[test]
fn key_map_overrides_system_layout() {
    #[derive(Clone, Debug, PartialEq)]
    struct T(String);
    let mut u: Ui<T> = Ui::new(crate::Look::ui_fonts(), 400, 200);
    let view = |v: &str| column([text_input(v).on_change(T).key_map(|code, mods| match code {
        KEY_Y => Some(if mods & SHIFT != 0 { 'Z' } else { 'z' }),
        KEY_Z => Some('y'),
        KEY_2 => Some('ě'),
        _ => None,
    })]);
    let mut val = String::new();
    u.rebuild("k".into(), view(&val));
    for (code, ch, mods) in [(KEY_Y, 'y', 0), (KEY_Z, 'z', 0), (KEY_2, '2', 0), (KEY_Y, 'Y', SHIFT), (KEY_Q, 'q', 0)] {
        let r = u.event(InputEvent::Key { code, action: 1, ch: Some(ch) }, mods);
        // System posle i znak podle sveho rozlozeni: pole ho ignoruje.
        let r2 = u.event(InputEvent::Text(ch), mods);
        for m in r.msgs.into_iter().chain(r2.msgs) {
            val = m.0;
        }
        u.rebuild("k".into(), view(&val));
    }
    assert_eq!(val, "zyěZ");
    // Backspace dal maze.
    let r = u.event(InputEvent::Key { code: KEY_BACKSPACE, action: 1, ch: None }, 0);
    assert_eq!(r.msgs.last().map(|m| m.0.clone()), Some(String::from("zyě")));
}
