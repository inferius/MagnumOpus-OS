//! Rozbalovaci vyber, kontextova nabidka, bublina.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::SHIFT;

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Lang(usize),
    Copy,
    Paste,
    Save,
    Row(Vec<u64>),
    Ctx(u64),
}

struct F {
    lang: usize,
    msgs: Vec<M>,
    actions: Vec<String>,
    ui: Ui<M>,
}

const LANGS: [&str; 4] = ["Čeština", "Deutsch", "English", "Slovenčina"];

impl F {
    fn new() -> Self {
        let mut f = F { lang: 0, msgs: Vec::new(), actions: Vec::new(), ui: Ui::new(crate::Look::ui_fonts(), 500, 420) };
        f.rebuild();
        f
    }

    fn rebuild(&mut self) {
        let ctx = vec![
            menu_item("Kopírovat", M::Copy).shortcut("Ctrl+C"),
            menu_item("Vložit", M::Paste).disabled("Schránka je prázdná"),
            menu_separator(),
            menu_item("Uložit", M::Save),
        ];
        let app_menu = vec![guiproto::menu::MenuItem {
            label: "Soubor".into(),
            children: vec![guiproto::menu::MenuItem { label: "Nový".into(), action: "file.new".into(), shortcut: "Ctrl+N".into(), ..Default::default() }],
            ..Default::default()
        }];
        let v = column([
            select(LANGS.iter().map(|s| String::from(*s)), self.lang).on_pick(M::Lang).key("lang"),
            button("Text").on_click(M::Save).context_menu(ctx).key("b"),
            button("").on_click(M::Save).tooltip("Nastavení").context_menu(menu_from(&app_menu)).key("ikona"),
            table(vec![col("Soubor", 0)], 5, |i| i as u64, |i| Cells::new([alloc::format!("f{i}")])).on_selection(M::Row).on_context(M::Ctx).context_menu(vec![menu_item("Kopírovat", M::Copy)]).height(150).key("t"),
        ]);
        self.ui.rebuild("p".into(), v);
    }

    fn ev(&mut self, e: InputEvent, mods: u16, t: Option<u64>) {
        let r = self.ui.event_at(e, mods, t);
        for m in r.msgs {
            if let M::Lang(k) = m {
                self.lang = k;
            }
            self.msgs.push(m);
        }
        self.actions.extend(r.actions.into_iter().map(|a| a.0));
        self.rebuild();
    }

    fn node(&self, role: Role, name: &str) -> guiproto::a11y::Node {
        self.ui.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap_or_else(|| panic!("{role:?} {name}"))
    }

    fn has(&self, role: Role) -> bool {
        self.ui.a11y().iter().any(|n| n.role == role)
    }

    fn click_at(&mut self, x: i32, y: i32) {
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0, None);
        self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, 0, None);
    }

    fn key(&mut self, code: u16, mods: u16) {
        self.ev(InputEvent::Key { code, action: 1, ch: None }, mods, None);
        self.ev(InputEvent::Key { code, action: 0, ch: None }, mods, None);
    }
}

/// Vyber: klik otevre seznam (stav `expanded`), klik na polozku vybere,
/// Esc zavre bez zmeny, klavesnice, zavreny vyber sipkami.
#[test]
fn select_mouse_and_keyboard() {
    let mut f = F::new();
    let cb = f.ui.a11y().into_iter().find(|n| n.role == Role::ComboBox).unwrap();
    assert_eq!(cb.value, "Čeština");
    f.click_at(cb.bounds.x + 10, cb.bounds.y + 10);
    assert!(f.has(Role::Menu));
    assert!(f.ui.a11y().into_iter().find(|n| n.role == Role::ComboBox).unwrap().state & state::EXPANDED != 0);
    // Polozka pod polem.
    let item = f.node(Role::MenuItem, "English");
    assert!(item.bounds.y > cb.bounds.y);
    f.click_at(item.bounds.x + 10, item.bounds.y + 5);
    assert_eq!(f.lang, 2);
    assert!(!f.has(Role::Menu));
    // Klavesnice: Enter otevre (zvyrazneno vybrane), dolu, Enter.
    f.key(KEY_ENTER, 0);
    assert!(f.node(Role::MenuItem, "English").state & state::FOCUSED != 0);
    f.key(KEY_DOWN, 0);
    f.key(KEY_ENTER, 0);
    assert_eq!(f.lang, 3);
    // Esc zavre bez zmeny.
    f.key(KEY_SPACE, 0);
    f.key(KEY_UP, 0);
    f.key(KEY_ESC, 0);
    assert!(!f.has(Role::Menu));
    assert_eq!(f.lang, 3);
    // Zavreny vyber: sipka nahoru primo.
    f.key(KEY_UP, 0);
    assert_eq!(f.lang, 2);
    // Stisk-tah-pusteni.
    let cb = f.ui.a11y().into_iter().find(|n| n.role == Role::ComboBox).unwrap();
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: cb.bounds.x + 10, y: cb.bounds.y + 10 }, 0, None);
    let it = f.node(Role::MenuItem, "Deutsch");
    f.ev(InputEvent::PointerMove { x: it.bounds.x + 10, y: it.bounds.y + 5 }, 0, None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: it.bounds.x + 10, y: it.bounds.y + 5 }, 0, None);
    assert_eq!(f.lang, 1);
}

/// Kontextova nabidka: prave tlacitko, zakazana polozka s duvodem,
/// oddelovac se preskakuje, klik mimo jen zavre.
#[test]
fn context_menu() {
    let mut f = F::new();
    let b = f.node(Role::Button, "Text").bounds;
    f.ev(InputEvent::Button { code: BTN_RIGHT, pressed: true, x: b.x + 5, y: b.y + 5 }, 0, None);
    assert!(f.has(Role::Menu));
    let paste = f.node(Role::MenuItem, "Vložit (Schránka je prázdná)");
    assert!(paste.state & state::DISABLED != 0);
    assert_eq!(f.node(Role::MenuItem, "Kopírovat").value, "Ctrl+C");
    // Klik na zakazanou nic, nabidka zustane.
    f.click_at(paste.bounds.x + 10, paste.bounds.y + 5);
    assert!(f.msgs.is_empty());
    // Sipky: Kopirovat -> (preskoci Vlozit a oddelovac) Ulozit.
    f.key(KEY_DOWN, 0);
    f.key(KEY_DOWN, 0);
    assert!(f.node(Role::MenuItem, "Uložit").state & state::FOCUSED != 0);
    f.key(KEY_ENTER, 0);
    assert_eq!(f.msgs, vec![M::Save]);
    // Klik mimo zavre a tlacitko pod nim neaktivuje.
    f.ev(InputEvent::Button { code: BTN_RIGHT, pressed: true, x: b.x + 5, y: b.y + 5 }, 0, None);
    f.click_at(b.x + 5, b.y + 5);
    assert!(!f.has(Role::Menu));
    assert_eq!(f.msgs, vec![M::Save]);
    // Shift+F10 na prvku s fokusem; akce pres pristupnost.
    let id = f.node(Role::Button, "Text").id;
    f.ui.a11y_action(id, actions::FOCUS, "");
    f.key(KEY_F10, SHIFT);
    let copy = f.node(Role::MenuItem, "Kopírovat").id;
    let r = f.ui.a11y_action(copy, actions::ACTIVATE, "");
    assert_eq!(r.msgs, vec![M::Copy]);
}

/// Nabidka z menu aplikace (akce z manifestu) a jmeno tlacitka z bubliny.
#[test]
fn menu_from_app_and_icon_button() {
    let mut f = F::new();
    let b = f.node(Role::Button, "Nastavení").bounds;
    f.ev(InputEvent::Button { code: BTN_RIGHT, pressed: true, x: b.x + 5, y: b.y + 5 }, 0, None);
    let n = f.node(Role::MenuItem, "Nový");
    assert_eq!(n.value, "Ctrl+N");
    f.click_at(n.bounds.x + 10, n.bounds.y + 5);
    assert_eq!(f.actions, ["file.new"]);
}

/// Bublina po 500 ms nad prvkem, klik ji skryje.
#[test]
fn tooltip_after_delay() {
    let mut f = F::new();
    let b = f.node(Role::Button, "Nastavení").bounds;
    let t0 = 1_000_000_000;
    f.ev(InputEvent::PointerMove { x: b.x + 5, y: b.y + 5 }, 0, Some(t0));
    assert_eq!(f.ui.next_deadline(), Some(t0 + popup::TIP_DELAY));
    assert!(!f.ui.timer(t0 + popup::TIP_DELAY / 2));
    assert!(f.ui.timer(t0 + popup::TIP_DELAY));
    assert_eq!(f.node(Role::Tooltip, "Nastavení").parent, f.node(Role::Button, "Nastavení").id);
    assert_eq!(f.ui.next_deadline(), None);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: b.x + 5, y: b.y + 5 }, 0, Some(t0 + popup::TIP_DELAY + 1));
    assert!(!f.has(Role::Tooltip));
    // Prvek bez bubliny nic necasuje.
    let t = f.node(Role::Button, "Text").bounds;
    f.ev(InputEvent::PointerMove { x: t.x + 5, y: t.y + 5 }, 0, Some(t0 * 3));
    assert_eq!(f.ui.next_deadline(), None);
}

/// Prave tlacitko na radku tabulky: vyber radku, `on_context` a nabidka.
#[test]
fn table_context_menu() {
    let mut f = F::new();
    let row = f.ui.a11y().into_iter().find(|n| n.role == Role::Row && n.name == "f2").unwrap().bounds;
    f.ev(InputEvent::Button { code: BTN_RIGHT, pressed: true, x: row.x + 10, y: row.y + 5 }, 0, None);
    assert_eq!(f.msgs, vec![M::Row(vec![2]), M::Ctx(2)]);
    assert!(f.has(Role::Menu));
    f.key(KEY_DOWN, 0);
    f.key(KEY_ENTER, 0);
    assert_eq!(f.msgs.last(), Some(&M::Copy));
}
