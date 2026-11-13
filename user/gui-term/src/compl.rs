//! Nabidka doplnovani v okne: sdilena komponenta seznamu navrhu
//! (`ui::complete::Completion`, vzhled `#compl` z `design/terminal.html`)
//! ve vrstve popoveru nad radkem vstupu. Polozky a vyber drzi `term::ui`
//! (`App::menu`), tady je jen jejich zobrazeni, vstup a pristupnost.
//! Radek vstupu si fokus necha: psani seznam zuzi, sipky hybou vyberem,
//! Enter / Tab doplni, Esc a klik mimo zavrou.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::Rect;
use guiproto::a11y::Node;
use guiproto::window::{InputEvent, InputRecord};
use term::ui::App;
use ui::complete::{Completion, Outcome, Suggestion};

/// Id uzlu pristupnosti seznamu (polozka k = +1+k, nejvic
/// `ui::complete::MAX_ITEMS` polozek).
pub const LIST_NODE: u32 = 5000;

/// Otevreny seznam (`None` v `Term` = nabidka zavrena).
pub struct Compl {
    pub comp: Completion,
    /// Polozky od `App` pri poslednim `sync` (jine = novy seznam).
    shown: Vec<(String, u8)>,
}

impl Compl {
    fn new() -> Self {
        Self { comp: Completion::default(), shown: Vec::new() }
    }

    /// Prevezme nabidku z `app` (zavrena -> `None`). `anchor` = bunka
    /// zacatku doplnovaneho slova, `area` = kde smi seznam lezet
    /// (pracovni plocha v souradnicich okna).
    pub fn sync(this: &mut Option<Compl>, app: &App, anchor: Rect, area: Rect, theme: &theme::Theme, min_hit: u32) {
        let Some(m) = &app.menu else {
            *this = None;
            return;
        };
        let c = this.get_or_insert_with(Compl::new);
        let now: Vec<(String, u8)> = m.items.iter().map(|(t, k, _)| (t.clone(), *k)).collect();
        if now != c.shown {
            let items = m
                .items
                .iter()
                .map(|(text, kind, help)| {
                    let (icon, what) = term::ui::menu::kind_info(*kind);
                    Suggestion::new(text, if help.is_empty() { what } else { help }).icon(icon)
                })
                .collect();
            c.comp.set_items(term::ui::menu::header(m.items.iter().map(|i| i.1)), items);
            c.shown = now;
        }
        if c.comp.selected != Some(m.sel) && m.sel < c.comp.items.len() {
            c.comp.selected = Some(m.sel);
        }
        c.comp.anchor = anchor;
        c.comp.prefer = ui::complete::Side::Above;
        c.comp.place(theme, min_hit, area);
    }

    /// Vysledek seznamu prenese do `app`. `true` = zmena (prekreslit).
    pub fn apply(&self, out: Outcome, app: &mut App) -> bool {
        match out {
            Outcome::Ignored => false,
            Outcome::Moved => {
                if let Some(k) = self.comp.selected {
                    app.menu_select(k);
                }
                true
            }
            Outcome::Accept(k) => {
                app.menu_accept(k);
                true
            }
            Outcome::Close => {
                app.menu_close();
                true
            }
        }
    }

    /// Klavesa pro otevreny seznam: `Some(redraw)` = spotrebovana.
    pub fn key(&mut self, r: &InputRecord, app: &mut App) -> Option<bool> {
        let out = self.comp.key(&r.event, r.mods);
        (out != Outcome::Ignored).then(|| self.apply(out, app))
    }

    /// Udalost mysi: `Some(redraw)` = spotrebovana (nad seznamem), jinak
    /// jde dal (`redraw` zmeny najeti nebo zavreni stiskem mimo se
    /// projevi pri dalsim kresleni).
    pub fn pointer(&mut self, ev: &InputEvent, app: &mut App) -> (bool, bool) {
        let inside = match *ev {
            InputEvent::PointerMove { x, y } | InputEvent::Button { x, y, .. } => self.comp.rect().contains(x, y),
            _ => false,
        };
        let out = self.comp.pointer(ev);
        // Kolecko nad seznamem posouva seznam (ne vystup).
        let consumed = inside || (matches!(ev, InputEvent::Scroll { .. }) && out == Outcome::Moved);
        (consumed, self.apply(out, app))
    }

    /// Uzly seznamu pod uzlem radku vstupu `parent`.
    pub fn a11y(&self, parent: u32) -> Vec<Node> {
        self.comp.a11y(parent, LIST_NODE, |k| LIST_NODE + 1 + k as u32, ui::ROOT_LAYER)
    }

    /// Akce pristupnosti na uzlu `node` (polozka seznamu); `None` = uzel
    /// seznamu neni.
    pub fn a11y_action(&mut self, node: u32, action: u32, app: &mut App) -> Option<bool> {
        let k = node.checked_sub(LIST_NODE + 1)? as usize;
        if k >= self.comp.items.len() {
            return None;
        }
        let out = self.comp.a11y_action(k, action);
        Some(self.apply(out, app))
    }
}
