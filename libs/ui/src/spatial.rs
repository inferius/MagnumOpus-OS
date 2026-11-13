//! Prostorova navigace fokusu (dalkovy ovladac, gamepad, sipky):
//! sipka, kterou prvek s fokusem nespotreboval, presune fokus na nejblizsi
//! prvek v danem smeru (design/televize.html). Poradi Tab zustava pro
//! klavesnici a ctecku.
//!
//! - **Vyber:** kandidat musi lezet ve smeru sipky (stred dal nez stred
//!   prvku s fokusem); skore = vzdalenost ve smeru + 2 x kolma odchylka.
//! - **Skupiny** (`Node::focus_group`): sipky podel osy skupiny zustanou
//!   v ni (na kraji se fokus nehne), kolme ze skupiny vystoupi. Pri
//!   navratu do skupiny dostane fokus naposledy vybrany prvek (pamet).
//! - **Vychozi fokus** (`Node::default_focus`): pri startu a vzdy, kdyz
//!   prvek s fokusem zmizi (fokus je vzdy nekde).

use gfx::Rect;
use inputproto::code::*;

use crate::{Key, Ui};

/// Osa skupiny fokusu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Rada (vlevo / vpravo zustava ve skupine).
    Horizontal,
    /// Sloupec (nahoru / dolu zustava ve skupine).
    Vertical,
    /// Mrizka (vsechny sipky zustavaji ve skupine).
    Grid,
}

fn center(r: Rect) -> (i64, i64) {
    (r.x as i64 + r.w as i64 / 2, r.y as i64 + r.h as i64 / 2)
}

impl<M: Clone> Ui<M> {
    /// Nejblizsi predek prvku `i` se skupinou fokusu (index, osa).
    pub(crate) fn group_of(&self, i: usize) -> Option<(usize, Axis)> {
        let mut p = self.elems[i].parent;
        while p > 0 {
            let g = p - 1;
            if let Some(a) = self.elems[g].focus_group {
                return Some((g, a));
            }
            p = self.elems[g].parent;
        }
        None
    }

    /// Lezi prvek `i` ve skupine `g` (kdekoli pod ni)?
    fn in_group(&self, i: usize, g: usize) -> bool {
        let mut p = self.elems[i].parent;
        while p > 0 {
            if p - 1 == g {
                return true;
            }
            p = self.elems[p - 1].parent;
        }
        false
    }

    /// Zapamatuje prvek s fokusem ve vsech jeho skupinach.
    pub(crate) fn remember_focus(&mut self) {
        let Some(f) = self.focus else { return };
        let key = self.elems[f].key;
        let mut p = self.elems[f].parent;
        while p > 0 {
            let g = p - 1;
            if self.elems[g].focus_group.is_some() {
                self.group_memory.insert(self.elems[g].key, key);
            }
            p = self.elems[g].parent;
        }
    }

    /// Prvek s vychozim fokusem (`Node::default_focus`), jinak prvni
    /// zamerovatelny.
    pub(crate) fn default_focus_index(&self) -> Option<usize> {
        self.elems.iter().position(|e| e.default_focus && e.focusable()).or_else(|| self.elems.iter().position(|e| e.focusable()))
    }

    /// Sipka `code` presune fokus prostorove. Vraci, zda se fokus hnul.
    pub(crate) fn spatial_move(&mut self, code: u16) -> bool {
        let (dx, dy): (i64, i64) = match code {
            KEY_LEFT => (-1, 0),
            KEY_RIGHT => (1, 0),
            KEY_UP => (0, -1),
            KEY_DOWN => (0, 1),
            _ => return false,
        };
        let Some(f) = self.focus else {
            self.focus = self.default_focus_index();
            return self.focus.is_some();
        };
        let (cx, cy) = center(self.window_rect(f));
        let group = self.group_of(f);
        // Podel osy skupiny jen ve skupine, kolmo jen mimo ni.
        let along = group.is_some_and(|(_, a)| match a {
            Axis::Horizontal => dx != 0,
            Axis::Vertical => dy != 0,
            Axis::Grid => true,
        });
        let mut best: Option<(i64, usize)> = None;
        for i in 0..self.elems.len() {
            if i == f || !self.elems[i].focusable() {
                continue;
            }
            if let Some((g, _)) = group {
                if along != self.in_group(i, g) {
                    continue;
                }
            }
            let (x, y) = center(self.window_rect(i));
            let primary = (x - cx) * dx + (y - cy) * dy;
            if primary <= 0 {
                continue;
            }
            let ortho = ((x - cx) * dy).abs() + ((y - cy) * dx).abs();
            let score = primary + 2 * ortho;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, i));
            }
        }
        let Some((_, mut to)) = best else { return false };
        // Vstup do jine skupiny: naposledy vybrany prvek v ni.
        if let Some((g, _)) = self.group_of(to)
            && group.is_none_or(|(cg, _)| cg != g)
            && let Some(k) = self.group_memory.get(&self.elems[g].key).copied()
            && let Some(m) = self.index_of(k)
            && self.elems[m].focusable()
        {
            to = m;
        }
        self.focus = Some(to);
        self.remember_focus();
        true
    }

    /// Klic skupiny, ve ktere je prvek s fokusem (testy, aplikace).
    pub fn focus_group_key(&self) -> Option<Key> {
        let f = self.focus?;
        self.group_of(f).map(|(g, _)| self.elems[g].key)
    }
}
