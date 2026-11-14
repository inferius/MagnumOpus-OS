//! Seznam s vicenasobnym vyberem (`multi_select`): stejna logika jako
//! tabulka (klik, Ctrl+klik, Shift+klik, Shift+sipky, Ctrl+sipky jen
//! kurzor, Ctrl+mezernik, Ctrl+A, hledani psanim).

use alloc::string::String;
use alloc::vec::Vec;

use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::{CTRL, SHIFT};

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Sel(Vec<u64>),
}

struct A {
    sel: Vec<u64>,
    ui: Ui<M>,
}

const ITEMS: [&str; 6] = ["Banán", "Borůvka", "Citron", "Hruška", "Jablko", "Švestka"];

impl A {
    fn new() -> Self {
        let mut a = A { sel: Vec::new(), ui: Ui::new(crate::Look::ui_fonts(), 400, 500) };
        a.rebuild();
        let l = a.list();
        a.ui.focus = Some(l);
        a
    }

    fn rebuild(&mut self) {
        let v = column([list(ITEMS.iter().map(|s| String::from(*s)), None).multi_select(true).selection(self.sel.clone()).on_selection(M::Sel).key("l")]);
        self.ui.rebuild("a".into(), v);
    }

    fn list(&self) -> usize {
        self.ui.index_of(Key::ROOT.mix(1).mix(Key::from("l").0)).unwrap()
    }

    fn apply(&mut self, r: event::Response<M>) {
        for m in r.msgs {
            let M::Sel(s) = m;
            self.sel = s;
            self.sel.sort_unstable();
        }
        self.rebuild();
    }

    fn key(&mut self, code: u16, ch: Option<char>, mods: u16) {
        let r = self.ui.event_at(InputEvent::Key { code, action: 1, ch }, mods, Some(1));
        self.apply(r);
        let r = self.ui.event_at(InputEvent::Key { code, action: 0, ch }, mods, Some(1));
        self.apply(r);
    }

    fn click(&mut self, k: usize, mods: u16) {
        let item = self.ui.elems.iter().find(|e| matches!(e.kind, layout::ElemKind::ListItem { index, .. } if index == k)).unwrap().rect;
        let (x, y) = (item.x + 10, item.y + item.h as i32 / 2);
        for down in [true, false] {
            let r = self.ui.event_at(InputEvent::Button { code: BTN_LEFT, pressed: down, x, y }, mods, Some(1));
            self.apply(r);
        }
    }

    fn selected_items(&self) -> Vec<usize> {
        self.ui.elems.iter().filter_map(|e| match e.kind {
            layout::ElemKind::ListItem { index, selected: true, .. } => Some(index),
            _ => None,
        }).collect()
    }
}

#[test]
fn list_mouse_ctrl_shift() {
    let mut a = A::new();
    a.click(1, 0);
    assert_eq!(a.sel, [1]);
    a.click(3, CTRL);
    assert_eq!(a.sel, [1, 3]);
    a.click(1, CTRL);
    assert_eq!(a.sel, [3], "Ctrl+klik odebere");
    a.click(5, SHIFT);
    assert_eq!(a.sel, [1, 2, 3, 4, 5], "rozsah od kotvy = posledni klik, i Ctrl+klik (Windows)");
    a.click(0, 0);
    assert_eq!(a.sel, [0]);
    assert_eq!(a.selected_items(), [0], "radky ukazuji vyber");
}

#[test]
fn list_keys_shift_ctrl_space_and_select_all() {
    let mut a = A::new();
    a.key(KEY_DOWN, None, 0);
    assert_eq!(a.sel, [0]);
    a.key(KEY_DOWN, None, SHIFT);
    a.key(KEY_DOWN, None, SHIFT);
    assert_eq!(a.sel, [0, 1, 2], "Shift+sipky rozsah");
    // Ctrl+sipka jen kurzor, Ctrl+mezernik prepne.
    a.key(KEY_DOWN, None, CTRL);
    a.key(KEY_DOWN, None, CTRL);
    assert_eq!(a.sel, [0, 1, 2]);
    a.key(KEY_SPACE, Some(' '), CTRL);
    assert_eq!(a.sel, [0, 1, 2, 4]);
    a.key(KEY_SPACE, Some(' '), CTRL);
    assert_eq!(a.sel, [0, 1, 2]);
    a.key(KEY_A, Some('a'), CTRL);
    assert_eq!(a.sel, [0, 1, 2, 3, 4, 5], "Ctrl+A");
    a.key(KEY_HOME, None, 0);
    assert_eq!(a.sel, [0]);
    a.key(KEY_END, None, SHIFT);
    assert_eq!(a.sel, [0, 1, 2, 3, 4, 5], "Shift+End");
    // Hledani psanim vybere jen nalezeny radek.
    a.key(KEY_H, Some('h'), 0);
    assert_eq!(a.sel, [3]);
}
