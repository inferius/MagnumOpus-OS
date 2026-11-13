//! Prepinac, volba, posuvnik hodnoty, segmenty, zalozky, neurcity prubeh.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods::{CTRL, SHIFT};

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Wifi(bool),
    Mode(u8),
    Vol(i32),
    Filter(usize),
    Tab(usize),
    Close(usize),
}

struct F {
    wifi: bool,
    mode: u8,
    vol: i32,
    filter: usize,
    tab: usize,
    tabs: Vec<String>,
    busy: bool,
    ui: Ui<M>,
}

impl F {
    fn new() -> Self {
        let mut f = F { wifi: false, mode: 0, vol: 40, filter: 0, tab: 0, tabs: vec!["Relace 1".into(), "Relace 2".into(), "Relace 3".into()], busy: false, ui: Ui::new(crate::Look::ui_fonts(), 500, 400) };
        f.rebuild();
        f
    }

    fn rebuild(&mut self) {
        let mut ch = vec![
            tabs(self.tabs.clone(), self.tab).on_pick(M::Tab).on_close(M::Close).key("tabs"),
            switch("Wi-Fi", self.wifi).on_toggle(M::Wifi).key("wifi"),
            row([radio("Světlý", self.mode == 0).on_click(M::Mode(0)).key("r0"), radio("Tmavý", self.mode == 1).on_click(M::Mode(1)).key("r1")]),
            slider(self.vol, 0, 100).step(5).on_value(M::Vol).key("vol"),
            segmented(["Vše".into(), "Běží".into(), "Agent".into()], self.filter).on_pick(M::Filter).key("seg"),
        ];
        if self.busy {
            ch.push(busy());
        }
        let v = column(ch);
        self.ui.rebuild("c".into(), v);
    }

    fn ev(&mut self, e: InputEvent, mods: u16) -> Vec<M> {
        let r = self.ui.event(e, mods);
        for m in &r.msgs {
            match *m {
                M::Wifi(b) => self.wifi = b,
                M::Mode(k) => self.mode = k,
                M::Vol(v) => self.vol = v,
                M::Filter(k) => self.filter = k,
                M::Tab(k) => self.tab = k,
                M::Close(k) => {
                    self.tabs.remove(k);
                }
            }
        }
        self.rebuild();
        r.msgs
    }

    fn node(&self, role: Role, name: &str) -> guiproto::a11y::Node {
        self.ui.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap()
    }

    fn click(&mut self, x: i32, y: i32) -> Vec<M> {
        let mut m = self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y }, 0);
        m.extend(self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }, 0));
        m
    }

    fn click_node(&mut self, role: Role, name: &str) -> Vec<M> {
        let b = self.node(role, name).bounds;
        self.click(b.x + 6, b.y + b.h as i32 / 2)
    }

    fn key(&mut self, code: u16, mods: u16) -> Vec<M> {
        let mut m = self.ev(InputEvent::Key { code, action: 1, ch: None }, mods);
        m.extend(self.ev(InputEvent::Key { code, action: 0, ch: None }, mods));
        m
    }

    fn focus(&mut self, role: Role, name: &str) {
        let id = self.node(role, name).id;
        let r = self.ui.a11y_action(id, actions::FOCUS, "");
        assert!(r.msgs.is_empty());
    }
}

#[test]
fn switch_and_radio() {
    let mut f = F::new();
    assert_eq!(f.click_node(Role::Switch, "Wi-Fi"), vec![M::Wifi(true)]);
    assert!(f.node(Role::Switch, "Wi-Fi").state & state::CHECKED != 0);
    // Mezernik az pri pusteni.
    f.focus(Role::Switch, "Wi-Fi");
    let down = f.ev(InputEvent::Key { code: KEY_SPACE, action: 1, ch: None }, 0);
    assert!(down.is_empty());
    assert_eq!(f.ev(InputEvent::Key { code: KEY_SPACE, action: 0, ch: None }, 0), vec![M::Wifi(false)]);
    assert_eq!(f.click_node(Role::Radio, "Tmavý"), vec![M::Mode(1)]);
    assert!(f.node(Role::Radio, "Tmavý").state & state::CHECKED != 0);
    // Uz vybrana volba znovu nic.
    assert!(f.click_node(Role::Radio, "Tmavý").is_empty());
}

/// Posuvnik: klik na drahu nastavi hodnotu (na krok), tah, klavesy,
/// kolecko, akce pristupnosti `set_value`.
#[test]
fn slider_value() {
    let mut f = F::new();
    let s = f.ui.a11y().into_iter().find(|n| n.role == Role::Slider).unwrap();
    assert_eq!(s.value, "40 (0-100)");
    let (x0, x1, y) = (s.bounds.x, s.bounds.right() as i32, s.bounds.y + s.bounds.h as i32 / 2);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x: x1 - 1, y }, 0);
    assert_eq!(f.vol, 100);
    f.ev(InputEvent::PointerMove { x: x0 + (x1 - x0) / 2, y }, 0);
    assert_eq!(f.vol, 50);
    f.ev(InputEvent::PointerMove { x: x0 - 40, y }, 0);
    assert_eq!(f.vol, 0);
    f.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x: x0 - 40, y }, 0);
    f.ev(InputEvent::PointerMove { x: x0 + 300, y: y + 300 }, 0);
    assert_eq!(f.vol, 0, "po pusteni tah nic");
    f.key(KEY_RIGHT, 0);
    assert_eq!(f.vol, 5);
    f.key(KEY_PAGEUP, 0);
    assert_eq!(f.vol, 55);
    f.key(KEY_END, 0);
    assert_eq!(f.vol, 100);
    f.ev(InputEvent::PointerMove { x: x0 + 10, y }, 0);
    f.ev(InputEvent::Scroll { dx: 0, dy: -2 }, 0);
    assert_eq!(f.vol, 90);
    let id = f.ui.a11y().into_iter().find(|n| n.role == Role::Slider).unwrap().id;
    let r = f.ui.a11y_action(id, actions::SET_VALUE, "33");
    assert_eq!(r.msgs, vec![M::Vol(33)]);
}

/// Segmenty: klik, sipky; podrizene uzly `radio` s akci.
#[test]
fn segmented_pick() {
    let mut f = F::new();
    assert_eq!(f.click_node(Role::Radio, "Agent"), vec![M::Filter(2)]);
    f.key(KEY_LEFT, 0);
    assert_eq!(f.filter, 1);
    f.key(KEY_HOME, 0);
    assert_eq!(f.filter, 0);
    let seg = f.ui.a11y().into_iter().find(|n| n.role == Role::Segmented).unwrap();
    assert_eq!(seg.value, "Vše");
    let id = f.node(Role::Radio, "Běží").id;
    let r = f.ui.a11y_action(id, actions::ACTIVATE, "");
    assert_eq!(r.msgs, vec![M::Filter(1)]);
}

/// Zalozky: klik vybere, krizek a prostredni tlacitko zavrou, Ctrl+Tab
/// odkudkoli prepina, sipky na zalozkach.
#[test]
fn tabs_select_close_and_ctrl_tab() {
    let mut f = F::new();
    assert_eq!(f.click_node(Role::Tab, "Relace 2"), vec![M::Tab(1)]);
    assert!(f.node(Role::Tab, "Relace 2").state & state::SELECTED != 0);
    // Ctrl+Tab s fokusem jinde (prepinac).
    f.focus(Role::Switch, "Wi-Fi");
    f.key(KEY_TAB, CTRL);
    assert_eq!(f.tab, 2);
    f.key(KEY_TAB, CTRL);
    assert_eq!(f.tab, 0);
    f.key(KEY_TAB, CTRL | SHIFT);
    assert_eq!(f.tab, 2);
    // Krizek na zalozce 1.
    let b = f.node(Role::Tab, "Relace 2").bounds;
    let is = f.ui.look.theme.size.icon_xs as i32;
    let sp3 = f.ui.look.theme.space[3] as i32;
    let (cx, cy) = (b.right() as i32 - sp3 - is / 2, b.y + b.h as i32 / 2);
    assert_eq!(f.click(cx, cy), vec![M::Close(1)]);
    assert_eq!(f.tabs, ["Relace 1", "Relace 3"]);
    // Prostredni tlacitko.
    let b = f.node(Role::Tab, "Relace 1").bounds;
    let m = f.ev(InputEvent::Button { code: BTN_MIDDLE, pressed: true, x: b.x + 5, y: b.y + 5 }, 0);
    assert_eq!(m, vec![M::Close(0)]);
}

/// Neurcity prubeh: animace po snimcich, jen kdyz je videt.
#[test]
fn busy_animates_only_when_present() {
    let mut f = F::new();
    assert!(!f.ui.wants_frame());
    f.busy = true;
    f.rebuild();
    assert!(f.ui.wants_frame());
    let p0 = f.ui.busy_phase;
    assert!(f.ui.tick());
    assert_ne!(f.ui.busy_phase, p0);
    let b = f.ui.a11y().into_iter().find(|n| n.role == Role::Progress).unwrap();
    assert!(b.state & state::BUSY != 0);
}

/// Pruhovy graf podilu (oddily disku): sirky podle vah bez mezer, cely
/// popisek ve stromu pristupnosti, v pruhu jen kdyz se vejde, srafy volneho
/// mista, legenda pod pruhem.
#[test]
fn segments_bar() {
    let items = vec![
        Segment::new(100, "EFI 100 MB", Tone::Muted),
        Segment::new(480_000, "Windows C: 480 GB", Tone::Other),
        Segment::new(300_000, "MagnumOpus 300 GB", Tone::Accent),
        Segment::new(220_000, "Volné 220 GB", Tone::Free),
    ];
    let mut u: Ui<()> = Ui::new(crate::Look::ui_fonts(), 640, 200);
    u.rebuild("g".into(), column([segments(items).legend(true)]));
    let nodes = u.a11y();
    let parts: Vec<_> = nodes.iter().filter(|n| n.role == Role::ListItem).collect();
    assert_eq!(parts.len(), 4);
    assert_eq!(parts[1].name, "Windows C: 480 GB");
    // Bez mezer a podle vah (EFI ma temer nulovou sirku).
    for k in 1..4 {
        assert_eq!(parts[k].bounds.x, parts[k - 1].bounds.x + parts[k - 1].bounds.w as i32);
    }
    assert!(parts[0].bounds.w <= 1);
    assert!(parts[1].bounds.w > parts[2].bounds.w && parts[2].bounds.w > parts[3].bounds.w);
    let bar = nodes.iter().find(|n| n.role == Role::List).unwrap().bounds;
    assert_eq!(parts[1].bounds.h, u.look.control_h() + u.look.theme.space[2]);
    assert!(bar.h > parts[1].bounds.h, "misto pro legendu");
    // Kresleni: usek Accent ma barvu `accent`, volne misto srafy (obe barvy).
    let (w, h) = (640u32, 200u32);
    let mut px = vec![0u32; (w * h) as usize];
    let mut c = gfx::Canvas::new(&mut px, w, h, w).unwrap();
    u.draw(&mut c, 0);
    let col = u.look.theme.color.clone();
    let a = parts[2].bounds;
    assert_eq!(c.get((a.x + 3) as u32, (a.y + 3) as u32), col.accent);
    let fr = parts[3].bounds;
    let mut seen = (false, false);
    for x in fr.x + 2..fr.x + 30 {
        let p = c.get(x as u32, (fr.y + 4) as u32);
        seen.0 |= p == col.surface;
        seen.1 |= p == col.surface_3;
    }
    assert!(seen.0 && seen.1, "srafy");
}

/// Obrazek: zmenseny se zachovanim pomeru stran na stred prvku, popis pro
/// ctecku; kratka data se nekresli (zadna panika).
#[test]
fn image_scaled_and_described() {
    let (w, h) = (40u32, 20u32);
    let px: alloc::rc::Rc<[u32]> = alloc::vec![0xFF20_50C4u32; (w * h) as usize].into();
    let mut u: Ui<()> = Ui::new(crate::Look::ui_fonts(), 300, 200);
    u.rebuild("i".into(), column([image(w, h, px.clone(), "Náhled obrázku").width(80).height(80), image(w, h, px.slice_short(), "vadný")]));
    let n = u.a11y().into_iter().find(|n| n.role == Role::Image && n.name == "Náhled obrázku").unwrap();
    let mut buf = vec![0u32; 300 * 200];
    let mut c = gfx::Canvas::new(&mut buf, 300, 200, 300).unwrap();
    u.draw(&mut c, 0);
    let b = n.bounds;
    // Na stred prvku, nezvetsuje se nad svou velikost (40x20).
    let (cx, cy) = (b.x + b.w as i32 / 2, b.y + b.h as i32 / 2);
    assert_eq!(c.get(cx as u32, cy as u32), 0xFF20_50C4);
    assert_ne!(c.get(cx as u32, (cy - 11) as u32), 0xFF20_50C4);
    assert_ne!(c.get((cx - 21) as u32, cy as u32), 0xFF20_50C4);
    // Mensi prostor zmensi se zachovanim pomeru: 40x20 v 20 px siroke oblasti.
    let mut u2: Ui<()> = Ui::new(crate::Look::ui_fonts(), 300, 200);
    u2.rebuild("i".into(), row([image(w, h, px, "malý").width(20), spacer()]));
    let b2 = u2.a11y().into_iter().find(|n| n.role == Role::Image).unwrap().bounds;
    let mut buf2 = vec![0u32; 300 * 200];
    let mut c2 = gfx::Canvas::new(&mut buf2, 300, 200, 300).unwrap();
    u2.draw(&mut c2, 0);
    let (cx, cy) = (b2.x + 10, b2.y + b2.h as i32 / 2);
    assert_eq!(c2.get(cx as u32, cy as u32), 0xFF20_50C4);
    assert_ne!(c2.get(cx as u32, (cy - 6) as u32), 0xFF20_50C4, "vyska 10 px");
}

trait Short {
    fn slice_short(&self) -> alloc::rc::Rc<[u32]>;
}

impl Short for alloc::rc::Rc<[u32]> {
    fn slice_short(&self) -> alloc::rc::Rc<[u32]> {
        self[..10].into()
    }
}
