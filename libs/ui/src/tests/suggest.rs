//! Seznam navrhu u pole (combobox): ve strome `Ui` i samostatne.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::SHIFT;

use crate::complete::{Completion, MAX_ITEMS, Outcome, Paint, Side};
use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Q(String),
    Pick(usize),
    Sel(usize),
    Closed,
    Go,
}

const PATHS: [(&str, &str); 4] = [("/home/", "složka"), ("/hosts", "soubor"), ("/hw/", "složka"), ("/hra", "prikaz")];

struct F {
    q: String,
    secure: bool,
    msgs: Vec<M>,
    ui: Ui<M>,
}

impl F {
    fn new() -> Self {
        let mut f = F { q: String::from("/h"), secure: false, msgs: Vec::new(), ui: Ui::new(crate::Look::ui_fonts(), 500, 400) };
        f.rebuild();
        f
    }

    fn items(&self) -> Vec<Suggestion> {
        PATHS
            .iter()
            .filter(|(p, _)| p.starts_with(self.q.as_str()) && *p != self.q)
            .map(|(p, t)| {
                let s = Suggestion::new(p, t).icon("folder");
                if *p == "/hw/" { s.disabled("bez práv") } else { s }
            })
            .collect()
    }

    fn rebuild(&mut self) {
        let field = if self.secure { password_input(&self.q) } else { text_input(&self.q) };
        let v = column([
            field.on_change(M::Q).suggest(suggest("Cesty", self.items(), M::Pick).on_select(M::Sel).on_close(M::Closed)).key("pole"),
            button("Spustit").on_click(M::Go).key("go"),
        ]);
        self.ui.rebuild("s".into(), v);
    }

    fn ev(&mut self, e: InputEvent, mods: u16) {
        let r = self.ui.event_at(e, mods, None);
        for m in r.msgs {
            match &m {
                M::Q(q) => self.q = q.clone(),
                M::Pick(k) => {
                    let it = self.items();
                    self.q = it[*k].name.clone();
                }
                _ => {}
            }
            self.msgs.push(m);
        }
        self.rebuild();
    }

    fn key(&mut self, code: u16, mods: u16) {
        self.ev(InputEvent::Key { code, action: 1, ch: None }, mods);
        self.ev(InputEvent::Key { code, action: 0, ch: None }, mods);
    }

    fn click(&mut self, x: i32, y: i32) {
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0);
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, 0);
    }

    fn options(&self) -> Vec<guiproto::a11y::Node> {
        self.ui.a11y().into_iter().filter(|n| n.role == Role::ListOption).collect()
    }
}

#[test]
fn suggest_opens_under_focused_field_and_typing_goes_to_field() {
    let mut f = F::new();
    assert!(f.ui.suggest_open(), "pole ma fokus a polozky");
    let field = f.ui.a11y().into_iter().find(|n| n.role == Role::TextInput).unwrap();
    assert_eq!(field.state & (state::AUTOCOMPLETE | state::EXPANDED), state::AUTOCOMPLETE | state::EXPANDED);
    let list = f.ui.overlay_rect().expect("seznam ve vrstve popoveru");
    assert!(list.y as i64 >= field.bounds.bottom(), "pod polem");
    // Psani jde do pole, seznam zustava a filtruje se.
    f.ev(InputEvent::Text('o'), 0);
    assert_eq!(f.q, "/ho");
    assert!(f.ui.suggest_open());
    assert_eq!(f.options().len(), 2);
}

#[test]
fn suggest_arrows_move_selection_without_leaving_field() {
    let mut f = F::new();
    let sel = |f: &F| f.options().iter().position(|n| n.state & state::ACTIVE != 0);
    assert_eq!(sel(&f), Some(0));
    f.key(KEY_DOWN, 0);
    assert_eq!(sel(&f), Some(1));
    assert_eq!(f.msgs, vec![M::Sel(1)]);
    // Zakazana polozka se preskoci, dolu za koncem dokola.
    f.key(KEY_DOWN, 0);
    assert_eq!(sel(&f), Some(3));
    f.key(KEY_DOWN, 0);
    assert_eq!(sel(&f), Some(0));
    f.key(KEY_UP, 0);
    assert_eq!(sel(&f), Some(3));
    f.key(KEY_HOME, 0);
    assert_eq!(sel(&f), Some(0));
    f.key(KEY_END, 0);
    assert_eq!(sel(&f), Some(3));
    // Pole porad ma fokus a hodnotu.
    assert_eq!(f.q, "/h");
    let field = f.ui.a11y().into_iter().find(|n| n.role == Role::TextInput).unwrap();
    assert!(field.state & state::FOCUSED != 0);
}

#[test]
fn suggest_enter_accepts_and_esc_closes_until_items_change() {
    let mut f = F::new();
    f.key(KEY_DOWN, 0);
    f.key(KEY_ENTER, 0);
    assert_eq!(f.q, "/hosts");
    assert!(f.msgs.contains(&M::Pick(1)) && f.msgs.contains(&M::Closed));
    // "/hosts" uz nema navrhy.
    assert!(!f.ui.suggest_open());
    f.q = String::from("/h");
    f.rebuild();
    assert!(f.ui.suggest_open());
    f.msgs.clear();
    f.key(KEY_ESC, 0);
    assert_eq!(f.msgs, vec![M::Closed]);
    assert!(!f.ui.suggest_open());
    // Stejne polozky: zustava zavreny; Enter pak patri poli.
    f.rebuild();
    assert!(!f.ui.suggest_open());
    // Jine polozky (psani) ho otevrou znovu.
    f.ev(InputEvent::Text('r'), 0);
    assert!(f.ui.suggest_open());
}

#[test]
fn suggest_tab_accepts_shift_tab_leaves() {
    let mut f = F::new();
    f.key(KEY_TAB, 0);
    assert_eq!(f.q, "/home/");
    let mut f = F::new();
    f.key(KEY_TAB, SHIFT);
    assert!(!f.msgs.iter().any(|m| matches!(m, M::Pick(_))));
}

#[test]
fn suggest_mouse_hover_click_and_click_outside() {
    let mut f = F::new();
    let o = f.options();
    let (x, y) = (o[3].bounds.x + 10, o[3].bounds.y + 5);
    f.ev(InputEvent::PointerMove { x, y }, 0);
    assert!(f.options()[3].state & state::HOVER != 0);
    // Zakazana polozka nejde vybrat ani najetim.
    let d = &f.options()[2];
    f.ev(InputEvent::PointerMove { x: d.bounds.x + 10, y: d.bounds.y + 5 }, 0);
    assert!(f.options()[2].state & state::HOVER == 0);
    f.click(x, y);
    assert_eq!(f.q, "/hra");
    assert!(f.msgs.contains(&M::Pick(3)));
    // Klik do pole seznam nezavre.
    let mut f = F::new();
    let field = f.ui.a11y().into_iter().find(|n| n.role == Role::TextInput).unwrap();
    f.click(field.bounds.x + 4, field.bounds.y + 4);
    assert!(f.ui.suggest_open());
    // Klik mimo zavre a jde dal.
    let mut f = F::new();
    let list = f.ui.overlay_rect().unwrap();
    assert!(!list.contains(480, 390));
    f.click(480, 390);
    assert!(f.msgs.contains(&M::Closed));
    assert!(!f.ui.suggest_open());
}

#[test]
fn suggest_a11y_list_and_activate() {
    let mut f = F::new();
    let nodes = f.ui.a11y();
    let field = nodes.iter().find(|n| n.role == Role::TextInput).unwrap();
    let list = nodes.iter().find(|n| n.role == Role::ListBox).unwrap();
    assert_eq!(list.name, "Cesty");
    assert_eq!(list.parent, field.id);
    let o = f.options();
    assert_eq!(o.len(), 4);
    assert!(o.iter().all(|n| n.parent == list.id));
    assert_eq!(o[0].value, "složka");
    assert!(o[2].state & state::DISABLED != 0 && o[2].name.contains("bez práv"));
    assert_eq!(o[2].actions, 0);
    let id = o[1].id;
    let r = f.ui.a11y_action(id, actions::ACTIVATE, "");
    assert!(r.msgs.contains(&M::Pick(1)));
}

#[test]
fn suggest_never_on_password_field() {
    let mut f = F::new();
    f.secure = true;
    f.rebuild();
    assert!(!f.ui.suggest_open());
    assert!(f.ui.a11y().iter().all(|n| n.role != Role::ListBox && n.state & state::AUTOCOMPLETE == 0));
}

#[test]
fn completion_standalone_above_anchor_keys_and_hit() {
    let th = theme::Theme::builtin(theme::Mode::Light);
    let items: Vec<Suggestion> = (0..30).map(|i| Suggestion::new(&alloc::format!("p{i}"), "soubor")).collect();
    let mut c = Completion::new("Cesty", items);
    c.anchor = Rect::new(20, 560, 600, 24);
    c.prefer = Side::Above;
    let r = c.place(&th, 0, Rect::new(0, 0, 800, 600));
    assert!(r.bottom() <= 560 && r.h <= th.size.completion_max_h && r.w == th.size.completion_w);
    // Pismena patri poli.
    assert_eq!(c.key(&InputEvent::Key { code: KEY_A, action: 1, ch: Some('a') }, 0), Outcome::Ignored);
    assert_eq!(c.key(&InputEvent::Key { code: KEY_LEFT, action: 1, ch: None }, 0), Outcome::Ignored);
    assert_eq!(c.key(&InputEvent::Key { code: KEY_HOME, action: 1, ch: None }, inputproto::mods::CTRL), Outcome::Ignored);
    assert_eq!(c.key(&InputEvent::Key { code: KEY_PAGEDOWN, action: 1, ch: None }, 0), Outcome::Moved);
    let s = c.selected.unwrap();
    assert!(s > 1, "o stranku");
    // Vybrana polozka je videt.
    let row = c.row_rect(s);
    assert!(row.y >= r.y && row.bottom() <= r.bottom());
    assert_eq!(c.key(&InputEvent::Key { code: KEY_ENTER, action: 1, ch: None }, 0), Outcome::Accept(s));
    assert_eq!(c.key(&InputEvent::Key { code: KEY_ESC, action: 1, ch: None }, 0), Outcome::Close);
    // Mys: najeti, klik, kolecko, stisk mimo.
    assert_eq!(c.hit(row.x + 4, row.y + 2), Some(s));
    assert_eq!(c.pointer(&InputEvent::PointerMove { x: row.x + 4, y: row.y + 2 }), Outcome::Moved);
    assert_eq!(c.pointer(&InputEvent::Scroll { dx: 0, dy: -1 }), Outcome::Moved);
    assert_eq!(c.pointer(&InputEvent::Button { code: BTN_LEFT, pressed: true, x: 5, y: 5 }), Outcome::Close);
    // Kresleni do platna vrstvy popoveru (pocatek = obdelnik seznamu).
    let mut px = vec![0u32; (r.w * r.h) as usize];
    let mut canvas = gfx::Canvas::new(&mut px, r.w, r.h, r.w).unwrap();
    let mut fonts = crate::Look::ui_fonts();
    let mut icons = gfx::icon::Icons::new();
    let mut p = Paint { theme: &th, fonts: &mut fonts, mono: None, icons: &mut icons };
    c.draw(&mut canvas, (r.x, r.y), &mut p);
    assert_eq!(px[(r.w * (r.h / 2) + r.w / 2) as usize] & 0xFF_FFFF, th.color.surface & 0xFF_FFFF);
    // Uzly pristupnosti s id od volajiciho.
    let n = c.a11y(7, 100, |k| 200 + k as u32, 3);
    assert_eq!(n.len(), 31);
    assert_eq!((n[0].role, n[0].parent, n[1].id, n[1].parent, n[1].layer), (Role::ListBox, 7, 200, 100, 3));
}

#[test]
fn completion_flips_side_and_limits_data() {
    let th = theme::Theme::builtin(theme::Mode::Light);
    let mut c = Completion::new("", vec![Suggestion::new("a", ""), Suggestion::new("b", "")]);
    // Pod kotvou neni misto -> nad ni.
    c.anchor = Rect::new(0, 380, 100, 20);
    let r = c.place(&th, 0, Rect::new(0, 0, 300, 400));
    assert!(r.bottom() <= 380);
    assert_eq!(r.w, 300, "sirka nejvys plocha");
    // Dotyk: vyssi radky.
    c.place(&th, 44, Rect::new(0, 0, 300, 400));
    assert_eq!(c.row_rect(1).y - c.row_rect(0).y, 44);
    // Limity dat zvenku.
    let long: String = core::iter::repeat_n('x', 1000).collect();
    let many: Vec<Suggestion> = (0..MAX_ITEMS + 50).map(|_| Suggestion { name: long.clone(), ..Default::default() }).collect();
    let c = Completion::new(&long, many);
    assert_eq!(c.items.len(), MAX_ITEMS);
    assert!(c.items[0].name.chars().count() <= crate::complete::MAX_TEXT);
    assert!(c.header.chars().count() <= crate::complete::MAX_TEXT);
    // Prazdny seznam: nic nepadne.
    let mut e = Completion::new("x", Vec::new());
    assert_eq!(e.key(&InputEvent::Key { code: KEY_DOWN, action: 1, ch: None }, 0), Outcome::Ignored);
    assert_eq!(e.key(&InputEvent::Key { code: KEY_ENTER, action: 1, ch: None }, 0), Outcome::Ignored);
    assert_eq!(e.hit(0, 0), None);
}

#[test]
fn completion_wheel_scroll_survives_place() {
    let th = theme::Theme::builtin(theme::Mode::Light);
    let items: Vec<Suggestion> = (0..30).map(|i| Suggestion::new(&alloc::format!("p{i}"), "")).collect();
    let mut c = Completion::new("", items);
    c.anchor = Rect::new(0, 0, 100, 20);
    let area = Rect::new(0, 0, 800, 600);
    let r = c.place(&th, 0, area);
    c.pointer(&InputEvent::PointerMove { x: r.x + 10, y: r.y + 30 });
    c.pointer(&InputEvent::Scroll { dx: 0, dy: -2 });
    let s = c.scroll;
    assert!(s > 0);
    // Dalsi udalost prepocita polohu: posun zustane.
    c.place(&th, 0, area);
    assert_eq!(c.scroll, s);
}
