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
mod field;
pub mod layout;
pub mod look;
pub mod node;
pub mod text;

#[cfg(feature = "os")]
pub mod client;

#[cfg(test)]
mod tests;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use gfx::text::Family;
use guiproto::Rect;
use guiproto::a11y::{self, Node as A11yNode, Role};
use guiproto::action::{Arg, Value};

pub use node::*;

use layout::{ElemKind, Element};
pub use look::Look;

/// Id uzlu s textem chyby prvku s id `id` (mimo rozsah id prvku).
pub fn error_node(id: u32) -> u32 {
    id | 0x8000_0000
}

/// Id vrstev, ktere knihovna pouziva: koren okna a pro posouvanou oblast
/// se stabilnim id `n` (stejne jako id jejiho uzlu pristupnosti) skupina
/// `SCROLL_BASE + 2n` a obsah `+ 1`.
pub const ROOT_LAYER: u32 = 1;
pub const SCROLL_BASE: u32 = 1000;

/// Id uzlu pristupnosti okna; prvky maji stabilni id od 2 (podle klice).
pub const WINDOW_NODE: u32 = 1;

/// Nejvyssi id prvku (vrstvy obsahu se vejdou do u32, uzly chyb maji
/// nejvyssi bit).
const ID_MAX: u32 = (u32::MAX - SCROLL_BASE) / 2 - 1;

pub fn scroll_layers(id: u32) -> (u32, u32) {
    let g = SCROLL_BASE + 2 * id;
    (g, g + 1)
}

/// Jednotny stav prvku: pocita ho jadro, komponenta podle nej jen kresli
/// (a strom pristupnosti ho hlasi).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct State {
    /// Ukazatel nad prvkem (ne u nedostupneho).
    pub hover: bool,
    /// Stisknuto: tlacitko mysi s ukazatelem nad prvkem (odjeti stav
    /// zrusi, navrat obnovi) nebo drzeny Enter / mezernik.
    pub pressed: bool,
    /// Prvek ma fokus okna.
    pub focused: bool,
    /// Ramecek fokusu je videt (fokus a okno je aktivni).
    pub focus_visible: bool,
    pub disabled: bool,
    /// Neplatna hodnota (`Node::error`).
    pub error: bool,
    pub selected: bool,
    pub checked: bool,
}

impl State {
    /// Bity stavu pro strom pristupnosti.
    pub fn a11y_bits(self) -> u32 {
        use a11y::state::*;
        let b = |c: bool, v: u32| if c { v } else { 0 };
        b(self.focused, FOCUSED)
            | b(self.checked, CHECKED)
            | b(self.disabled, DISABLED)
            | b(self.selected, SELECTED)
            | b(self.hover, HOVER)
            | b(self.pressed, PRESSED)
            | b(self.error, INVALID)
    }
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

    /// Menu aplikace (nabidky s akcemi z manifestu, `guiproto::menu`):
    /// kresli ho system (paleta prikazu Ctrl+K, menu v titulku). Posila se
    /// pri zmene (zaskrtnuti, zakazano s duvodem).
    fn menu(&self) -> Vec<guiproto::menu::MenuItem> {
        Vec::new()
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
    /// Index prvku podle klice (po poslednim rozvrzeni).
    pub(crate) by_key: BTreeMap<Key, usize>,
    /// Stabilni id prvku podle klice (uzel pristupnosti, vrstvy oblasti).
    ids: BTreeMap<Key, u32>,
    used_ids: BTreeSet<u32>,
    next_id: u32,
    /// Id prvku podle indexu (po poslednim rozvrzeni).
    pub(crate) elem_ids: Vec<u32>,
    /// Stav podle indexu prvku plati jen pro aktualni rozvrzeni;
    /// `rebuild` ho prevede pres klice.
    pub(crate) focus: Option<usize>,
    pub(crate) hover: Option<usize>,
    pub(crate) pressed: Option<usize>,
    /// Text, kurzor, vyber a historie textovych poli podle klice prvku.
    pub(crate) edits: BTreeMap<Key, text::TextEdit>,
    /// Posun obsahu textovych poli (x jednoradkove, y viceradkove).
    pub(crate) tscroll: BTreeMap<Key, (i32, i32)>,
    /// Posledni klik do pole (klic, cas, x, y, poradi) pro dvojklik.
    pub(crate) last_click: Option<(Key, u64, i32, i32, u8)>,
    /// Pole, ve kterem tah mysi vybira text.
    pub(crate) drag_field: Option<Key>,
    /// Pole, ktere pozadalo o vlozeni ze schranky (`Ui::paste`).
    pub(crate) paste_target: Option<Key>,
    /// Posun obsahu posouvanych oblasti podle klice prvku.
    pub(crate) scroll: BTreeMap<Key, i32>,
    /// Ukazatel (souradnice okna).
    pub(crate) pointer: (i32, i32),
    pub(crate) mods: u16,
    pub title: String,
    /// Okno ma fokus systemu: jen tehdy se kresli ramecek fokusu prvku.
    pub(crate) active: bool,
    /// Prvek s drzenou klavesou Enter / mezernik (akce az pri pusteni).
    pub(crate) key_pressed: Option<usize>,
}

/// Posouvana oblast: index prvku (prostor `index + 1` pro `Ui::draw`),
/// stabilni id (vrstvy, `scroll_layers`), obdelnik v okne, vyska obsahu,
/// posun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollArea {
    pub index: usize,
    pub id: u32,
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
            by_key: BTreeMap::new(),
            ids: BTreeMap::new(),
            used_ids: BTreeSet::new(),
            next_id: 2,
            elem_ids: Vec::new(),
            focus: None,
            hover: None,
            pressed: None,
            edits: BTreeMap::new(),
            tscroll: BTreeMap::new(),
            last_click: None,
            drag_field: None,
            paste_target: None,
            scroll: BTreeMap::new(),
            pointer: (-1, -1),
            mods: 0,
            title: String::new(),
            active: true,
            key_pressed: None,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
    }

    /// Znovu rozvrhne rozhrani podle `view` (po kazde zmene stavu aplikace).
    /// Stav prvku (fokus, stisk, kurzor, posun) se prenese podle klicu.
    pub fn rebuild(&mut self, title: String, view: Node<M>) {
        let key_of = |o: Option<usize>| o.and_then(|i| self.elems.get(i)).map(|e| e.key);
        let (fk, pk, kk) = (key_of(self.focus), key_of(self.pressed), key_of(self.key_pressed));
        let p = self.look.pad();
        let area = Rect::new(p as i32, p as i32, self.w.saturating_sub(2 * p), self.h.saturating_sub(2 * p));
        self.elems = layout::layout(view, area, &self.look);
        self.title = title;
        self.by_key = self.elems.iter().enumerate().map(|(i, e)| (e.key, i)).collect();
        self.assign_ids();
        let find = |k: Option<Key>| k.and_then(|k| self.by_key.get(&k).copied());
        let focus = find(fk).filter(|&i| self.elems[i].focusable());
        self.focus = focus.or_else(|| self.elems.iter().position(|e| e.focusable()));
        self.pressed = find(pk);
        self.key_pressed = find(kk);
        // Textova pole prevezmou hodnotu aplikace (kurzor a historie zustanou).
        self.sync_fields();
        // Posun nejvys do konce obsahu.
        let areas: Vec<(Key, i32)> = self.scroll_areas().iter().map(|a| (self.elems[a.index].key, (a.content_h as i32 - a.rect.h as i32).max(0))).collect();
        self.scroll.retain(|k, _| areas.iter().any(|(j, _)| j == k));
        for (k, max) in areas {
            let s = self.scroll.entry(k).or_insert(0);
            *s = (*s).clamp(0, max);
        }
        // Najeti podle ukazatele (prvky se mohly posunout).
        self.hover = if self.pointer.0 >= 0 { self.hit(self.pointer.0, self.pointer.1) } else { None };
    }

    /// Stabilni id prvku podle klicu: prvek, ktery zustal, si id necha;
    /// novy dostane dalsi volne. Uvolnene id se znovu pouzije az po obehu
    /// citace, aby akce pristupnosti poslana na stary uzel netrefila jiny
    /// prvek.
    fn assign_ids(&mut self) {
        let (by_key, used) = (&self.by_key, &mut self.used_ids);
        self.ids.retain(|k, id| {
            let keep = by_key.contains_key(k);
            if !keep {
                used.remove(id);
            }
            keep
        });
        self.elem_ids.clear();
        for i in 0..self.elems.len() {
            let k = self.elems[i].key;
            let id = match self.ids.get(&k) {
                Some(&id) => id,
                None => {
                    let mut id = self.next_id;
                    while self.used_ids.contains(&id) {
                        id = if id >= ID_MAX { 2 } else { id + 1 };
                    }
                    self.next_id = if id >= ID_MAX { 2 } else { id + 1 };
                    self.used_ids.insert(id);
                    self.ids.insert(k, id);
                    id
                }
            };
            self.elem_ids.push(id);
        }
    }

    /// Id uzlu pristupnosti prvku `i`.
    pub fn node_id(&self, i: usize) -> u32 {
        self.elem_ids[i]
    }

    /// Index prvku s klicem `k` (po poslednim rozvrzeni).
    pub fn index_of(&self, k: Key) -> Option<usize> {
        self.by_key.get(&k).copied()
    }

    /// Jednotny stav prvku `i`.
    pub fn state(&self, i: usize) -> State {
        let e = &self.elems[i];
        let dis = e.disabled;
        let focused = self.focus == Some(i);
        let (selected, checked) = match &e.kind {
            ElemKind::ListItem { selected, .. } => (*selected, false),
            ElemKind::Checkbox { checked, .. } => (false, *checked),
            _ => (false, false),
        };
        State {
            hover: self.hover == Some(i) && !dis,
            pressed: !dis && ((self.pressed == Some(i) && self.hover == Some(i)) || self.key_pressed == Some(i)),
            focused,
            focus_visible: focused && self.active && !dis,
            disabled: dis,
            error: e.error.is_some(),
            selected,
            checked,
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
                    id: self.elem_ids[i],
                    rect: e.rect,
                    content_h: content_h.max(e.rect.h),
                    offset: self.scroll.get(&e.key).copied().unwrap_or(0),
                }),
                _ => None,
            })
            .collect()
    }

    /// Strom pristupnosti okna. Id uzlu jsou stabilni (podle klicu).
    pub fn a11y(&self) -> Vec<A11yNode> {
        use a11y::actions::*;
        let mut out = alloc::vec![A11yNode::new(WINDOW_NODE, 0, Role::Window, &self.title)];
        for (i, e) in self.elems.iter().enumerate() {
            let parent = if e.parent == 0 { WINDOW_NODE } else { self.elem_ids[e.parent - 1] };
            let none = String::new;
            let (role, name, value, actions) = match &e.kind {
                ElemKind::Label { text, .. } => (Role::Label, if e.agent { alloc::format!("agent: {text}") } else { text.clone() }, none(), 0),
                ElemKind::Button { text, .. } => (Role::Button, text.clone(), none(), ACTIVATE | FOCUS),
                // Heslo: hodnota se nikdy neposila (ani jeji delka).
                ElemKind::TextInput { placeholder, secure: true, .. } => (Role::PasswordInput, placeholder.clone(), none(), SET_VALUE | FOCUS),
                ElemKind::TextInput { value, placeholder, .. } => (Role::TextInput, placeholder.clone(), value.clone(), SET_VALUE | FOCUS),
                ElemKind::Checkbox { text, .. } => (Role::Checkbox, text.clone(), none(), TOGGLE | ACTIVATE),
                ElemKind::List { .. } => (Role::List, none(), none(), FOCUS),
                // Agent i pro ctecku obrazovky (ne jen barvou).
                ElemKind::ListItem { text, agent, .. } => (Role::ListItem, if *agent { alloc::format!("agent: {text}") } else { text.clone() }, none(), ACTIVATE),
                ElemKind::Progress { value } => (Role::Progress, none(), alloc::format!("{value}"), 0),
                ElemKind::Group => (Role::Group, none(), none(), 0),
                ElemKind::Scroll { .. } => (Role::ScrollArea, none(), none(), SCROLL),
            };
            let layer = if e.space == 0 { ROOT_LAYER } else { scroll_layers(self.elem_ids[e.space - 1]).1 };
            let id = self.elem_ids[i];
            out.push(A11yNode { id, parent, role, name, value, state: self.state(i).a11y_bits(), layer, bounds: e.rect, actions });
            // Text chyby jako podrizeny uzel pole (ctecka ho precte s polem).
            if let Some(err) = &e.error {
                out.push(A11yNode { id: error_node(id), parent: id, role: Role::Label, name: err.clone(), value: none(), state: 0, layer, bounds: e.rect, actions: 0 });
            }
        }
        out
    }

    /// Prvek s fokusem (index).
    pub fn focused(&self) -> Option<usize> {
        self.focus
    }
}
