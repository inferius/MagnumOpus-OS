//! Tabulka: virtualizace, vyber, klavesy, razeni, tazeni, rolovani.

extern crate std;

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;

use gfx::Canvas;
use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::{CTRL, SHIFT};

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Sel(Vec<u64>),
    Open(u64),
    Sort(usize, bool),
    Drag(Vec<u64>),
    Menu(u64),
    Drop(Option<u64>, Vec<u64>),
}

/// Aplikace se seznamem souboru (`n` radku); `calls` pocita, kolikrat
/// tabulka cetla obsah radku.
struct F {
    names: Rc<Vec<String>>,
    sel: Vec<u64>,
    sort: Option<(usize, bool)>,
    multi: bool,
    calls: Rc<Cell<usize>>,
    msgs: Vec<M>,
    ui: Ui<M>,
}

impl F {
    fn new(n: usize) -> Self {
        let names = Rc::new((0..n).map(|i| alloc::format!("soubor {i:05}")).collect());
        let mut f = F { names, sel: Vec::new(), sort: None, multi: true, calls: Rc::new(Cell::new(0)), msgs: Vec::new(), ui: Ui::new(crate::Look::ui_fonts(), 500, 400) };
        f.rebuild();
        f
    }

    fn view(&self) -> Node<M> {
        let (names, calls) = (Rc::clone(&self.names), Rc::clone(&self.calls));
        let mut t = table(vec![col("Název", 0).sortable(), col("Velikost", 100).numeric().sortable()], self.names.len(), |i| 1000 + i as u64, move |i| {
            calls.set(calls.get() + 1);
            Cells::new([names[i].clone(), alloc::format!("{} kB", i)]).icon("file")
        })
        .selection(self.sel.clone())
        .multi_select(self.multi)
        .on_selection(M::Sel)
        .on_activate(M::Open)
        .on_sort(M::Sort)
        .on_drag(M::Drag)
        .on_context(M::Menu)
        .on_drop(M::Drop)
        .key("soubory");
        if let Some((c, a)) = self.sort {
            t = t.sorted(c, a);
        }
        column([t.height(300)])
    }

    fn rebuild(&mut self) {
        let v = self.view();
        self.ui.rebuild("t".into(), v);
    }

    fn ev(&mut self, e: InputEvent, mods: u16, t: Option<u64>) {
        let r = self.ui.event_at(e, mods, t);
        for m in r.msgs {
            match &m {
                M::Sel(s) => self.sel = s.clone(),
                M::Sort(c, a) => self.sort = Some((*c, *a)),
                _ => {}
            }
            self.msgs.push(m);
        }
        self.rebuild();
    }

    fn body(&self) -> Rect {
        let n = self.ui.a11y().into_iter().find(|n| n.role == Role::Table).unwrap();
        let rh = self.ui.look.row_h() as i32;
        Rect::new(n.bounds.x + 1, n.bounds.y + 1 + rh, n.bounds.w - 2, n.bounds.h - 2 - rh as u32)
    }

    /// Bod uprostred radku `k` (pri nulovem posunu).
    fn row_pt(&self, k: usize) -> (i32, i32) {
        let b = self.body();
        let rh = self.ui.look.row_h() as i32;
        (b.x + 40, b.y + k as i32 * rh + rh / 2)
    }

    fn click(&mut self, k: usize, mods: u16, t: u64) {
        let (x, y) = self.row_pt(k);
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, mods, Some(t));
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, mods, Some(t));
    }

    fn key(&mut self, code: u16, mods: u16) {
        self.ev(InputEvent::Key { code, action: 1, ch: None }, mods, None);
        self.ev(InputEvent::Key { code, action: 0, ch: None }, mods, None);
    }

    fn scroll(&self) -> (i32, i32) {
        let s = &self.ui.tables[&self.ui.elems[self.ui.elems.iter().position(|e| matches!(e.kind, crate::layout::ElemKind::Table(_))).unwrap()].key];
        (s.scroll, s.target)
    }
}

/// Deset tisic radku: kresleni i strom pristupnosti ctou jen viditelne.
#[test]
fn virtualized_draw_and_a11y() {
    let mut f = F::new(10_000);
    let (w, h) = (500u32, 400u32);
    let mut px = vec![0u32; (w * h) as usize];
    let mut c = Canvas::new(&mut px, w, h, w).unwrap();
    f.calls.set(0);
    f.ui.draw(&mut c, 0);
    let visible = (300 / f.ui.look.row_h()) as usize + 2;
    assert!(f.calls.get() <= visible, "kresleni cetlo {} radku", f.calls.get());
    let nodes = f.ui.a11y();
    let rows = nodes.iter().filter(|n| n.role == Role::Row).count();
    assert!(rows <= visible && rows > 3, "{rows}");
    let t = nodes.iter().find(|n| n.role == Role::Table).unwrap();
    assert_eq!(t.value, "10000 řádků");
    assert_eq!(nodes.iter().filter(|n| n.role == Role::ColumnHeader).count(), 2);
    // Ctrl+End: kurzor na posledni radek, ten je videt (posun hned).
    f.click(0, 0, 1);
    f.key(KEY_END, CTRL);
    f.key(KEY_SPACE, 0);
    assert_eq!(f.sel, vec![1000 + 9999]);
    let (s, t) = f.scroll();
    assert_eq!(s, t);
    assert!(s > 0);
    assert!(f.ui.a11y().iter().any(|n| n.role == Role::Row && n.name.starts_with("soubor 09999")));
}

/// Klik, Ctrl+klik, Shift+klik, Ctrl+A, Shift+sipky.
#[test]
fn mouse_and_keyboard_selection() {
    let mut f = F::new(50);
    f.click(2, 0, 1_000_000_000);
    assert_eq!(f.sel, vec![1002]);
    f.click(4, CTRL, 3_000_000_000);
    assert_eq!(f.sel, vec![1002, 1004]);
    f.click(4, CTRL, 5_000_000_000);
    assert_eq!(f.sel, vec![1002]);
    // Shift: rozsah od kotvy (posledni klik bez Shift = radek 4).
    f.click(6, SHIFT, 7_000_000_000);
    assert_eq!(f.sel, vec![1004, 1005, 1006]);
    f.key(KEY_DOWN, SHIFT);
    assert_eq!(f.sel, vec![1004, 1005, 1006, 1007]);
    // Ctrl+sipka jen kurzor, Ctrl+mezernik prepne.
    f.key(KEY_DOWN, CTRL);
    f.key(KEY_DOWN, CTRL);
    f.key(KEY_SPACE, CTRL);
    assert_eq!(f.sel, vec![1004, 1005, 1006, 1007, 1009]);
    f.ev(InputEvent::Key { code: KEY_A, action: 1, ch: Some('a') }, CTRL, None);
    assert_eq!(f.sel.len(), 50);
    // Jednoduchy vyber: Ctrl+klik vybere jen radek.
    f.multi = false;
    f.rebuild();
    // Home vrati posun na zacatek (kurzor na radku 9 tabulku posunul).
    f.key(KEY_HOME, 0);
    assert_eq!(f.scroll(), (0, 0));
    f.click(1, CTRL, 9_000_000_000);
    assert_eq!(f.sel, vec![1001]);
    // Stav vybrani pro ctecku.
    let r = f.ui.a11y().into_iter().find(|n| n.role == Role::Row && n.name.starts_with("soubor 00001")).unwrap();
    assert!(r.state & state::SELECTED != 0);
}

/// Dvojklik a Enter aktivuji, prave tlacitko kontextova nabidka.
#[test]
fn activate_and_context() {
    let mut f = F::new(20);
    f.click(3, 0, 1_000_000_000);
    f.click(3, 0, 1_200_000_000);
    assert!(f.msgs.contains(&M::Open(1003)));
    f.key(KEY_DOWN, 0);
    f.key(KEY_ENTER, 0);
    assert!(f.msgs.contains(&M::Open(1004)));
    let (x, y) = f.row_pt(7);
    f.ev(InputEvent::Button { code: BTN_RIGHT, pressed: true, x, y }, 0, None);
    assert_eq!(f.sel, vec![1007], "prave tlacitko vybere radek");
    assert!(f.msgs.contains(&M::Menu(1007)));
}

/// Klik na zahlavi radi, znovu opacne; tah za okraj meni sirku.
#[test]
fn sort_and_resize_columns() {
    let mut f = F::new(20);
    let h: Vec<_> = f.ui.a11y().into_iter().filter(|n| n.role == Role::ColumnHeader).collect();
    let (x, y) = (h[1].bounds.x + 30, h[1].bounds.y + 5);
    for _ in 0..2 {
        f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, None);
        f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, 0, None);
    }
    assert_eq!(f.msgs, vec![M::Sort(1, true), M::Sort(1, false)]);
    assert_eq!(f.ui.a11y().into_iter().find(|n| n.role == Role::ColumnHeader && n.name == "Velikost").unwrap().value, "sestupně");
    // Okraj prvniho sloupce: tah doleva o 50 px.
    let edge = h[0].bounds.x + h[0].bounds.w as i32;
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: edge, y }, 0, None);
    f.ev(InputEvent::PointerMove { x: edge - 50, y }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: edge - 50, y }, 0, None);
    let w0 = f.ui.a11y().into_iter().find(|n| n.role == Role::ColumnHeader && n.name == "Název").unwrap().bounds.w;
    assert_eq!(w0, h[0].bounds.w - 50);
    assert_eq!(f.msgs.len(), 2, "zmena sirky neradi");
    // Akce pristupnosti na zahlavi radi taky.
    let id = f.ui.a11y().into_iter().find(|n| n.role == Role::ColumnHeader && n.name == "Název").unwrap().id;
    let r = f.ui.a11y_action(id, actions::ACTIVATE, "");
    assert_eq!(r.msgs, vec![M::Sort(0, true)]);
}

/// Tah vybranych radku: `on_drag` s jejich id, vyber se nezmeni; stisk na
/// vybranem radku bez tahu vybere jen ten radek.
#[test]
fn drag_selected_rows() {
    let mut f = F::new(20);
    f.click(1, 0, 1_000_000_000);
    f.click(3, SHIFT, 3_000_000_000);
    assert_eq!(f.sel, vec![1001, 1002, 1003]);
    let (x, y) = f.row_pt(2);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, Some(5_000_000_000));
    f.ev(InputEvent::PointerMove { x: x + 3, y }, 0, None);
    assert!(!f.msgs.iter().any(|m| matches!(m, M::Drag(_))), "pod prahem neni tah");
    f.ev(InputEvent::PointerMove { x: x + 20, y: y + 10 }, 0, None);
    f.ev(InputEvent::PointerMove { x: x + 40, y: y + 20 }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: x + 40, y: y + 20 }, 0, None);
    assert_eq!(f.msgs.iter().filter(|m| matches!(m, M::Drag(_))).count(), 1);
    assert!(f.msgs.contains(&M::Drag(vec![1001, 1002, 1003])));
    assert_eq!(f.sel, vec![1001, 1002, 1003]);
    f.click(2, 0, 9_000_000_000);
    assert_eq!(f.sel, vec![1002]);
}

/// Kolecko roluje plynule (cil hned, posun po snimcich), na kraji pusti
/// kolecko dal; posuvnik tazenim a klikem do drahy.
#[test]
fn smooth_wheel_and_scrollbar() {
    let mut f = F::new(500);
    let (x, y) = f.row_pt(2);
    f.ev(InputEvent::PointerMove { x, y }, 0, None);
    f.ev(InputEvent::Scroll { dx: 0, dy: -1 }, 0, None);
    let (s, t) = f.scroll();
    assert_eq!(s, 0);
    assert!(t > 0);
    assert!(f.ui.wants_frame());
    let mut n = 0;
    while f.ui.tick() {
        n += 1;
        assert!(n < 30);
    }
    assert!(n > 1, "vic snimku = plynule");
    assert_eq!(f.scroll().0, t);
    assert!(!f.ui.wants_frame());
    // Nahoru za zacatek: tabulka kolecko nezpracuje.
    for _ in 0..5 {
        f.ev(InputEvent::Scroll { dx: 0, dy: 1 }, 0, None);
    }
    while f.ui.tick() {}
    assert_eq!(f.scroll().0, 0);
    let r = f.ui.event(InputEvent::Scroll { dx: 0, dy: 1 }, 0);
    assert!(r.scrolls.is_empty() && !r.redraw);
    // Posuvnik: tah jezdcem dolu.
    let b = f.body();
    let tx = b.right() as i32 - 6;
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: tx, y: b.y + 5 }, 0, None);
    f.ev(InputEvent::PointerMove { x: tx, y: b.y + 105 }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: tx, y: b.y + 105 }, 0, None);
    let (s, t) = f.scroll();
    assert!(s > 0 && s == t, "tah jezdcem posune hned");
    assert!(f.sel.is_empty(), "posuvnik nevybira");
    // Klik do drahy pod jezdcem: o stranku.
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: tx, y: b.bottom() as i32 - 4 }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: tx, y: b.bottom() as i32 - 4 }, 0, None);
    assert_eq!(f.scroll().1, s + b.h as i32);
}

/// Psani skoci na radek, ktery zacina napsanym textem.
#[test]
fn type_to_find() {
    let mut f = F::new(300);
    f.click(0, 0, 1);
    for (i, ch) in "soubor 00250".chars().enumerate() {
        f.ev(InputEvent::Text(ch), 0, Some(10_000_000 + i as u64 * 100_000_000));
    }
    assert_eq!(f.sel, vec![1250]);
    let (s, _) = f.scroll();
    assert!(s > 0, "radek je videt");
    // Akce pristupnosti na radku ho vybere.
    let row = f.ui.a11y().into_iter().find(|n| n.role == Role::Row && n.name.starts_with("soubor 00251")).unwrap();
    let r = f.ui.a11y_action(row.id, actions::ACTIVATE, "");
    assert_eq!(r.msgs, vec![M::Sel(vec![1251])]);
}

/// Klavesy, ktere tabulka nespotrebuje (F2, Delete, Ctrl+C, Alt+sipka),
/// dostane aplikace jako zkratky; sipky a Enter s `on_activate` ne.
#[test]
fn unconsumed_keys_go_to_app() {
    let mut f = F::new(20);
    f.click(1, 0, 1);
    let un = |f: &mut F, code: u16, ch: Option<char>, mods: u16| f.ui.event(InputEvent::Key { code, action: 1, ch }, mods).unhandled_key;
    assert_eq!(un(&mut f, KEY_F2, None, 0), Some((KEY_F2, None, 0)));
    assert_eq!(un(&mut f, KEY_DELETE, None, 0), Some((KEY_DELETE, None, 0)));
    assert_eq!(un(&mut f, KEY_C, Some('c'), CTRL), Some((KEY_C, Some('c'), CTRL)));
    assert_eq!(un(&mut f, KEY_LEFT, None, inputproto::mods::ALT), Some((KEY_LEFT, None, inputproto::mods::ALT)));
    assert_eq!(un(&mut f, KEY_DOWN, None, 0), None);
    assert_eq!(un(&mut f, KEY_ENTER, None, 0), None);
    // Pusteni klavesy se nehlasi.
    assert_eq!(f.ui.event(InputEvent::Key { code: KEY_F2, action: 0, ch: None }, 0).unhandled_key, None);
}

/// Pusteni tazenych radku: na radek `on_drop(Some(id), ids)`, na sebe
/// sama nic, cilovy radek je pri tahu zvyrazneny.
#[test]
fn drop_rows_on_row() {
    let mut f = F::new(20);
    f.click(1, 0, 1_000_000_000);
    f.click(2, SHIFT, 3_000_000_000);
    let (x, y) = f.row_pt(1);
    let (_, ty) = f.row_pt(5);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, Some(5_000_000_000));
    f.ev(InputEvent::PointerMove { x, y: y + 20 }, 0, None);
    f.ev(InputEvent::PointerMove { x, y: ty }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y: ty }, 0, None);
    assert!(f.msgs.contains(&M::Drop(Some(1005), vec![1001, 1002])), "{:?}", f.msgs);
    // Na sebe sama: nic.
    f.msgs.clear();
    let (_, y2) = f.row_pt(2);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, Some(9_000_000_000));
    f.ev(InputEvent::PointerMove { x, y: y + 20 }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y: y2 }, 0, None);
    assert!(!f.msgs.iter().any(|m| matches!(m, M::Drop(..))), "{:?}", f.msgs);
}
