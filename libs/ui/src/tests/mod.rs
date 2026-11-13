//! Testy knihovny UI (na hostiteli: `cargo test -p ui`).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Canvas;
use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum Msg {
    Inc,
    Title(String),
    Important(bool),
    Select(usize),
    Save,
}

struct Demo {
    count: u32,
    title: String,
    important: bool,
    items: Vec<String>,
    selected: Option<usize>,
    saved: u32,
}

impl Demo {
    fn new() -> Self {
        Self {
            count: 0,
            title: String::new(),
            important: false,
            items: (1..=20).map(|i| format!("Položka {i}")).collect(),
            selected: None,
            saved: 0,
        }
    }
}

impl App for Demo {
    type Msg = Msg;

    fn title(&self) -> String {
        "Ukázka".into()
    }

    fn view(&self, _cx: &Context) -> Node<Msg> {
        column([
            label(&format!("Počet: {}", self.count)).heading(),
            row([button("Přidat").on_click(Msg::Inc), button("Uložit").on_click(Msg::Save), spacer()]),
            text_input(&self.title).placeholder("Název").on_change(Msg::Title).on_submit(Msg::Save),
            checkbox("Důležitá", self.important).on_toggle(Msg::Important),
            scroll(list(self.items.clone(), self.selected).on_select(Msg::Select)).height(120),
            progress(self.count * 10),
        ])
    }

    fn update(&mut self, m: Msg) {
        match m {
            Msg::Inc => self.count += 1,
            Msg::Title(t) => self.title = t,
            Msg::Important(b) => self.important = b,
            Msg::Select(i) => self.selected = Some(i),
            Msg::Save => self.saved += 1,
        }
    }
}

struct H {
    app: Demo,
    ui: Ui<Msg>,
}

impl H {
    fn new() -> Self {
        let app = Demo::new();
        let mut ui = Ui::new(crate::Look::ui_fonts(), 420, 400);
        ui.refresh(&app);
        Self { app, ui }
    }

    fn apply(&mut self, r: event::Response<Msg>) -> event::Response<Msg> {
        for m in r.msgs.iter().cloned() {
            self.app.update(m);
        }
        self.ui.refresh(&self.app);
        r
    }

    fn ev(&mut self, e: InputEvent) -> event::Response<Msg> {
        let r = self.ui.event(e, 0);
        self.apply(r)
    }

    /// Stisk a pusteni klavesy.
    fn key(&mut self, code: u16) {
        self.ev(InputEvent::Key { code, action: 1, ch: None });
        self.ev(InputEvent::Key { code, action: 0, ch: None });
    }

    fn node(&self, role: Role, name: &str) -> guiproto::a11y::Node {
        self.ui.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap()
    }

    fn click_node(&mut self, role: Role, name: &str) {
        let n = self.node(role, name);
        let (x, y) = (n.bounds.x + 5, n.bounds.y + 5);
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y });
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y });
    }
}

#[test]
fn layout_places_elements_inside_window() {
    let h = H::new();
    let nodes = h.ui.a11y();
    assert_eq!(nodes[0].role, Role::Window);
    assert_eq!(nodes[0].name, "Ukázka");
    let add = h.node(Role::Button, "Přidat");
    let save = h.node(Role::Button, "Uložit");
    assert_eq!(add.bounds.y, save.bounds.y, "tlacitka v radku");
    assert!(save.bounds.x > add.bounds.x + add.bounds.w as i32);
    let input = nodes.iter().find(|n| n.role == Role::TextInput).unwrap();
    assert_eq!(input.name, "Název");
    assert!(input.bounds.w > 300, "pole roste na sirku sloupce");
    for n in &nodes[1..] {
        if n.layer == ROOT_LAYER {
            assert!(n.bounds.right() <= 420 && n.bounds.bottom() <= 400, "{n:?}");
        }
    }
    // Polozky seznamu jsou v prostoru posouvane oblasti.
    let item = h.node(Role::ListItem, "Položka 20");
    assert_ne!(item.layer, ROOT_LAYER);
    assert!(item.bounds.y > 120, "obsah je vyssi nez oblast");
}

#[test]
fn click_button_and_focus_by_tab() {
    let mut h = H::new();
    h.click_node(Role::Button, "Přidat");
    assert_eq!(h.app.count, 1);
    // Prvni fokus je prvni tlacitko; Tab -> Ulozit, Enter.
    let first = h.ui.a11y().into_iter().find(|n| n.state & state::FOCUSED != 0).unwrap();
    assert_eq!(first.name, "Přidat");
    h.key(KEY_TAB);
    h.key(KEY_ENTER);
    assert_eq!(h.app.saved, 1);
    // Shift+Tab zpet.
    let r = h.ui.event(InputEvent::Key { code: KEY_TAB, action: 1, ch: None }, inputproto::mods::SHIFT);
    h.apply(r);
    h.key(KEY_SPACE);
    assert_eq!(h.app.count, 2);
    // Uvolneni mimo tlacitko nic.
    let b = h.node(Role::Button, "Přidat");
    h.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: b.bounds.x + 2, y: b.bounds.y + 2 });
    h.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: 1, y: 1 });
    assert_eq!(h.app.count, 2);
}

#[test]
fn text_input_editing_with_diacritics() {
    let mut h = H::new();
    let n = h.ui.a11y().into_iter().find(|n| n.role == Role::TextInput).unwrap();
    h.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: n.bounds.x + 5, y: n.bounds.y + 5 });
    h.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: n.bounds.x + 5, y: n.bounds.y + 5 });
    for ch in "Čaj".chars() {
        h.ev(InputEvent::Text(ch));
    }
    assert_eq!(h.app.title, "Čaj");
    h.key(KEY_LEFT);
    h.key(KEY_BACKSPACE);
    assert_eq!(h.app.title, "Čj");
    h.key(KEY_HOME);
    h.ev(InputEvent::Text('ž'));
    assert_eq!(h.app.title, "žČj");
    h.key(KEY_END);
    h.key(KEY_DELETE);
    assert_eq!(h.app.title, "žČj");
    h.key(KEY_ENTER);
    assert_eq!(h.app.saved, 1);
    let v = h.ui.a11y().into_iter().find(|n| n.role == Role::TextInput).unwrap();
    assert_eq!(v.value, "žČj");
}

#[test]
fn checkbox_list_and_a11y_actions() {
    let mut h = H::new();
    h.click_node(Role::Checkbox, "Důležitá");
    assert!(h.app.important);
    assert!(h.node(Role::Checkbox, "Důležitá").state & state::CHECKED != 0);
    // Akce pristupnosti bez souradnic.
    let cb = h.node(Role::Checkbox, "Důležitá").id;
    let r = h.ui.a11y_action(cb, actions::TOGGLE, "");
    h.apply(r);
    assert!(!h.app.important);
    let item = h.node(Role::ListItem, "Položka 3").id;
    let r = h.ui.a11y_action(item, actions::ACTIVATE, "");
    h.apply(r);
    assert_eq!(h.app.selected, Some(2));
    let input = h.ui.a11y().into_iter().find(|n| n.role == Role::TextInput).unwrap().id;
    let r = h.ui.a11y_action(input, actions::SET_VALUE, "Nákup");
    h.apply(r);
    assert_eq!(h.app.title, "Nákup");
    // Sipky v seznamu s fokusem.
    let list = h.ui.a11y().into_iter().find(|n| n.role == Role::List).unwrap().id;
    let r = h.ui.a11y_action(list, actions::FOCUS, "");
    h.apply(r);
    h.key(KEY_DOWN);
    assert_eq!(h.app.selected, Some(3));
    h.key(KEY_UP);
    h.key(KEY_UP);
    assert_eq!(h.app.selected, Some(1));
    // Neznamy uzel nic.
    let r = h.ui.a11y_action(9999, actions::ACTIVATE, "");
    assert!(r.msgs.is_empty());
}

#[test]
fn wheel_scrolls_without_redraw_and_hit_uses_offset() {
    let mut h = H::new();
    let area = h.ui.scroll_areas()[0];
    assert!(area.content_h > area.rect.h);
    h.ev(InputEvent::PointerMove { x: area.rect.x + 10, y: area.rect.y + 10 });
    let r = h.ev(InputEvent::Scroll { dx: 0, dy: -1 });
    assert_eq!(r.scrolls, vec![(area.id, 40)]);
    assert!(r.msgs.is_empty());
    // Klik na prvni viditelny radek = polozka posunuta o 40 px.
    let y = area.rect.y + 5;
    h.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: area.rect.x + 10, y });
    h.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: area.rect.x + 10, y });
    let row = h.ui.look.row_h() as i32;
    let inset = h.ui.look.theme.space[2] as i32;
    assert_eq!(h.app.selected, Some(((40 + 5 - inset) / row) as usize));
    // Posun nejvys na konec obsahu.
    for _ in 0..50 {
        h.ev(InputEvent::Scroll { dx: 0, dy: -1 });
    }
    let a = h.ui.scroll_areas()[0];
    assert_eq!(a.offset, a.content_h as i32 - a.rect.h as i32);
}

#[test]
fn render_window_snapshot() {
    let mut h = H::new();
    h.app.title = "Nákup na sobotu".into();
    h.app.important = true;
    h.app.count = 4;
    h.app.selected = Some(1);
    h.ui.refresh(&h.app);
    let (w, hh) = (420u32, 400u32);
    let mut px = vec![0u32; (w * hh) as usize];
    let mut c = Canvas::new(&mut px, w, hh, w).unwrap();
    h.ui.draw(&mut c, 0);
    // Pozadi a tlacitko maji barvy motivu.
    let col = h.ui.look.theme.color.clone();
    assert_eq!(c.get(2, 2), col.surface);
    let b = h.node(Role::Button, "Uložit").bounds;
    assert_eq!(c.get(b.x as u32 + b.w / 2, b.y as u32 + 2), col.surface);
    assert_eq!(c.get(b.x as u32 + b.w / 2, b.y as u32), col.border_strong);
    // Obsah posouvane oblasti do vlastniho bufferu.
    let a = h.ui.scroll_areas()[0];
    let mut spx = vec![0u32; (a.rect.w * a.content_h) as usize];
    let mut sc = Canvas::new(&mut spx, a.rect.w, a.content_h, a.rect.w).unwrap();
    h.ui.draw(&mut sc, a.index + 1);
    // Vybrana polozka 2 (index 1) ma barvu vyberu.
    let row = h.ui.look.row_h();
    assert_eq!(sc.get(14, row + h.ui.look.theme.space[2] + 4), col.accent_soft);
    // Slozeni pro kontrolu okem.
    let mut comp = px.clone();
    let mut cc = Canvas::new(&mut comp, w, hh, w).unwrap();
    let img = gfx::Image::new(&spx, a.rect.w, a.content_h, a.rect.w).unwrap();
    cc.draw_image(&img, Rect::new(0, 0, a.rect.w, a.rect.h), a.rect, 255, true);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let _ = std::fs::write(dir.join("ui-window.png"), gfx::png::encode(&comp, w, hh, w));
}

/// Data ze sluzby mezi stiskem a pustenim zmeni pocet radku seznamu
/// (indexy prvku za nim se posunou), tlacitko zustane na miste: klik se
/// neztrati (spravce uloh nad taskd).
#[test]
fn click_survives_relayout_between_press_and_release() {
    let view = |rows: usize| -> Node<Msg> {
        column([
            scroll(list((0..rows).map(|i| alloc::format!("radek {i}")), None)).height(120),
            button("Pozastavit").on_click(Msg::Inc),
        ])
    };
    let mut ui: Ui<Msg> = Ui::new(crate::Look::ui_fonts(), 400, 300);
    ui.rebuild("t".into(), view(3));
    let b = ui.a11y().into_iter().find(|n| n.role == Role::Button).unwrap().bounds;
    let (x, y) = (b.x + 5, b.y + 5);
    ui.event(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0);
    ui.rebuild("t".into(), view(5));
    let r = ui.event(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, 0);
    assert_eq!(r.msgs.len(), 1, "klik po prerozvrzeni");
}

#[test]
fn enter_and_space_activate_on_release() {
    let mut h = H::new();
    // Fokus na prvnim tlacitku (Pridat): stisk Enter sam nic.
    h.ev(InputEvent::Key { code: KEY_ENTER, action: 1, ch: None });
    assert_eq!(h.app.count, 0, "akce az pri pusteni");
    h.ev(InputEvent::Key { code: KEY_ENTER, action: 0, ch: None });
    assert_eq!(h.app.count, 1);
    // Mys: stisk na tlacitku, odjeti mimo, pusteni = nic.
    let b = h.node(Role::Button, "Přidat").bounds;
    let (x, y) = (b.x + 3, b.y + 3);
    h.ev(InputEvent::PointerMove { x, y });
    h.ev(InputEvent::Button { code: inputproto::code::BTN_LEFT, pressed: true, x, y });
    h.ev(InputEvent::PointerMove { x: x + 300, y: y + 300 });
    h.ev(InputEvent::Button { code: inputproto::code::BTN_LEFT, pressed: false, x: x + 300, y: y + 300 });
    assert_eq!(h.app.count, 1, "pusteni mimo tlacitko akci zrusi");
}

mod context;
mod controls;
mod field;
mod keys;
mod popup;
mod table;
mod text;
