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

pub mod context;
mod controls;
pub mod draw;
pub mod event;
mod field;
pub mod layout;
pub mod look;
pub mod node;
pub(crate) mod popup;
pub mod scrollbar;
mod table;
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
pub use context::{Context, SizeClass};

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

/// Nejvyssi id prvku: nad nim jsou virtualni uzly tabulek (bit 30) a uzly
/// chyb (bit 31); vrstvy obsahu `SCROLL_BASE + 2n` se vejdou do u32.
const ID_MAX: u32 = 0x3FFF_FFFF;

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
    /// Rozhrani pro kontext okna `cx` (velikost, trida sirky: adaptivni
    /// rozvrzeni podle sirky okna, ne obrazovky).
    fn view(&self, cx: &Context) -> Node<Self::Msg>;
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

    /// Zprava z kanalu `index` (`client::run_channels`, poradi jako ve
    /// vektoru kanalu). Vychozi: kanal 0 = [`App::external`].
    fn external_from(&mut self, index: usize, data: &[u8]) -> bool {
        if index == 0 { self.external(data) } else { false }
    }

    /// Pozadovana velikost okna pri otevreni: (sirka, vyska, strop sirky
    /// a vysky v promile pracovni plochy), viz `Window::request_size`.
    /// `None` = velikost z `client::run`.
    fn size_hint(&self) -> Option<(u32, u32, u16, u16)> {
        None
    }

    /// Klavesa, kterou nespotreboval prvek s fokusem (ani textove pole):
    /// zkratky aplikace (F2, Delete, Enter mimo tabulku, Ctrl+C/X/V nad
    /// tabulkou, Ctrl+N, Alt+sipky). `ch` = znak podle rozlozeni,
    /// `mods` = `inputproto::mods`.
    fn key(&self, code: u16, ch: Option<char>, mods: u16) -> Option<Self::Msg> {
        let _ = (code, ch, mods);
        None
    }

    /// Ucinky po `update` (schranka): klient je vybere po kazde davce zprav.
    fn effects(&mut self) -> Vec<Effect> {
        Vec::new()
    }

    /// Data vlozena ze schranky na zadost `Effect::Paste` (format, ktery
    /// prisel). Vraci, zda se zmenil stav.
    fn pasted(&mut self, format: &str, data: &[u8]) -> bool {
        let _ = (format, data);
        false
    }
}

/// Ucinek aplikace, ktery provede klient (Elm: `update` jen meni stav).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Effect {
    /// Kopirovat: data ve vic formatech (napr. `text/uri-list` a text).
    /// Plati jen hned po fyzickem vstupu (Ctrl+C, polozka menu).
    Copy { formats: Vec<(String, Vec<u8>)> },
    /// Vlozit: prvni z formatu, ktery schranka ma; odpoved `App::pasted`.
    Paste { formats: Vec<String> },
}

/// Stav rozhrani jednoho okna.
pub struct Ui<M> {
    /// Vzhled: motiv z tokenu a pisma.
    pub look: Look,
    /// Ikony design systemu (zaskrtnuti, ...).
    pub(crate) icons: gfx::icon::Icons,
    pub w: u32,
    pub h: u32,
    /// Kontext okna (velikost, trida sirky, obrazovka).
    pub(crate) cx: Context,
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
    /// Stav tabulek (posun, kurzor, kotva, sirky sloupcu) podle klice.
    pub(crate) tables: BTreeMap<Key, table::TableState>,
    /// Rozpracovany stisk v tabulce (radek, zahlavi, posuvnik).
    pub(crate) table_press: Option<table::Press>,
    /// Tazene radky (od `on_drag` do pusteni).
    pub(crate) dragging: Option<table::Dragging>,
    /// Faze animace neurciteho prubehu (px, roste po snimcich).
    pub(crate) busy_phase: u32,
    /// Otevrena nabidka (vyber, kontextova).
    pub(crate) popup: Option<popup::Popup<M>>,
    /// Bublina: prvek pod ukazatelem a od kdy (odpocet), zobrazena.
    pub(crate) tip: Option<(Key, u64)>,
    pub(crate) tip_shown: Option<Key>,
    /// Cas posledni udalosti (ns).
    pub(crate) now: u64,
    /// Nabidka a bublina ve vlastni vrstve popoveru (klient v OS).
    pub(crate) overlay: bool,
    /// Posun bubliny pri kresleni do vrstvy popoveru (jeji pocatek).
    pub(crate) tip_shift: (i32, i32),
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
        let look = Look::new(fonts, theme::Mode::Light);
        let cx = Context::new(w, h, &look.theme.breakpoint);
        Self {
            look,
            cx,
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
            tables: BTreeMap::new(),
            table_press: None,
            dragging: None,
            busy_phase: 0,
            popup: None,
            tip: None,
            tip_shown: None,
            now: 0,
            overlay: false,
            tip_shift: (0, 0),
            scroll: BTreeMap::new(),
            pointer: (-1, -1),
            mods: 0,
            title: String::new(),
            active: true,
            key_pressed: None,
        }
    }

    /// Nova velikost okna. Vraci, zda se zmenila trida sirky (aplikace
    /// muze zmenit rozvrzeni).
    pub fn resize(&mut self, w: u32, h: u32) -> bool {
        self.w = w;
        self.h = h;
        let bp = self.look.theme.breakpoint;
        self.cx.resize(w, h, &bp)
    }

    /// Kontext okna (pro `App::view`).
    pub fn context(&self) -> &Context {
        &self.cx
    }

    /// Kontext od kompozitoru (`WINDOW_CONTEXT`).
    pub fn set_context(&mut self, output: u32, screen: Rect, work_area: Rect, scale: u32, refresh_mhz: u32, generation: u32) {
        let c = &mut self.cx;
        (c.output, c.screen, c.work_area, c.scale, c.refresh_mhz, c.generation) = (output, screen, work_area, scale, refresh_mhz, generation);
    }

    /// Rozsireni kontextu (drzeni, ukazatele, klavesnice, dlazdice...).
    pub fn set_context_ext(&mut self, e: &context::ContextExt) {
        let c = &mut self.cx;
        c.posture = e.posture;
        c.distance = e.distance;
        c.insets = e.insets;
        c.safe = e.safe;
        c.pointers = e.pointers.clone();
        c.keyboard = e.keyboard;
        c.obscured = e.obscured.clone();
        c.tile = e.tile;
        c.min_hit = e.min_hit as u32;
        self.look.min_hit = c.min_hit;
        c.text_scale = e.text_scale as u32;
    }

    /// Poloha obsahu okna na obrazovce (`WINDOW_PLACE`).
    pub fn set_window_rect(&mut self, r: Rect) {
        self.cx.window = r;
    }

    /// Klient kresli nabidku a bublinu do vlastni vrstvy popoveru (muze
    /// presahovat okno); bez toho se kresli do okna (testy).
    pub fn use_overlay_layer(&mut self, on: bool) {
        self.overlay = on;
    }

    /// Formaty ve schrance (klient po zprave `Changed`).
    pub fn set_clipboard_formats(&mut self, f: Vec<String>) {
        self.cx.clipboard = f;
    }

    /// Znovu rozvrhne rozhrani aplikace pro aktualni kontext okna.
    pub fn refresh<A: App<Msg = M>>(&mut self, app: &A) {
        let v = app.view(&self.cx);
        self.rebuild(app.title(), v);
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
        self.sync_tables();
        // Posun nejvys do konce obsahu.
        let areas: Vec<(Key, i32)> = self.scroll_areas().iter().map(|a| (self.elems[a.index].key, (a.content_h as i32 - a.rect.h as i32).max(0))).collect();
        self.scroll.retain(|k, _| areas.iter().any(|(j, _)| j == k));
        for (k, max) in areas {
            let s = self.scroll.entry(k).or_insert(0);
            *s = (*s).clamp(0, max);
        }
        // Nabidka a bublina zmizelych prvku pryc.
        if self.popup.as_ref().is_some_and(|p| self.index_of(p.owner()).is_none()) {
            self.popup = None;
        }
        if self.tip.is_some_and(|(k, _)| self.index_of(k).is_none()) {
            self.tip = None;
            self.tip_shown = None;
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
            let mut extra = 0;
            let (role, name, value, actions) = if let Some((ro, n, v, s, a)) = self.control_role(i) {
                extra = s;
                (ro, n, v, a)
            } else {
                match &e.kind {
                    ElemKind::Label { text, .. } => (Role::Label, if e.agent { alloc::format!("agent: {text}") } else { text.clone() }, none(), 0),
                    // Tlacitko jen s ikonou: jmeno z bubliny.
                    ElemKind::Button { text, .. } => (Role::Button, if text.is_empty() { e.tooltip.clone().unwrap_or_default() } else { text.clone() }, none(), ACTIVATE | FOCUS),
                    ElemKind::Select { options, selected, .. } => {
                        if self.popup.as_ref().is_some_and(|p| p.owner() == e.key) {
                            extra = a11y::state::EXPANDED;
                        }
                        (Role::ComboBox, none(), options.get(*selected).cloned().unwrap_or_default(), ACTIVATE | FOCUS)
                    }
                    // Heslo: hodnota se nikdy neposila (ani jeji delka).
                    ElemKind::TextInput { placeholder, secure: true, .. } => (Role::PasswordInput, placeholder.clone(), none(), SET_VALUE | FOCUS),
                    ElemKind::TextInput { value, placeholder, .. } => (Role::TextInput, placeholder.clone(), value.clone(), SET_VALUE | FOCUS),
                    ElemKind::Checkbox { text, .. } => (Role::Checkbox, text.clone(), none(), TOGGLE | ACTIVATE),
                    ElemKind::List { .. } => (Role::List, none(), none(), FOCUS),
                    // Agent i pro ctecku obrazovky (ne jen barvou).
                    ElemKind::ListItem { text, agent, .. } => (Role::ListItem, if *agent { alloc::format!("agent: {text}") } else { text.clone() }, none(), ACTIVATE),
                    ElemKind::Progress { value } => (Role::Progress, none(), alloc::format!("{value}"), 0),
                    ElemKind::Table(t) => (Role::Table, t.columns.first().map_or_else(none, |c| c.title.clone()), alloc::format!("{} řádků", t.count), FOCUS | SCROLL),
                    ElemKind::Image { alt, .. } => (Role::Image, alt.clone(), none(), 0),
                    ElemKind::Group => (Role::Group, none(), none(), 0),
                    ElemKind::Scroll { .. } => (Role::ScrollArea, none(), none(), SCROLL),
                    _ => (Role::Group, none(), none(), 0),
                }
            };
            let layer = if e.space == 0 { ROOT_LAYER } else { scroll_layers(self.elem_ids[e.space - 1]).1 };
            let id = self.elem_ids[i];
            out.push(A11yNode { id, parent, role, name, value, state: self.state(i).a11y_bits() | extra, layer, bounds: e.rect, actions });
            self.control_children(i, id, layer, &mut out);
            // Tabulka: zahlavi a viditelne radky.
            if matches!(e.kind, ElemKind::Table(_)) {
                self.table_a11y(i, id, layer, &mut out);
            }
            // Text chyby jako podrizeny uzel pole (ctecka ho precte s polem).
            if let Some(err) = &e.error {
                out.push(A11yNode { id: error_node(id), parent: id, role: Role::Label, name: err.clone(), value: none(), state: 0, layer, bounds: e.rect, actions: 0 });
            }
        }
        self.popup_a11y(&mut out);
        out
    }

    /// Prvek s fokusem (index).
    pub fn focused(&self) -> Option<usize> {
        self.focus
    }
}
