//! Nativni knihovna UI (milnik 6c): jedna knihovna pro vsechny aplikace.
//!
//! Aplikace popise rozhrani deklarativne ([`node`]); knihovna ho rozvrhne
//! ([`layout`]), vykresli do bufferu ([`draw`]), prevede vstup a akce
//! pristupnosti na zpravy aplikace ([`event`]) a odvodi strom vrstev
//! a strom pristupnosti - aplikace nic z toho nepise rucne.
//!
//! Jadro (`Ui`) nezavisi na OS a testuje se na hostiteli
//! (`cargo test -p ui`). Behove prostredi pro aplikace v OS (kanal ke
//! kompozitoru, buffery ve VMO, smycka) je v [`client`] (vlastnost `os`).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod draw;
pub mod event;
pub mod layout;
pub mod look;
pub mod node;

#[cfg(feature = "os")]
pub mod client;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::text::Family;
use guiproto::Rect;
use guiproto::a11y::{self, Node as A11yNode, Role};
use guiproto::action::{Arg, Value};

pub use node::*;

use layout::{ElemKind, Element};
pub use look::Look;

/// Id vrstev, ktere knihovna pouziva: koren okna a pro posouvany obsah
/// s indexem prvku `i` skupina `SCROLL_BASE + 2i` a obsah `+ 1`.
pub const ROOT_LAYER: u32 = 1;
pub const SCROLL_BASE: u32 = 1000;

/// Id uzlu pristupnosti: okno 1, prvek s indexem `i` je `i + 2`.
pub const WINDOW_NODE: u32 = 1;

pub fn node_id(index: usize) -> u32 {
    index as u32 + 2
}

pub fn scroll_layers(index: usize) -> (u32, u32) {
    let g = SCROLL_BASE + 2 * index as u32;
    (g, g + 1)
}

/// Aplikace nad knihovnou.
pub trait App {
    type Msg: Clone;
    fn title(&self) -> String;
    fn view(&self) -> Node<Self::Msg>;
    fn update(&mut self, msg: Self::Msg);
    /// Akce z manifestu (volana shellem, asistentem, agentem). Argumenty
    /// uz overil kompozitor proti deklaraci.
    fn action(&mut self, id: &str, args: &[Arg]) -> Result<Option<Value>, String> {
        let _ = (id, args);
        Err(String::from("akce neni implementovana"))
    }

    /// Zprava z dalsiho kanalu aplikace (napr. sluzba uloh), viz
    /// `client::run_with`. Vraci, zda se zmenil stav (prekreslit).
    fn external(&mut self, data: &[u8]) -> bool {
        let _ = data;
        false
    }
}

/// Stav rozhrani jednoho okna.
pub struct Ui<M> {
    /// Vzhled: motiv z tokenu a pisma.
    pub look: Look,
    /// Ikony design systemu (zaskrtnuti, ...).
    pub(crate) icons: gfx::icon::Icons,
    pub w: u32,
    pub h: u32,
    pub(crate) elems: Vec<Element<M>>,
    pub(crate) focus: Option<usize>,
    pub(crate) hover: Option<usize>,
    pub(crate) pressed: Option<usize>,
    /// Kurzor v textovem poli (index znaku) podle indexu prvku.
    pub(crate) cursor: BTreeMap<usize, usize>,
    /// Posun obsahu posouvanych oblasti podle indexu prvku.
    pub(crate) scroll: BTreeMap<usize, i32>,
    /// Ukazatel (souradnice okna).
    pub(crate) pointer: (i32, i32),
    pub(crate) mods: u16,
    pub title: String,
    /// Okno ma fokus systemu: jen tehdy se kresli ramecek fokusu prvku.
    pub(crate) active: bool,
    /// Misto stisknuteho prvku (prostor, obdelnik) pro parovani s pustenim.
    pub(crate) pressed_rect: Option<(usize, Rect)>,
}

/// Posouvana oblast: index prvku, obdelnik v okne, vyska obsahu, posun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollArea {
    pub index: usize,
    pub rect: Rect,
    pub content_h: u32,
    pub offset: i32,
}

impl<M: Clone> Ui<M> {
    pub fn new(fonts: Family, w: u32, h: u32) -> Self {
        Self {
            look: Look::new(fonts, theme::Mode::Light),
            icons: gfx::icon::Icons::new(),
            w,
            h,
            elems: Vec::new(),
            focus: None,
            hover: None,
            pressed: None,
            cursor: BTreeMap::new(),
            scroll: BTreeMap::new(),
            pointer: (-1, -1),
            mods: 0,
            title: String::new(),
            active: true,
            pressed_rect: None,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
    }

    /// Znovu rozvrhne rozhrani podle `view` (po kazde zmene stavu aplikace).
    pub fn rebuild(&mut self, title: String, view: Node<M>) {
        let p = self.look.pad();
        let area = Rect::new(p as i32, p as i32, self.w.saturating_sub(2 * p), self.h.saturating_sub(2 * p));
        self.elems = layout::layout(view, area, &self.look);
        self.title = title;
        let n = self.elems.len();
        let ok = |i: &usize| *i < n;
        if !self.focus.as_ref().is_some_and(ok) || !self.focus.is_some_and(|i| self.elems[i].focusable()) {
            self.focus = self.elems.iter().position(|e| e.focusable());
        }
        self.hover = self.hover.filter(ok);
        self.pressed = self.pressed.filter(ok);
        // Kurzor nejvys na konec hodnoty.
        let lens: Vec<(usize, usize)> = self
            .elems
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match &e.kind {
                ElemKind::TextInput { value, .. } => Some((i, value.chars().count())),
                _ => None,
            })
            .collect();
        self.cursor.retain(|i, _| lens.iter().any(|(j, _)| j == i));
        for (i, l) in lens {
            let c = self.cursor.entry(i).or_insert(l);
            *c = (*c).min(l);
        }
        // Posun nejvys do konce obsahu.
        let areas: Vec<(usize, i32)> = self.scroll_areas().iter().map(|a| (a.index, (a.content_h as i32 - a.rect.h as i32).max(0))).collect();
        self.scroll.retain(|i, _| areas.iter().any(|(j, _)| j == i));
        for (i, max) in areas {
            let s = self.scroll.entry(i).or_insert(0);
            *s = (*s).clamp(0, max);
        }
    }

    /// Posouvane oblasti (kazda je vlastni vrstva v kompozitoru).
    pub fn scroll_areas(&self) -> Vec<ScrollArea> {
        self.elems
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e.kind {
                ElemKind::Scroll { content_h } if e.space == 0 => Some(ScrollArea {
                    index: i,
                    rect: e.rect,
                    content_h: content_h.max(e.rect.h),
                    offset: self.scroll.get(&i).copied().unwrap_or(0),
                }),
                _ => None,
            })
            .collect()
    }

    /// Strom pristupnosti okna.
    pub fn a11y(&self) -> Vec<A11yNode> {
        let mut out = alloc::vec![A11yNode::new(WINDOW_NODE, 0, Role::Window, &self.title)];
        for (i, e) in self.elems.iter().enumerate() {
            let parent = if e.parent == 0 { WINDOW_NODE } else { node_id(e.parent - 1) };
            let (role, name, value, mut state, actions) = match &e.kind {
                ElemKind::Label { text, .. } => (Role::Label, if e.agent { alloc::format!("agent: {text}") } else { text.clone() }, String::new(), 0, 0),
                ElemKind::Button { text, .. } => (Role::Button, text.clone(), String::new(), 0, a11y::actions::ACTIVATE | a11y::actions::FOCUS),
                ElemKind::TextInput { value, placeholder, .. } => {
                    (Role::TextInput, placeholder.clone(), value.clone(), 0, a11y::actions::SET_VALUE | a11y::actions::FOCUS)
                }
                ElemKind::Checkbox { text, checked, .. } => (
                    Role::Checkbox,
                    text.clone(),
                    String::new(),
                    if *checked { a11y::state::CHECKED } else { 0 },
                    a11y::actions::TOGGLE | a11y::actions::ACTIVATE,
                ),
                ElemKind::List { .. } => (Role::List, String::new(), String::new(), 0, a11y::actions::FOCUS),
                ElemKind::ListItem { text, selected, agent, .. } => (
                    Role::ListItem,
                    // Agent i pro ctecku obrazovky (ne jen barvou).
                    if *agent { alloc::format!("agent: {text}") } else { text.clone() },
                    String::new(),
                    if *selected { a11y::state::SELECTED } else { 0 },
                    a11y::actions::ACTIVATE,
                ),
                ElemKind::Progress { value } => (Role::Progress, String::new(), alloc::format!("{value}"), 0, 0),
                ElemKind::Group => (Role::Group, String::new(), String::new(), 0, 0),
                ElemKind::Scroll { .. } => (Role::Group, String::new(), String::new(), 0, a11y::actions::SCROLL),
            };
            if self.focus == Some(i) {
                state |= a11y::state::FOCUSED;
            }
            if e.disabled {
                state |= a11y::state::DISABLED;
            }
            let layer = if e.space == 0 { ROOT_LAYER } else { scroll_layers(e.space - 1).1 };
            out.push(A11yNode {
                id: node_id(i),
                parent,
                role,
                name,
                value,
                state,
                layer,
                bounds: e.rect,
                actions,
            });
        }
        out
    }

    /// Prvek s fokusem (index).
    pub fn focused(&self) -> Option<usize> {
        self.focus
    }
}
