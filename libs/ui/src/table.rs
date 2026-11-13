//! Tabulka a seznam s virtualizaci (`table`): kresli a do stromu
//! pristupnosti dava jen viditelne radky, data cte line pres `id` a `row`
//! ze [`crate::node::TableSpec`]. Tisice polozek (spravce souboru) tak
//! stoji jen tolik, kolik radku je videt.
//!
//! - **Vyber** drzi aplikace; tabulka posila novy vyber (`on_selection`).
//!   Klik vybere radek, Ctrl+klik prepne, Shift+klik rozsah od kotvy,
//!   Ctrl+A vse. Klavesy: sipky, PgUp/PgDn, Home/End (Shift = rozsah,
//!   Ctrl = jen kurzor), mezernik (Ctrl = prepnout), Enter = `on_activate`,
//!   psani = skok na radek zacinajici napsanym textem.
//! - **Razeni:** klik na zahlavi razeneho sloupce (`on_sort`), znovu =
//!   opacne; tah za okraj zahlavi meni sirku sloupce.
//! - **Tazeni:** tah vybranych radku o vic nez [`DRAG_START`] px posle
//!   `on_drag` s jejich id (cil a pusteni resi aplikace).
//! - **Rolovani:** kolecko plynule (animace k cili po snimcich, viz
//!   [`Ui::tick`]), posuvnik [`crate::scrollbar`], klavesy hned.

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use guiproto::a11y::{self, Node as A11yNode, Role};
use guiproto::window::InputEvent;
use inputproto::code::*;
use inputproto::mods;

use crate::event::Response;
use crate::field::MULTI_CLICK;
use crate::layout::ElemKind;
use crate::node::TableSpec;
use crate::scrollbar::{Hit, Scrollbar};
use crate::{Key, State, Ui};

/// Od kolika px pohybu se stisk na radku stane tazenim.
pub const DRAG_START: i32 = 6;

/// Jak dlouho drzi napsany text pro skok na radek (ns).
const FIND_RESET: u64 = 1_000_000_000;

/// Radku na jeden krok kolecka.
const WHEEL_ROWS: i32 = 3;

/// Prave tazene radky (id) a tabulka, odkud jsou (pro cil pusteni).
pub(crate) type Dragging = (Key, Vec<u64>);

/// Okoli okraje zahlavi pro zmenu sirky sloupce (px).
const RESIZE_GRIP: i32 = 4;

/// Stav tabulky podle klice prvku (prezije prerozvrzeni).
#[derive(Debug, Clone, Default)]
pub(crate) struct TableState {
    pub scroll: i32,
    /// Cil plynuleho rolovani.
    pub target: i32,
    /// Kurzor a kotva rozsahu: (index-napoveda, id).
    pub cursor: Option<(usize, u64)>,
    pub anchor: Option<(usize, u64)>,
    /// Sirky sloupcu zmenene tazenim (prazdne = ze `Col::width`).
    pub widths: Vec<u32>,
    find: String,
    find_time: u64,
    last_click: Option<(u64, u64)>,
}

/// Rozpracovany stisk v tabulce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Press {
    /// Radek: `deferred` = vyber az pri pusteni (stisk na vybranem radku
    /// muze byt zacatek tazeni).
    Row { key: Key, idx: usize, id: u64, x: i32, y: i32, deferred: bool, dragged: bool },
    Header { key: Key, col: usize },
    Resize { key: Key, col: usize, start_x: i32, start_w: u32 },
    Thumb { key: Key, grab: i32 },
}

impl Press {
    fn key(&self) -> Key {
        match *self {
            Press::Row { key, .. } | Press::Header { key, .. } | Press::Resize { key, .. } | Press::Thumb { key, .. } => key,
        }
    }
}

/// Geometrie tabulky (souradnice prostoru prvku).
struct Geo {
    rect: Rect,
    header: Option<Rect>,
    body: Rect,
    rh: u32,
}

/// Id virtualniho uzlu pristupnosti (zahlavi, radek) v rozsahu nad id
/// prvku a pod uzly chyb.
fn virt_id(table: u32, kind: u64, n: u64) -> u32 {
    0x4000_0000 | (Key::ROOT.mix(table as u64).mix(kind).mix(n).0 as u32 & 0x3FFF_FFFF)
}

/// Text pro hledani psanim (mala pismena).
fn fold(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

impl<M: Clone> Ui<M> {
    pub(crate) fn spec(&self, i: usize) -> Option<&TableSpec<M>> {
        match &self.elems.get(i)?.kind {
            ElemKind::Table(t) => Some(t),
            _ => None,
        }
    }

    fn geo(&self, i: usize) -> Geo {
        let rect = self.elems[i].rect;
        let rh = self.look.row_h();
        let header = self.spec(i).is_some_and(|t| t.header);
        let inner = Rect::new(rect.x + 1, rect.y + 1, rect.w.saturating_sub(2), rect.h.saturating_sub(2));
        if header {
            let h = Rect::new(inner.x, inner.y, inner.w, rh);
            Geo { rect, header: Some(h), body: Rect::new(inner.x, inner.y + rh as i32, inner.w, inner.h.saturating_sub(rh)), rh }
        } else {
            Geo { rect, header: None, body: inner, rh }
        }
    }

    /// Sloupce: (x, sirka) v souradnicich prostoru.
    fn col_xs(&self, i: usize) -> Vec<(i32, u32)> {
        let Some(t) = self.spec(i) else { return Vec::new() };
        let g = self.geo(i);
        let st = self.tables.get(&self.elems[i].key);
        let widths: Vec<u32> = t.columns.iter().enumerate().map(|(k, c)| st.and_then(|s| s.widths.get(k).copied()).unwrap_or(c.width)).collect();
        let fixed: u32 = widths.iter().sum();
        let flex = widths.iter().filter(|&&w| w == 0).count() as u32;
        let rest = g.body.w.saturating_sub(fixed);
        let mut x = g.body.x;
        widths
            .iter()
            .map(|&w| {
                let w = if w == 0 { rest / flex.max(1) } else { w };
                let o = (x, w);
                x += w as i32;
                o
            })
            .collect()
    }

    fn content_h(&self, i: usize) -> u32 {
        self.spec(i).map_or(0, |t| t.count as u32 * self.look.row_h())
    }

    fn max_scroll(&self, i: usize) -> i32 {
        (self.content_h(i) as i32 - self.geo(i).body.h as i32).max(0)
    }

    /// Index radku s id `id` (napoveda `hint`, jinak hledani).
    fn resolve(&self, i: usize, (hint, id): (usize, u64)) -> Option<usize> {
        let t = self.spec(i)?;
        if hint < t.count && (t.id)(hint) == id {
            return Some(hint);
        }
        (0..t.count).find(|&k| (t.id)(k) == id)
    }

    fn row_at(&self, i: usize, y: i32) -> Option<usize> {
        let g = self.geo(i);
        let scroll = self.tables.get(&self.elems[i].key).map_or(0, |s| s.scroll);
        if y < g.body.y || y >= g.body.bottom() as i32 {
            return None;
        }
        let k = ((y - g.body.y + scroll) / g.rh as i32) as usize;
        (k < self.spec(i)?.count).then_some(k)
    }

    fn scrollbar(&self, i: usize) -> Option<Scrollbar> {
        let scroll = self.tables.get(&self.elems[i].key).map_or(0, |s| s.scroll);
        Scrollbar::new(self.geo(i).body, self.content_h(i), scroll, &self.look.theme)
    }

    /// Posune tak, aby byl radek `k` videt (hned, bez animace).
    fn reveal_row(&mut self, i: usize, k: usize) {
        let g = self.geo(i);
        let max = self.max_scroll(i);
        let key = self.elems[i].key;
        let s = self.tables.entry(key).or_default();
        let top = k as i32 * g.rh as i32;
        let mut y = s.target;
        if top < y {
            y = top;
        } else if top + g.rh as i32 > y + g.body.h as i32 {
            y = top + g.rh as i32 - g.body.h as i32;
        }
        s.scroll = y.clamp(0, max);
        s.target = s.scroll;
    }

    /// Novy vyber: zprava aplikaci, kdyz se lisi.
    fn emit_selection(&self, i: usize, sel: Vec<u64>, r: &mut Response<M>) {
        let Some(t) = self.spec(i) else { return };
        if sel != t.selection
            && let Some(f) = &t.on_selection
        {
            r.msgs.push(f(sel));
        }
        r.redraw = true;
    }

    fn ids(&self, i: usize, a: usize, b: usize) -> Vec<u64> {
        let Some(t) = self.spec(i) else { return Vec::new() };
        let (a, b) = (a.min(b), a.max(b).min(t.count.saturating_sub(1)));
        (a..=b).map(|k| (t.id)(k)).collect()
    }

    /// Je radek `k` nadpis sekce?
    fn is_header(&self, i: usize, k: usize) -> bool {
        self.spec(i).is_some_and(|t| k < t.count && (t.row)(k).header)
    }

    /// Nejblizsi vybiratelny radek od `k` smerem `dir` (nadpisy preskoci);
    /// kdyz tam zadny neni, opacnym smerem.
    fn selectable(&self, i: usize, k: usize, dir: i32) -> Option<usize> {
        let n = self.spec(i)?.count as i64;
        for d in [dir as i64, -dir as i64] {
            let mut j = k as i64;
            while (0..n).contains(&j) {
                if !self.is_header(i, j as usize) {
                    return Some(j as usize);
                }
                j += d;
            }
        }
        None
    }

    /// Vyber radku `k` podle modifikatoru (`ctrl` prepne, `shift` rozsah).
    fn select_row(&mut self, i: usize, k: usize, ctrl: bool, shift: bool, r: &mut Response<M>) {
        if self.is_header(i, k) {
            return;
        }
        let Some(t) = self.spec(i) else { return };
        let id = (t.id)(k);
        let multi = t.multi;
        let cur = t.selection.clone();
        let key = self.elems[i].key;
        let anchor = self.tables.get(&key).and_then(|s| s.anchor).and_then(|a| self.resolve(i, a));
        let sel = if !multi {
            alloc::vec![id]
        } else if shift && let Some(a) = anchor {
            let mut v = self.ids(i, a, k);
            if ctrl {
                for x in cur {
                    if !v.contains(&x) {
                        v.push(x);
                    }
                }
            }
            v
        } else if ctrl {
            let mut v = cur;
            match v.iter().position(|&x| x == id) {
                Some(p) => {
                    v.remove(p);
                }
                None => v.push(id),
            }
            v
        } else {
            alloc::vec![id]
        };
        let s = self.tables.entry(key).or_default();
        s.cursor = Some((k, id));
        if !(shift && multi) || s.anchor.is_none() {
            s.anchor = Some((k, id));
        }
        self.emit_selection(i, sel, r);
    }

    /// Udalost pro tabulku. `None` = tabulka ji nezpracovala.
    pub(crate) fn table_event(&mut self, ev: &InputEvent, time: Option<u64>) -> Option<Response<M>> {
        let mut r = Response::default();
        match *ev {
            InputEvent::Button { code, pressed: true, x, y } if code == BTN_LEFT || code == BTN_RIGHT => {
                let i = self.hit(x, y)?;
                self.spec(i)?;
                let (sx, sy) = self.to_space(i, x, y);
                self.focus = Some(i);
                self.pressed = None;
                self.hover = Some(i);
                r.redraw = true;
                let key = self.elems[i].key;
                let g = self.geo(i);
                if code == BTN_RIGHT {
                    if let Some(k) = self.row_at(i, sy) {
                        let id = (self.spec(i)?.id)(k);
                        if !self.spec(i)?.selection.contains(&id) {
                            self.select_row(i, k, false, false, &mut r);
                        }
                        if let Some(f) = &self.spec(i)?.on_context {
                            r.msgs.push(f(id));
                        }
                    }
                    self.open_context(i, Some((x, y)));
                    return Some(r);
                }
                if let Some(sb) = self.scrollbar(i)
                    && let Some(h) = sb.hit(sx, sy)
                {
                    let s = self.tables.entry(key).or_default();
                    match h {
                        Hit::Thumb { grab } => self.table_press = Some(Press::Thumb { key, grab }),
                        Hit::PageUp => s.target = (s.target - g.body.h as i32).max(0),
                        Hit::PageDown => s.target = (s.target + g.body.h as i32).min(sb.max),
                    }
                    return Some(r);
                }
                if let Some(h) = g.header
                    && h.contains(sx, sy)
                {
                    let cols = self.col_xs(i);
                    let last = cols.len().saturating_sub(1);
                    for (c, &(cx, cw)) in cols.iter().enumerate() {
                        let edge = cx + cw as i32;
                        if c < last && (sx - edge).abs() <= RESIZE_GRIP {
                            self.table_press = Some(Press::Resize { key, col: c, start_x: sx, start_w: cw });
                            return Some(r);
                        }
                    }
                    if let Some(c) = cols.iter().position(|&(cx, cw)| sx >= cx && sx < cx + cw as i32) {
                        self.table_press = Some(Press::Header { key, col: c });
                    }
                    return Some(r);
                }
                let Some(k) = self.row_at(i, sy) else { return Some(r) };
                if self.is_header(i, k) {
                    return Some(r);
                }
                let t = self.spec(i)?;
                let id = (t.id)(k);
                let (ctrl, shift) = (self.mods & mods::CTRL != 0, self.mods & mods::SHIFT != 0);
                // Dvojklik: aktivace.
                let st = self.tables.entry(key).or_default();
                let double = matches!((st.last_click, time), (Some((lid, lt)), Some(now)) if lid == id && now.saturating_sub(lt) <= MULTI_CLICK);
                st.last_click = time.map(|t| (id, t));
                if double {
                    st.last_click = None;
                    if let Some(f) = &self.spec(i)?.on_activate {
                        r.msgs.push(f(id));
                    }
                    return Some(r);
                }
                let t = self.spec(i)?;
                let deferred = !ctrl && !shift && t.selection.contains(&id) && (t.selection.len() > 1 || t.on_drag.is_some());
                if !deferred {
                    self.select_row(i, k, ctrl, shift, &mut r);
                }
                self.table_press = Some(Press::Row { key, idx: k, id, x: sx, y: sy, deferred, dragged: false });
                Some(r)
            }
            InputEvent::PointerMove { x, y } => {
                let p = self.table_press?;
                let i = self.index_of(p.key())?;
                self.pointer = (x, y);
                let (sx, sy) = self.to_space(i, x, y);
                r.redraw = true;
                match p {
                    Press::Thumb { key, grab } => {
                        let sb = self.scrollbar(i)?;
                        let o = sb.offset_for(sy, grab);
                        let s = self.tables.entry(key).or_default();
                        (s.scroll, s.target) = (o, o);
                    }
                    Press::Resize { key, col, start_x, start_w } => {
                        let min = self.look.theme.size.hit_min;
                        let n = self.spec(i)?.columns.len();
                        let cols = self.col_xs(i);
                        let s = self.tables.entry(key).or_default();
                        if s.widths.len() != n {
                            s.widths = cols.iter().map(|c| c.1).collect();
                        }
                        s.widths[col] = (start_w as i32 + sx - start_x).max(min as i32) as u32;
                    }
                    Press::Row { key, idx, id, x: px, y: py, deferred, dragged: false } if (sx - px).abs().max((sy - py).abs()) > DRAG_START => {
                        let t = self.spec(i)?;
                        let ids = if t.selection.contains(&id) { t.selection.clone() } else { alloc::vec![id] };
                        if let Some(f) = &t.on_drag {
                            r.msgs.push(f(ids.clone()));
                            self.dragging = Some((key, ids));
                        }
                        self.table_press = Some(Press::Row { key, idx, id, x: px, y: py, deferred, dragged: true });
                    }
                    // Tah pokracuje: cil pod ukazatelem (zvyrazneni radku).
                    Press::Row { dragged: true, .. } => {
                        self.hover = self.hit(x, y);
                    }
                    _ => return None,
                }
                Some(r)
            }
            InputEvent::Button { code: BTN_LEFT, pressed: false, x, y } => {
                let p = self.table_press.take()?;
                let i = self.index_of(p.key())?;
                let (sx, sy) = self.to_space(i, x, y);
                r.redraw = true;
                // Pusteni tazenych radku na tabulku s `on_drop`.
                if let Some((_, ids)) = self.dragging.take()
                    && let Some(t) = self.hit(x, y)
                    && let Some(f) = self.spec(t).and_then(|s| s.on_drop.as_ref())
                {
                    let (_, ty) = self.to_space(t, x, y);
                    let target = self.row_at(t, ty).map(|k| (self.spec(t).unwrap().id)(k));
                    // Ne na sebe sama.
                    if target.is_none_or(|g| !ids.contains(&g)) {
                        r.msgs.push(f(target, ids));
                    }
                }
                match p {
                    Press::Row { idx, id, deferred: true, dragged: false, .. } => {
                        let k = self.resolve(i, (idx, id))?;
                        self.select_row(i, k, false, false, &mut r);
                    }
                    Press::Header { col, .. } => {
                        let g = self.geo(i);
                        let cols = self.col_xs(i);
                        let over = g.header.is_some_and(|h| h.contains(sx, sy)) && cols.get(col).is_some_and(|&(cx, cw)| sx >= cx && sx < cx + cw as i32);
                        let t = self.spec(i)?;
                        if over && t.columns.get(col).is_some_and(|c| c.sortable) {
                            let asc = !matches!(t.sort, Some((c, true)) if c == col);
                            if let Some(f) = &t.on_sort {
                                r.msgs.push(f(col, asc));
                            }
                        }
                    }
                    _ => {}
                }
                Some(r)
            }
            InputEvent::Scroll { dy, .. } => {
                let i = self.hit(self.pointer.0, self.pointer.1)?;
                self.spec(i)?;
                let max = self.max_scroll(i);
                let step = WHEEL_ROWS * self.look.row_h() as i32;
                let s = self.tables.entry(self.elems[i].key).or_default();
                let t = (s.target - dy * step).clamp(0, max);
                if t == s.target {
                    // Na kraji: kolecko dostane oblast pod tabulkou.
                    return None;
                }
                s.target = t;
                r.redraw = true;
                Some(r)
            }
            InputEvent::Key { code, action, ch } if action != 0 && code != KEY_TAB => {
                let i = self.focus?;
                self.spec(i)?;
                self.table_key(i, code, ch, &mut r).then_some(r)
            }
            InputEvent::Text(ch) => {
                let i = self.focus?;
                self.spec(i)?;
                if ch.is_control() || self.mods & mods::CTRL != 0 {
                    return Some(r);
                }
                self.table_find(i, ch, time.unwrap_or(0), &mut r);
                Some(r)
            }
            _ => None,
        }
    }

    fn table_key(&mut self, i: usize, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        let Some(t) = self.spec(i) else { return false };
        let (n, multi) = (t.count, t.multi);
        if n == 0 {
            return false;
        }
        let key = self.elems[i].key;
        let (ctrl, shift) = (self.mods & mods::CTRL != 0, self.mods & mods::SHIFT != 0);
        let cur = self.tables.get(&key).and_then(|s| s.cursor).and_then(|c| self.resolve(i, c));
        let page = (self.geo(i).body.h / self.look.row_h()).max(1) as usize;
        let to = match code {
            KEY_UP => Some(cur.map_or(0, |c| c.saturating_sub(1))),
            KEY_DOWN => Some(cur.map_or(0, |c| (c + 1).min(n - 1))),
            KEY_PAGEUP => Some(cur.map_or(0, |c| c.saturating_sub(page))),
            KEY_PAGEDOWN => Some(cur.map_or(0, |c| (c + page).min(n - 1))),
            KEY_HOME => Some(0),
            KEY_END => Some(n - 1),
            _ => None,
        };
        // Nadpisy sekci se preskoci (smer podle klavesy).
        let dir = if matches!(code, KEY_UP | KEY_PAGEUP | KEY_END) { -1 } else { 1 };
        let to = to.and_then(|k| self.selectable(i, k, dir));
        if let Some(k) = to {
            if ctrl && multi {
                // Jen kurzor (vyber zustane), mezernikem pak prepnout.
                let id = (self.spec(i).unwrap().id)(k);
                self.tables.entry(key).or_default().cursor = Some((k, id));
                r.redraw = true;
            } else {
                self.select_row(i, k, false, shift, r);
            }
            self.reveal_row(i, k);
            return true;
        }
        let letter = ch.filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).or((code == KEY_A).then_some('a'));
        match code {
            KEY_SPACE => {
                let k = cur.unwrap_or(0);
                self.select_row(i, k, ctrl, false, r);
                self.reveal_row(i, k);
            }
            KEY_ENTER | KEY_KPENTER => {
                if let Some(k) = cur {
                    let t = self.spec(i).unwrap();
                    if let Some(f) = &t.on_activate {
                        r.msgs.push(f((t.id)(k)));
                    }
                }
            }
            _ if ctrl && multi && letter == Some('a') => {
                let all = self.ids(i, 0, n - 1);
                self.emit_selection(i, all, r);
            }
            _ => return false,
        }
        true
    }

    /// Psani v tabulce: skok na radek, jehoz prvni bunka zacina napsanym
    /// textem (mezi pismeny nejvys sekunda).
    fn table_find(&mut self, i: usize, ch: char, now: u64, r: &mut Response<M>) {
        let key = self.elems[i].key;
        let cur = self.tables.get(&key).and_then(|s| s.cursor).and_then(|c| self.resolve(i, c));
        let s = self.tables.entry(key).or_default();
        if now.saturating_sub(s.find_time) > FIND_RESET {
            s.find.clear();
        }
        s.find_time = now;
        s.find.extend(ch.to_lowercase());
        let want = s.find.clone();
        let Some(t) = self.spec(i) else { return };
        let n = t.count;
        // Prvni pismeno hleda od dalsiho radku (opakovanim se prochazi).
        let start = cur.map_or(0, |c| if want.chars().count() == 1 { c + 1 } else { c });
        let hit = (0..n).map(|d| (start + d) % n.max(1)).find(|&k| (t.row)(k).cells.first().is_some_and(|c| fold(c).starts_with(&want)));
        if let Some(k) = hit {
            self.select_row(i, k, false, false, r);
            self.reveal_row(i, k);
        }
    }

    /// Krok plynuleho rolovani (volat na snimek). Vraci, zda se neco
    /// pohnulo (prekreslit).
    pub fn tick(&mut self) -> bool {
        let mut moved = false;
        if self.has_busy() {
            self.busy_phase = self.busy_phase.wrapping_add(self.look.theme.space[2]);
            moved = true;
        }
        for s in self.tables.values_mut() {
            if s.scroll != s.target {
                let d = s.target - s.scroll;
                let step = if d.abs() < 3 { d } else { d / 3 };
                s.scroll += step;
                moved = true;
            }
        }
        moved
    }

    /// Bezi animace (rolovani), takze je treba dalsi snimek?
    pub fn wants_frame(&self) -> bool {
        self.has_busy() || self.tables.values().any(|s| s.scroll != s.target)
    }

    /// Stav tabulek po rozvrzeni: zmizele pryc, posun v mezich.
    pub(crate) fn sync_tables(&mut self) {
        let keys: Vec<(Key, usize)> = (0..self.elems.len()).filter(|&i| self.spec(i).is_some()).map(|i| (self.elems[i].key, i)).collect();
        self.tables.retain(|k, _| keys.iter().any(|(j, _)| j == k));
        for (k, i) in keys {
            let max = self.max_scroll(i);
            let s = self.tables.entry(k).or_default();
            s.scroll = s.scroll.clamp(0, max);
            s.target = s.target.clamp(0, max);
        }
        if self.table_press.is_some_and(|p| self.index_of(p.key()).is_none()) {
            self.table_press = None;
            self.dragging = None;
        }
    }

    /// Nakresli tabulku `i`.
    pub(crate) fn draw_table(&mut self, c: &mut Canvas<'_>, i: usize, st: State) {
        let th = self.look.theme.clone();
        let col = &th.color;
        let sp = &th.space;
        let g = self.geo(i);
        let key = self.elems[i].key;
        let ts = self.tables.get(&key).cloned().unwrap_or_default();
        c.round_fill_bordered(g.rect, th.radius.card, col.surface, col.border, 1);
        if st.focus_visible {
            crate::draw::focus_ring(c, g.rect, th.radius.card, th.size.focus_ring, th.size.focus_offset, col.focus);
        }
        let cols = self.col_xs(i);
        let old = c.clip();
        let inner = Rect::new(g.rect.x + 1, g.rect.y + 1, g.rect.w.saturating_sub(2), g.rect.h.saturating_sub(2));
        c.set_clip(old.intersect(&inner));
        let Some(spec) = self.spec(i) else { return };
        let (count, sort) = (spec.count, spec.sort);
        let titles: Vec<(String, bool)> = spec.columns.iter().map(|k| (k.title.clone(), k.numeric)).collect();
        let selection = spec.selection.clone();
        // Zahlavi: caption s vahou body_strong, barva ink_muted, sipka razeni.
        if let Some(h) = g.header {
            let hs = theme::TextStyle { weight: th.text.body_strong.weight, ..th.text.caption };
            for (k, &(x, w)) in cols.iter().enumerate() {
                let (title, numeric) = &titles[k];
                let f = self.look.face(hs);
                let tw = f.width(title, hs.size) as i32;
                let base = h.y + f.baseline_in(hs.size, h.h);
                let room = w.saturating_sub(2 * sp[3]);
                let tx = if *numeric { x + w as i32 - sp[3] as i32 - tw } else { x + sp[3] as i32 };
                f.draw_ellipsized(c, tx, base, title, hs.size, col.ink_muted, room);
                if let Some((sc, asc)) = sort
                    && sc == k
                {
                    let is = th.size.icon_xs;
                    let ix = if *numeric { tx - is as i32 - sp[1] as i32 } else { tx + tw + sp[1] as i32 };
                    self.icons.draw(c, if asc { "up" } else { "down" }, ix, h.y + (h.h - is) as i32 / 2, is, th.icon_stroke, col.ink_muted);
                }
            }
            c.fill(Rect::new(h.x, h.bottom() as i32 - 1, h.w, 1), col.border);
        }
        // Viditelne radky.
        c.set_clip(old.intersect(&g.body));
        let rh = g.rh as i32;
        let first = (ts.scroll / rh).max(0) as usize;
        let last = (((ts.scroll + g.body.h as i32) / rh) as usize + 1).min(count);
        let cursor = ts.cursor.and_then(|cu| self.resolve(i, cu));
        let dropping = self.dragging.is_some() && self.spec(i).is_some_and(|s| s.on_drop.is_some());
        let hover_row = if self.hover == Some(i) && (self.table_press.is_none() || dropping) {
            let (px, py) = self.to_space(i, self.pointer.0, self.pointer.1);
            g.body.contains(px, py).then(|| self.row_at(i, py)).flatten()
        } else {
            None
        };
        let bs = self.look.body();
        for k in first..last {
            let Some(spec) = self.spec(i) else { break };
            let (id, cells) = ((spec.id)(k), (spec.row)(k));
            let y = g.body.y + k as i32 * rh - ts.scroll;
            let rr = Rect::new(g.body.x, y, g.body.w, g.rh);
            // Nadpis sekce (`.sec`): caption s vahou body_strong, velka
            // pismena, ink_muted; bez cary a bez vyberu.
            if cells.header {
                let hs = theme::TextStyle { weight: th.text.body_strong.weight, ..th.text.caption };
                let text: String = cells.cells.first().map_or(String::new(), |t| t.to_uppercase());
                let f = self.look.face(hs);
                let base = y + f.baseline_in(hs.size, g.rh);
                f.draw_ellipsized(c, rr.x + sp[3] as i32, base, &text, hs.size, col.ink_muted, rr.w.saturating_sub(2 * sp[3]));
                continue;
            }
            if selection.contains(&id) {
                c.fill(rr, col.accent_soft);
            } else if hover_row == Some(k) {
                c.fill(rr, col.surface_3);
            }
            // Cil pusteni: ram akcentu kolem radku pod ukazatelem.
            if dropping && hover_row == Some(k) && !self.dragging.as_ref().is_some_and(|d| d.1.contains(&id)) {
                c.round_stroke(rr, th.radius.control, 2, col.accent);
            }
            c.fill(Rect::new(rr.x, rr.bottom() as i32 - 1, rr.w, 1), col.border);
            for (ci, &(x, w)) in cols.iter().enumerate() {
                let mut tx = x + sp[3] as i32;
                let mut room = w.saturating_sub(2 * sp[3]);
                if ci == 0
                    && let Some(icon) = cells.icon
                {
                    let is = th.size.icon_s;
                    self.icons.draw(c, icon, tx, y + (rh - is as i32) / 2, is, th.icon_stroke, col.ink_muted);
                    tx += (is + sp[2]) as i32;
                    room = room.saturating_sub(is + sp[2]);
                }
                let Some(text) = cells.cells.get(ci) else { continue };
                let f = self.look.face(bs);
                let base = y + f.baseline_in(bs.size, g.rh);
                if titles.get(ci).is_some_and(|t| t.1) {
                    let tw = f.width(text, bs.size).min(room) as i32;
                    f.draw_ellipsized(c, x + w as i32 - sp[3] as i32 - tw, base, text, bs.size, col.ink, room);
                } else {
                    f.draw_ellipsized(c, tx, base, text, bs.size, col.ink, room);
                }
            }
            // Kurzor klavesnice v aktivni tabulce.
            if st.focus_visible && cursor == Some(k) {
                let ring = th.size.focus_ring;
                let fr = Rect::new(rr.x + ring as i32, rr.y + ring as i32, rr.w.saturating_sub(2 * ring), rr.h.saturating_sub(2 * ring));
                c.round_stroke(fr, th.radius.control, ring, col.focus);
            }
        }
        c.set_clip(old.intersect(&inner));
        if let Some(sb) = self.scrollbar(i) {
            let (px, py) = self.to_space(i, self.pointer.0, self.pointer.1);
            let hover = self.hover == Some(i) && sb.hit(px, py).is_some();
            let active = matches!(self.table_press, Some(Press::Thumb { key: k, .. }) if k == key);
            sb.draw(c, &th, hover, active);
        }
        c.set_clip(old);
    }

    /// Uzly pristupnosti tabulky: zahlavi a jen viditelne radky (stromu
    /// staci, co je na obrazovce; ctecka roluje akci `scroll`).
    pub(crate) fn table_a11y(&self, i: usize, id: u32, layer: u32, out: &mut Vec<A11yNode>) {
        let Some(t) = self.spec(i) else { return };
        let g = self.geo(i);
        let cols = self.col_xs(i);
        if let Some(h) = g.header {
            for (k, c) in t.columns.iter().enumerate() {
                let (x, w) = cols[k];
                let value = match t.sort {
                    Some((s, true)) if s == k => String::from("vzestupně"),
                    Some((s, false)) if s == k => String::from("sestupně"),
                    _ => String::new(),
                };
                let actions = if c.sortable { a11y::actions::ACTIVATE } else { 0 };
                out.push(A11yNode { id: virt_id(id, 1, k as u64), parent: id, role: Role::ColumnHeader, name: c.title.clone(), value, state: 0, layer, bounds: Rect::new(x, h.y, w, h.h), actions });
            }
        }
        let ts = self.tables.get(&self.elems[i].key).cloned().unwrap_or_default();
        let rh = g.rh as i32;
        let first = (ts.scroll / rh).max(0) as usize;
        let last = (((ts.scroll + g.body.h as i32) / rh) as usize + 1).min(t.count);
        let cursor = ts.cursor.and_then(|c| self.resolve(i, c));
        for k in first..last {
            let rid = (t.id)(k);
            let cells = (t.row)(k);
            let y = g.body.y + k as i32 * rh - ts.scroll;
            let bounds = Rect::new(g.body.x, y, g.body.w, g.rh).intersect(&g.body);
            let mut state = 0;
            if t.selection.contains(&rid) {
                state |= a11y::state::SELECTED;
            }
            if cursor == Some(k) && self.focus == Some(i) {
                state |= a11y::state::FOCUSED;
            }
            out.push(A11yNode {
                id: virt_id(id, 2, rid),
                parent: id,
                role: if cells.header { Role::Label } else { Role::Row },
                name: cells.cells.join(", "),
                value: String::new(),
                state,
                layer,
                bounds,
                actions: a11y::actions::ACTIVATE,
            });
        }
    }

    /// Akce pristupnosti na virtualnim uzlu tabulky (radek: vybrat,
    /// zahlavi: seradit). Vraci, zda uzel patril nejake tabulce.
    pub(crate) fn table_a11y_action(&mut self, node: u32, action: u32, r: &mut Response<M>) -> bool {
        for i in 0..self.elems.len() {
            let Some(t) = self.spec(i) else { continue };
            let id = self.elem_ids[i];
            if let Some(c) = (0..t.columns.len()).find(|&c| virt_id(id, 1, c as u64) == node) {
                if action & a11y::actions::ACTIVATE != 0
                    && t.columns[c].sortable
                    && let Some(f) = &t.on_sort
                {
                    let asc = !matches!(t.sort, Some((s, true)) if s == c);
                    r.msgs.push(f(c, asc));
                }
                return true;
            }
            let mut nodes = Vec::new();
            self.table_a11y(i, id, 0, &mut nodes);
            let rows: Vec<u32> = nodes.iter().filter(|n| n.role == Role::Row).map(|n| n.id).collect();
            if rows.contains(&node) {
                let ts = self.tables.get(&self.elems[i].key).cloned().unwrap_or_default();
                let first = (ts.scroll / self.look.row_h() as i32).max(0) as usize;
                let k = first + rows.iter().position(|&x| x == node).unwrap();
                if action & (a11y::actions::ACTIVATE | a11y::actions::FOCUS) != 0 {
                    self.focus = Some(i);
                    self.select_row(i, k, false, false, r);
                }
                return true;
            }
        }
        false
    }
}
