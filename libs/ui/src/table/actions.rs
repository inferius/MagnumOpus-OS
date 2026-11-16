//! Tlacitka v radku tabulky (`Cells::actions`): ikonova tlacitka ve stylu
//! `ghost` vpravo v poslednim sloupci (design/plocha.html, Spravce uloh).
//!
//! - Mys: stisk na tlacitku posle `on_row_action` (id radku, id akce)
//!   a nemeni vyber; najeti zvyrazni tlacitko.
//! - Klavesnice: v radku s kurzorem Sipka vpravo / vlevo prechazi mezi
//!   tlacitky (prvni Sipka vpravo vstoupi na prvni, Sipka vlevo z prvniho
//!   vrati fokus radku), Enter nebo Mezernik tlacitko stiskne. Zmena radku
//!   fokus tlacitka zrusi.
//! - Pristupnost: kazde tlacitko je uzel `Button` pod uzlem radku (jmeno =
//!   popisek, neaktivni = `DISABLED`), akce `activate` ho stiskne.

use alloc::vec::Vec;

use gfx::{Canvas, Rect};
use guiproto::a11y::{self, Node as A11yNode, Role};
use inputproto::code::*;

use crate::event::Response;
use crate::node::CellAction;
use crate::{State, Ui};

use super::virt_id;

/// Druh virtualniho uzlu pristupnosti pro tlacitko radku.
const A11Y_KIND: u64 = 3;

/// Cast pro `hover_part`: horni bit = tlacitko, pak radek a poradi.
const PART: u32 = 0x8000_0000;

/// Id uzlu tlacitka `idx` v radku `rid`.
fn node_id(table: u32, rid: u64, idx: usize) -> u32 {
    virt_id(table, A11Y_KIND, rid.wrapping_mul(64).wrapping_add(idx as u64))
}

impl<M: Clone> Ui<M> {
    /// Obdelniky tlacitek radku `k` (souradnice prostoru), zprava doleva
    /// v poslednim sloupci; poradi odpovida `Cells::actions`.
    pub(super) fn action_rects(&self, i: usize, k: usize, n: usize) -> Vec<Rect> {
        let Some(&(cx, cw)) = self.col_xs(i).last() else { return Vec::new() };
        let g = self.geo(i);
        let sp = &self.look.theme.space;
        let scroll = self.tables.get(&self.elems[i].key).map_or(0, |s| s.scroll);
        let y = g.body.y + k as i32 * g.rh as i32 - scroll;
        let s = g.rh.saturating_sub(2 * sp[1]);
        let right = cx + cw as i32 - sp[2] as i32;
        (0..n).map(|j| Rect::new(right - ((n - j) as u32 * (s + sp[0])) as i32, y + sp[1] as i32, s, s)).collect()
    }

    /// Tlacitka radku `k`.
    fn row_actions(&self, i: usize, k: usize) -> Vec<CellAction> {
        self.spec(i).map_or(Vec::new(), |t| (t.row)(k).actions)
    }

    /// Tlacitko pod bodem prostoru: (radek, poradi).
    pub(super) fn action_at(&self, i: usize, sx: i32, sy: i32) -> Option<(usize, usize)> {
        let k = self.row_at(i, sy)?;
        let acts = self.row_actions(i, k);
        if acts.is_empty() {
            return None;
        }
        let j = self.action_rects(i, k, acts.len()).iter().position(|r| r.contains(sx, sy))?;
        Some((k, j))
    }

    /// Cast pro najeti (`table_part`).
    pub(super) fn action_part(&self, i: usize, sx: i32, sy: i32) -> Option<u32> {
        self.action_at(i, sx, sy).map(|(k, j)| PART | ((k as u32) << 6 & 0x7FFF_FFC0) | j as u32)
    }

    /// Stisk tlacitka `j` v radku `k` (je-li aktivni).
    pub(super) fn press_action(&mut self, i: usize, k: usize, j: usize, r: &mut Response<M>) {
        let Some(t) = self.spec(i) else { return };
        let id = (t.id)(k);
        let acts = (t.row)(k).actions;
        if let (Some(a), Some(f)) = (acts.get(j), &t.on_row_action)
            && a.enabled
        {
            r.msgs.push(f(id, a.id));
        }
        r.redraw = true;
    }

    /// Klavesy tlacitek v radku s kurzorem; `true` = klavesu vzaly.
    pub(super) fn action_key(&mut self, i: usize, cur: Option<usize>, code: u16, r: &mut Response<M>) -> bool {
        let key = self.elems[i].key;
        let Some(k) = cur else { return false };
        let n = self.row_actions(i, k).len();
        let at = self.tables.get(&key).and_then(|s| s.action);
        match code {
            KEY_RIGHT if n > 0 => {
                let next = at.map_or(0, |j| (j + 1).min(n - 1));
                self.tables.entry(key).or_default().action = Some(next);
                r.redraw = true;
                true
            }
            KEY_LEFT if at.is_some() => {
                self.tables.entry(key).or_default().action = at.and_then(|j| j.checked_sub(1));
                r.redraw = true;
                true
            }
            KEY_ENTER | KEY_KPENTER | KEY_SPACE if at.is_some_and(|j| j < n) => {
                self.press_action(i, k, at.unwrap(), r);
                true
            }
            _ => false,
        }
    }

    /// Nakresli tlacitka radku `k`: ikona `ink_muted` (neaktivni s kryti
    /// `opacity.disabled`), najeti `surface_3`, fokus z klavesnice prstenec.
    pub(super) fn draw_actions(&mut self, c: &mut Canvas<'_>, i: usize, k: usize, acts: &[CellAction], hover: Option<u32>, st: State, focused_action: Option<usize>) {
        let th = self.look.theme.clone();
        let col = &th.color;
        for (j, (a, r)) in acts.iter().zip(self.action_rects(i, k, acts.len())).enumerate() {
            let part = PART | ((k as u32) << 6 & 0x7FFF_FFC0) | j as u32;
            if a.enabled && hover == Some(part) {
                c.round_fill(r, th.radius.control, col.surface_3);
            }
            if st.focus_visible && focused_action == Some(j) {
                c.round_stroke(r, th.radius.control, th.size.focus_ring, col.focus);
            }
            let is = th.size.icon_s;
            let ink = if a.enabled { col.ink_muted } else { crate::draw::fade(col.ink_muted, col.surface, th.opacity.disabled) };
            self.icons.draw(c, a.icon, r.x + (r.w as i32 - is as i32) / 2, r.y + (r.h as i32 - is as i32) / 2, is, th.icon_stroke, ink);
        }
    }

    /// Uzly pristupnosti tlacitek radku `k` (pod uzlem radku `row`).
    pub(super) fn actions_a11y(&self, i: usize, table: u32, row: u32, k: usize, rid: u64, layer: u32, out: &mut Vec<A11yNode>) {
        let acts = self.row_actions(i, k);
        let key = self.elems[i].key;
        let at = self.tables.get(&key).and_then(|s| s.action);
        let cur = self.tables.get(&key).and_then(|s| s.cursor).and_then(|c| self.resolve(i, c));
        for (j, (a, r)) in acts.iter().zip(self.action_rects(i, k, acts.len())).enumerate() {
            let mut state = 0;
            if !a.enabled {
                state |= a11y::state::DISABLED;
            }
            if self.focus == Some(i) && cur == Some(k) && at == Some(j) {
                state |= a11y::state::FOCUSED;
            }
            out.push(A11yNode {
                id: node_id(table, rid, j),
                parent: row,
                role: Role::Button,
                name: a.label.clone(),
                value: alloc::string::String::new(),
                state,
                layer,
                bounds: r,
                actions: if a.enabled { a11y::actions::ACTIVATE } else { 0 },
            });
        }
    }

    /// Akce pristupnosti na tlacitku radku; `true` = uzel byl tlacitko.
    pub(super) fn actions_a11y_action(&mut self, i: usize, table: u32, node: u32, action: u32, r: &mut Response<M>) -> bool {
        let Some(t) = self.spec(i) else { return false };
        let g = self.geo(i);
        let scroll = self.tables.get(&self.elems[i].key).map_or(0, |s| s.scroll);
        let rh = g.rh as i32;
        let first = (scroll / rh).max(0) as usize;
        let last = (((scroll + g.body.h as i32) / rh) as usize + 1).min(t.count);
        for k in first..last {
            let rid = (t.id)(k);
            let n = (t.row)(k).actions.len();
            if let Some(j) = (0..n).find(|&j| node_id(table, rid, j) == node) {
                if action & a11y::actions::ACTIVATE != 0 {
                    self.press_action(i, k, j, r);
                }
                return true;
            }
        }
        false
    }
}
