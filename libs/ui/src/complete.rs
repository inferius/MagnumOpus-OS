//! Seznam navrhu u textoveho pole (combobox, doplnovani): pole si drzi
//! fokus klavesnice a dal se do nej pise, sipky jen hybou vyberem
//! v seznamu. Na rozdil od nabidky (`popup`) si seznam klavesnici nebere.
//!
//! Dva zpusoby pouziti:
//! - **ve strome `Ui`:** `text_input(..).suggest(suggest(..))`, knihovna
//!   resi klavesy, mys, kresleni do vrstvy popoveru a uzly pristupnosti
//!   (`ui::suggest`);
//! - **samostatne** (napr. gui-term s vlastni mrizkou): [`Completion`]
//!   drzi stav, [`Completion::key`] a [`Completion::pointer`] prevadi vstup
//!   na [`Outcome`], [`Completion::place`] spocita obdelnik u kotvy,
//!   [`Completion::draw`] nakresli obsah do dodaneho platna
//!   a [`Completion::a11y`] vrati uzly pristupnosti.
//!
//! Vzhled podle `#compl` v `design/terminal.html`: karta popoveru se
//! zahlavim (`caption`, `ink_muted`, vaha `body_strong`), radek = mrizka
//! ikona (`icon_m`) | jmeno (mono, vaha `body_strong`) | typ (`caption`,
//! `ink_muted`), vyska `control_h`, zaobleni `xs`, vyber `accent_soft`,
//! sirka `size.completion_w`, nejvetsi vyska `size.completion_max_h`.
//!
//! Klavesy (jen kdyz je seznam otevreny): nahoru / dolu (dokola), PgUp /
//! PgDn, Home / End, Enter / Tab prijme vybranou polozku, Esc zavre.
//! Ostatni klavesy a klavesy s Ctrl / Alt / Shift+Tab vrati
//! [`Outcome::Ignored`] a patri poli.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::icon::Icons;
use gfx::text::Family;
use gfx::{Canvas, Rect};
use guiproto::a11y::{self, Node as A11yNode, Role};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods;
use theme::{TextStyle, Theme};

/// Nejvic polozek (vic se orizne; data muzou prijit zvenku).
pub const MAX_ITEMS: usize = 1000;

/// Nejdelsi text polozky a zahlavi (znaky).
pub const MAX_TEXT: usize = 200;

/// Odstup seznamu od kotvy (px).
const ANCHOR_GAP: i32 = 2;

/// Polozka seznamu navrhu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Suggestion {
    /// Jmeno ikony z `gfx::icon` (prazdne = bez ikony).
    pub icon: String,
    /// Jmeno (doplni se do pole).
    pub name: String,
    /// Typ nebo popis vpravo ("slozka", "prikaz").
    pub detail: String,
    /// Zakazana polozka: duvod (nejde vybrat, ctecka ho precte).
    pub disabled: Option<String>,
}

impl Suggestion {
    pub fn new(name: &str, detail: &str) -> Self {
        Self { icon: String::new(), name: clip(name), detail: clip(detail), disabled: None }
    }

    pub fn icon(mut self, name: &str) -> Self {
        self.icon = clip(name);
        self
    }

    pub fn disabled(mut self, why: &str) -> Self {
        self.disabled = Some(clip(why));
        self
    }

    /// Lze ji vybrat a prijmout?
    pub fn usable(&self) -> bool {
        self.disabled.is_none()
    }
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

/// Kde seznam lezi vuci kotve (kdyz se tam nevejde, otoci se).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Side {
    /// Pod kotvou (pole v okne).
    #[default]
    Below,
    /// Nad kotvou (radek vstupu terminalu).
    Above,
}

/// Co udalost se seznamem udelala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Seznamu nepatri: klavesa jde do pole, udalost mysi dal.
    Ignored,
    /// Zmenil se vyber, najeti nebo posun (prekreslit); udalost spotrebovana.
    Moved,
    /// Prijmout polozku `i` (Enter, Tab, klik).
    Accept(usize),
    /// Zavrit (Esc; stisk mysi mimo seznam - ten pritom jde dal).
    Close,
}

/// Rozmery odvozene z tokenu (`Completion::place`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Metrics {
    row_h: u32,
    pad: u32,
    head_h: u32,
}

/// Stav seznamu navrhu (samostatne pouziti; ve strome `Ui` ho drzi
/// knihovna).
#[derive(Debug, Clone, Default)]
pub struct Completion {
    /// Zahlavi ("Cesty", "Prikazy"); prazdne = bez zahlavi.
    pub header: String,
    pub items: Vec<Suggestion>,
    /// Vybrana polozka (aktivni potomek pole).
    pub selected: Option<usize>,
    /// Polozka pod ukazatelem.
    pub hover: Option<usize>,
    /// Posun seznamu (px).
    pub scroll: u32,
    /// Obdelnik radku / kurzoru, u ktereho seznam lezi.
    pub anchor: Rect,
    /// Nad nebo pod kotvou.
    pub prefer: Side,
    rect: Rect,
    m: Metrics,
    pointer: (i32, i32),
    /// Po nastaveni polozek ukazat vybranou pri pristim `place`.
    reveal_pending: bool,
}

impl Completion {
    /// Seznam se zahlavim a polozkami (orizne je na [`MAX_ITEMS`]
    /// a [`MAX_TEXT`]); vybrana je prvni pouzitelna.
    pub fn new(header: &str, items: Vec<Suggestion>) -> Self {
        let mut c = Self::default();
        c.set_items(header, items);
        c
    }

    /// Nove polozky: vyber na prvni pouzitelnou, posun na zacatek.
    pub fn set_items(&mut self, header: &str, mut items: Vec<Suggestion>) {
        items.truncate(MAX_ITEMS);
        for it in &mut items {
            if it.name.chars().count() > MAX_TEXT {
                it.name = clip(&it.name);
            }
            if it.detail.chars().count() > MAX_TEXT {
                it.detail = clip(&it.detail);
            }
        }
        self.header = clip(header);
        self.items = items;
        self.selected = self.items.iter().position(Suggestion::usable);
        self.hover = None;
        self.scroll = 0;
        self.reveal_pending = true;
    }

    /// Obdelnik seznamu (po [`Completion::place`]).
    pub fn rect(&self) -> Rect {
        self.rect
    }

    /// Spocita obdelnik u kotvy uvnitr `area` (stejne souradnice jako
    /// kotva) a vrati ho. `min_hit` = nejmensi cil z kontextu okna (dotyk
    /// 44 px), 0 = podle tokenu. Volat po zmene kotvy, polozek nebo plochy.
    pub fn place(&mut self, theme: &Theme, min_hit: u32, area: Rect) -> Rect {
        let sp = &theme.space;
        let head_h = if self.header.is_empty() { 0 } else { theme.text.caption.line + 2 * sp[2] };
        self.m = Metrics { row_h: theme.size.control_h.max(min_hit).max(1), pad: sp[2], head_h };
        let content = 2 * self.m.pad + head_h + self.items.len() as u32 * self.m.row_h;
        let want = content.min(theme.size.completion_max_h.max(self.m.row_h + 2 * self.m.pad + head_h));
        let w = theme.size.completion_w.min(area.w);
        let a = self.anchor;
        let below_room = (area.bottom() - a.bottom() - ANCHOR_GAP as i64).max(0) as u32;
        let above_room = (a.y as i64 - area.y as i64 - ANCHOR_GAP as i64).max(0) as u32;
        // Preferovana strana, kdyz se tam nevejde a na druhe je vic mista,
        // druha.
        let side = match self.prefer {
            Side::Below if below_room < want && above_room > below_room => Side::Above,
            Side::Above if above_room < want && below_room > above_room => Side::Below,
            s => s,
        };
        let h = want.min(if side == Side::Below { below_room } else { above_room });
        let y = match side {
            Side::Below => a.bottom() as i32 + ANCHOR_GAP,
            Side::Above => a.y - ANCHOR_GAP - h as i32,
        };
        let x = (a.x as i64).clamp(area.x as i64, (area.right() - w as i64).max(area.x as i64)) as i32;
        self.rect = Rect::new(x, y, w, h);
        self.clamp_scroll();
        // Vybranou ukazat jen po nastaveni polozek (posun kolecka zustane).
        if self.reveal_pending
            && let Some(k) = self.selected
        {
            self.reveal(k);
        }
        self.reveal_pending = false;
        self.rect
    }

    /// Plocha radku (bez okraje a zahlavi).
    fn list_rect(&self) -> Rect {
        let r = self.rect;
        let top = self.m.pad + self.m.head_h;
        Rect::new(r.x + self.m.pad as i32, r.y + top as i32, r.w.saturating_sub(2 * self.m.pad), r.h.saturating_sub(top + self.m.pad))
    }

    fn total_h(&self) -> u32 {
        self.items.len() as u32 * self.m.row_h
    }

    fn clamp_scroll(&mut self) {
        let max = self.total_h().saturating_sub(self.list_rect().h);
        self.scroll = self.scroll.min(max);
    }

    /// Posune seznam, aby byla polozka `k` videt.
    fn reveal(&mut self, k: usize) {
        let top = k as u32 * self.m.row_h;
        let view = self.list_rect().h;
        if top < self.scroll {
            self.scroll = top;
        } else if top + self.m.row_h > self.scroll + view {
            self.scroll = (top + self.m.row_h).saturating_sub(view);
        }
        self.clamp_scroll();
    }

    /// Prvni pouzitelna polozka od `from` smerem `dir` (bez preskoceni
    /// konce); `None` = zadna.
    fn usable_from(&self, from: usize, dir: i32) -> Option<usize> {
        let n = self.items.len() as i64;
        let mut k = from as i64;
        while (0..n).contains(&k) {
            if self.items[k as usize].usable() {
                return Some(k as usize);
            }
            k += dir as i64;
        }
        None
    }

    /// Dalsi pouzitelna polozka dokola.
    fn step_wrap(&self, dir: i32) -> Option<usize> {
        let n = self.items.len();
        if n == 0 {
            return None;
        }
        let start = match (self.selected, dir > 0) {
            (Some(s), _) => s,
            (None, true) => n - 1,
            (None, false) => 0,
        };
        (1..=n).map(|d| if dir > 0 { (start + d) % n } else { (start + n - d % n) % n }).find(|&k| self.items[k].usable())
    }

    fn select(&mut self, k: Option<usize>) {
        if let Some(k) = k {
            self.selected = Some(k);
            self.reveal(k);
        }
    }

    /// Klavesa (pole ma fokus, seznam je otevreny). `mods_now` =
    /// modifikatory (`inputproto::mods`).
    pub fn key(&mut self, ev: &InputEvent, mods_now: u16) -> Outcome {
        let InputEvent::Key { code, action, .. } = *ev else { return Outcome::Ignored };
        if action == 0 || mods_now & (mods::CTRL | mods::ALT) != 0 {
            return Outcome::Ignored;
        }
        let shift = mods_now & mods::SHIFT != 0;
        let n = self.items.len();
        let page = (self.list_rect().h / self.m.row_h.max(1)).max(1) as usize;
        let to = match code {
            KEY_ESC => return Outcome::Close,
            KEY_ENTER | KEY_KPENTER => return self.accept_selected(),
            KEY_TAB if !shift => return self.accept_selected(),
            _ if n == 0 => return Outcome::Ignored,
            KEY_DOWN => self.step_wrap(1),
            KEY_UP => self.step_wrap(-1),
            KEY_HOME => self.usable_from(0, 1),
            KEY_END => self.usable_from(n - 1, -1),
            KEY_PAGEDOWN => {
                let k = self.selected.map_or(0, |s| (s + page).min(n - 1));
                self.usable_from(k, 1).or_else(|| self.usable_from(k, -1))
            }
            KEY_PAGEUP => {
                let k = self.selected.map_or(0, |s| s.saturating_sub(page));
                self.usable_from(k, -1).or_else(|| self.usable_from(k, 1))
            }
            _ => return Outcome::Ignored,
        };
        self.select(to);
        Outcome::Moved
    }

    fn accept_selected(&self) -> Outcome {
        match self.selected.filter(|&k| self.items.get(k).is_some_and(Suggestion::usable)) {
            Some(k) => Outcome::Accept(k),
            None => Outcome::Ignored,
        }
    }

    /// Udalost mysi (souradnice jako kotva). Najeti zvyrazni, klik
    /// prijme, kolecko posouva; stisk mimo seznam vrati `Close` (klik
    /// pritom patri tomu, na co miri).
    pub fn pointer(&mut self, ev: &InputEvent) -> Outcome {
        match *ev {
            InputEvent::PointerMove { x, y } => {
                self.pointer = (x, y);
                let h = self.hit(x, y);
                if h != self.hover {
                    self.hover = h;
                    return Outcome::Moved;
                }
                Outcome::Ignored
            }
            InputEvent::PointerLeave => {
                self.pointer = (i32::MIN, i32::MIN);
                if self.hover.take().is_some() { Outcome::Moved } else { Outcome::Ignored }
            }
            InputEvent::Button { pressed: true, x, y, code } => {
                self.pointer = (x, y);
                if !self.rect.contains(x, y) {
                    return Outcome::Close;
                }
                match self.hit(x, y) {
                    Some(k) if code == BTN_LEFT => Outcome::Accept(k),
                    _ => Outcome::Moved,
                }
            }
            InputEvent::Button { pressed: false, x, y, .. } => {
                if self.rect.contains(x, y) { Outcome::Moved } else { Outcome::Ignored }
            }
            InputEvent::Scroll { dy, .. } => {
                let (x, y) = self.pointer;
                if !self.rect.contains(x, y) {
                    return Outcome::Ignored;
                }
                let step = dy.unsigned_abs().saturating_mul(self.m.row_h);
                self.scroll = if dy > 0 { self.scroll.saturating_sub(step) } else { self.scroll.saturating_add(step) };
                self.clamp_scroll();
                self.hover = self.hit(x, y);
                Outcome::Moved
            }
            _ => Outcome::Ignored,
        }
    }

    /// Pouzitelna polozka v bodu (souradnice jako kotva).
    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        let l = self.list_rect();
        if !l.contains(x, y) || self.m.row_h == 0 {
            return None;
        }
        let k = ((y - l.y) as u32 + self.scroll) / self.m.row_h;
        let k = k as usize;
        self.items.get(k).filter(|it| it.usable()).map(|_| k)
    }

    /// Obdelnik radku `k` (souradnice jako kotva, muze lezet mimo
    /// viditelnou plochu).
    pub fn row_rect(&self, k: usize) -> Rect {
        let l = self.list_rect();
        Rect::new(l.x, l.y + (k as u32 * self.m.row_h) as i32 - self.scroll as i32, l.w, self.m.row_h)
    }

    /// Nakresli seznam do platna, jehoz pocatek lezi v `origin`
    /// (souradnice kotvy; platno okna -> `(0, 0)`, platno vrstvy popoveru
    /// -> jeho poloha). Pismo jmena: `mono` (tokeny `font.mono`), bez nej
    /// pisma UI.
    pub fn draw(&self, c: &mut Canvas<'_>, origin: (i32, i32), p: &mut Paint<'_>) {
        let th = p.theme;
        let col = &th.color;
        let sp = &th.space;
        let shift = |r: Rect| Rect::new(r.x - origin.0, r.y - origin.1, r.w, r.h);
        let r = shift(self.rect);
        if r.is_empty() {
            return;
        }
        let edge = if th.mode == theme::Mode::Contrast { 2 } else { 1 };
        c.round_fill_bordered(r, th.radius.card, col.surface, col.border, edge);
        let cap = th.text.caption;
        let strong = th.text.body_strong.weight;
        let gap = sp[3] as i32;
        let old = c.clip();
        if self.m.head_h > 0 {
            let hs = TextStyle { weight: strong, ..cap };
            let hr = Rect::new(r.x + self.m.pad as i32, r.y + self.m.pad as i32, r.w.saturating_sub(2 * self.m.pad), self.m.head_h);
            c.set_clip(old.intersect(&hr));
            let f = p.fonts.face(hs.weight);
            let b = hr.y + f.baseline_in(hs.size, hr.h);
            f.draw_ellipsized(c, hr.x + gap, b, &self.header, hs.size, col.ink_muted, hr.w.saturating_sub(2 * gap as u32));
        }
        let list = shift(self.list_rect());
        c.set_clip(old.intersect(&list));
        let small = th.text.small;
        let is = th.size.icon_s;
        let icon_col = th.size.icon_m as i32;
        let first = (self.scroll / self.m.row_h.max(1)) as usize;
        for k in first..self.items.len() {
            let row = shift(self.row_rect(k));
            if row.y as i64 >= list.bottom() {
                break;
            }
            let it = &self.items[k];
            if self.selected == Some(k) {
                c.round_fill(row, th.radius.xs, col.accent_soft);
            } else if self.hover == Some(k) {
                c.round_fill(row, th.radius.xs, col.surface_3);
            }
            let ink = if it.usable() { col.ink } else { col.ink_faint };
            let mut x = row.x + gap;
            if !it.icon.is_empty() {
                p.icons.draw(c, &it.icon, x + (icon_col - is as i32) / 2, row.y + (row.h - is) as i32 / 2, is, th.icon_stroke, ink);
            }
            x += icon_col + gap;
            // Typ vpravo (auto sirka), jmeno do zbytku.
            let detail = it.disabled.as_deref().unwrap_or(&it.detail);
            let df = p.fonts.face(cap.weight);
            let dw = if detail.is_empty() { 0 } else { (df.width(detail, cap.size) as i32).min(row.w as i32 / 2) };
            if dw > 0 {
                let b = row.y + df.baseline_in(cap.size, row.h);
                df.draw_ellipsized(c, row.right() as i32 - gap - dw, b, detail, cap.size, col.ink_muted, dw as u32);
            }
            let room = (row.right() as i32 - gap - dw - if dw > 0 { gap } else { 0 } - x).max(0) as u32;
            let f = match p.mono.as_deref_mut() {
                Some(m) => m.face(strong),
                None => p.fonts.face(strong),
            };
            let b = row.y + f.baseline_in(small.size, row.h);
            f.draw_ellipsized(c, x, b, &it.name, small.size, ink, room);
        }
        c.set_clip(old);
    }

    /// Uzly pristupnosti: seznam `list_id` (role `ListBox`, jmeno =
    /// zahlavi) pod rodicem `parent` (pole) a polozky s id `option_id(k)`
    /// (role `ListOption`; vybrana ma `SELECTED | ACTIVE`). `layer` =
    /// vrstva, ve ktere lezi obdelniky (souradnice jako kotva).
    pub fn a11y(&self, parent: u32, list_id: u32, option_id: impl Fn(usize) -> u32, layer: u32) -> Vec<A11yNode> {
        use a11y::state::*;
        let mut out = Vec::with_capacity(self.items.len() + 1);
        out.push(A11yNode { id: list_id, parent, role: Role::ListBox, name: self.header.clone(), value: String::new(), state: 0, layer, bounds: self.rect, actions: a11y::actions::SCROLL });
        let list = self.list_rect();
        for (k, it) in self.items.iter().enumerate() {
            let mut state = 0;
            if self.selected == Some(k) {
                state |= SELECTED | ACTIVE;
            }
            if self.hover == Some(k) {
                state |= HOVER;
            }
            let name = match &it.disabled {
                Some(why) => {
                    state |= DISABLED;
                    alloc::format!("{} ({why})", it.name)
                }
                None => it.name.clone(),
            };
            let bounds = self.row_rect(k).intersect(&list);
            out.push(A11yNode { id: option_id(k), parent: list_id, role: Role::ListOption, name, value: it.detail.clone(), state, layer, bounds, actions: if it.usable() { a11y::actions::ACTIVATE } else { 0 } });
        }
        out
    }

    /// Akce pristupnosti na polozce `k` (ctecka, agent): `ACTIVATE`
    /// prijme, `FOCUS` vybere.
    pub fn a11y_action(&mut self, k: usize, action: u32) -> Outcome {
        if !self.items.get(k).is_some_and(Suggestion::usable) {
            return Outcome::Ignored;
        }
        if action & a11y::actions::ACTIVATE != 0 {
            return Outcome::Accept(k);
        }
        if action & a11y::actions::FOCUS != 0 {
            self.select(Some(k));
            return Outcome::Moved;
        }
        Outcome::Ignored
    }
}

/// Co kresleni potrebuje: motiv, pisma UI, volitelne mono a ikony.
pub struct Paint<'a> {
    pub theme: &'a Theme,
    pub fonts: &'a mut Family,
    pub mono: Option<&'a mut Family>,
    pub icons: &'a mut Icons,
}
