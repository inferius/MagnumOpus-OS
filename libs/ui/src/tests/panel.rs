//! Komponenta panel (design/bubliny-panely.html): rozmery z tokenu,
//! volby radio s ikonou a popisem (sipky vyberou), prepinac v radku,
//! stavovy radek, pata, pristupnost.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use guiproto::a11y::{Role, actions, state};
use guiproto::window::InputEvent;
use inputproto::code::*;

use crate::*;

#[derive(Clone, Debug, PartialEq)]
enum M {
    Look(u8),
    Contrast(bool),
    Settings,
}

struct F {
    look: u8,
    contrast: bool,
    ui: Ui<M>,
}

impl F {
    fn new() -> Self {
        let t = theme::Theme::builtin(theme::Mode::Light);
        let mut f = F { look: 2, contrast: false, ui: Ui::new(crate::Look::ui_fonts(), t.size.panel_w, 600) };
        f.ui.window_pad = false;
        f.rebuild();
        f
    }

    fn rebuild(&mut self) {
        let th = self.ui.look.theme.clone();
        let s = PanelSize::of(&th);
        let opts = [("sun", "Světlý", ""), ("moon", "Tmavý", ""), ("clock", "Automaticky", "podle východu a západu slunce")];
        let mut items = vec![panel_header(&s, "moon", "Vzhled", "teď světlý")];
        let radios: Vec<Node<M>> = opts.iter().enumerate().map(|(k, (ic, t, d))| radio(t, self.look == k as u8).on_click(M::Look(k as u8)).panel_row(Some(ic), d)).collect();
        items.push(column(radios).gap(0));
        items.push(panel_state(&s, "info", "Teď světlý, podle východu a západu slunce.", false));
        items.push(switch("Vysoký kontrast", self.contrast).on_toggle(M::Contrast).panel_row(Some("contrast"), "z Přístupnosti"));
        items.push(panel_footer(&s, vec![button("Nastavení vzhledu").icon("palette").on_click(M::Settings).disabled(true).tooltip("Nastavení zatím nejsou").grow()]));
        // Pohled bez okraje okna (panel ma vlastni odsazeni).
        let v = panel(items);
        self.ui.rebuild("Vzhled".into(), v);
    }

    fn ev(&mut self, e: InputEvent) -> Vec<M> {
        let r = self.ui.event(e, 0);
        for m in &r.msgs {
            match *m {
                M::Look(k) => self.look = k,
                M::Contrast(b) => self.contrast = b,
                M::Settings => {}
            }
        }
        self.rebuild();
        r.msgs
    }

    fn node(&self, role: Role, name: &str) -> guiproto::a11y::Node {
        self.ui.a11y().into_iter().find(|n| n.role == role && n.name == name).unwrap_or_else(|| panic!("{role:?} {name}"))
    }

    fn click(&mut self, role: Role, name: &str) -> Vec<M> {
        let b = self.node(role, name).bounds;
        let (x, y) = (b.x + b.w as i32 / 2, b.y + b.h as i32 / 2);
        let mut m = self.ev(InputEvent::Button { code: BTN_LEFT, pressed: true, x, y });
        m.extend(self.ev(InputEvent::Button { code: BTN_LEFT, pressed: false, x, y }));
        m
    }
}

#[test]
fn panel_rows_have_token_sizes_icons_and_descriptions() {
    let f = F::new();
    let t = &f.ui.look.theme;
    let auto = f.node(Role::Radio, "Automaticky");
    assert_eq!(auto.state & state::CHECKED, state::CHECKED);
    assert_eq!(auto.value, "podle východu a západu slunce", "popis radku je hodnota pro ctecku");
    // Radek: nejmene `panel_row`, pres celou sirku obsahu.
    for name in ["Světlý", "Tmavý", "Automaticky"] {
        let n = f.node(Role::Radio, name);
        assert!(n.bounds.h >= t.size.panel_row, "{name}: vyska {}", n.bounds.h);
    }
    let light = f.node(Role::Radio, "Světlý");
    assert_eq!(light.bounds.h, t.size.panel_row, "radek bez popisu ma presne panel_row");
    let sw = f.node(Role::Switch, "Vysoký kontrast");
    assert_eq!(sw.value, "z Přístupnosti");
    // Volby jsou pod sebou bez mezer (jedna skupina).
    let dark = f.node(Role::Radio, "Tmavý");
    assert_eq!(dark.bounds.y, light.bounds.y + light.bounds.h as i32);
    // Hlavicka: nazev a souhrn; stav; pata s vypnutym tlacitkem a duvodem.
    f.node(Role::Label, "Vzhled");
    f.node(Role::Label, "teď světlý");
    f.node(Role::Label, "Teď světlý, podle východu a západu slunce.");
    let b = f.node(Role::Button, "Nastavení vzhledu");
    assert_eq!(b.state & state::DISABLED, state::DISABLED);
}

#[test]
fn panel_radio_group_arrows_select_and_click_picks() {
    let mut f = F::new();
    // Tab: zastavka je zvolena volba.
    let id = f.node(Role::Radio, "Automaticky").id;
    f.ui.a11y_action(id, actions::FOCUS, "");
    let m = f.ev(InputEvent::Key { code: KEY_UP, action: 1, ch: None });
    assert_eq!(m, vec![M::Look(1)], "sipka nahoru vybere Tmavy");
    assert_eq!(f.node(Role::Radio, "Tmavý").state & state::CHECKED, state::CHECKED);
    let m = f.click(Role::Radio, "Světlý");
    assert_eq!(m, vec![M::Look(0)]);
    let m = f.click(Role::Switch, "Vysoký kontrast");
    assert_eq!(m, vec![M::Contrast(true)]);
    assert_eq!(f.node(Role::Switch, "Vysoký kontrast").state & state::CHECKED, state::CHECKED);
}

#[test]
fn panel_draws_hover_and_transparent_background() {
    let mut f = F::new();
    let t = f.ui.look.theme.clone();
    let (w, h) = (f.ui.w, f.ui.h);
    let n = f.node(Role::Radio, "Tmavý");
    let mut px = vec![0u32; (w * h) as usize];
    let probe = |px: &[u32], x: i32, y: i32| px[(y as u32 * w + x as u32) as usize];
    {
        let mut c = gfx::Canvas::new(&mut px, w, h, w).unwrap();
        f.ui.draw(&mut c, 0);
    }
    // Vlevo od ikony je pozadi okna.
    assert_eq!(probe(&px, n.bounds.x + 2, n.bounds.y + 2), t.color.surface);
    // Najeti: cely radek `surface_3`.
    f.ev(InputEvent::PointerMove { x: n.bounds.x + 30, y: n.bounds.y + 10 });
    {
        let mut c = gfx::Canvas::new(&mut px, w, h, w).unwrap();
        f.ui.draw(&mut c, 0);
    }
    assert_eq!(probe(&px, n.bounds.x + 2, n.bounds.y + 2), t.color.surface_3);
    // Pruhledne pozadi: volajici dal vlastni (material), ui ho neprepise.
    f.ui.transparent = true;
    f.ev(InputEvent::PointerLeave);
    let mut px = vec![0xFF12_3456u32; (w * h) as usize];
    {
        let mut c = gfx::Canvas::new(&mut px, w, h, w).unwrap();
        f.ui.draw(&mut c, 0);
    }
    assert_eq!(probe(&px, n.bounds.x + 2, n.bounds.y + 2), 0xFF12_3456);
    let _ = String::new();
}
