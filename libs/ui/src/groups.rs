//! Klavesnice skupin prvku jako na Windows / macOS / GNOME:
//!
//! - **radio**: sourozenci se stejnym rodicem jsou skupina a jedna
//!   zastavka Tab (vybrana volba, jinak prvni); sipky presunou fokus na
//!   dalsi / predchozi volbu (dokola) a rovnou ji vyberou;
//! - **zavreny vyber** (`select`): pismena skoci na volbu zacinajici
//!   napsanym textem (hledani psanim, `TYPEAHEAD` od posledniho znaku),
//!   Home / End prvni a posledni volba;
//! - **seznam** (`list`): Home / End, PgUp / PgDn (stranka), hledani
//!   psanim.

use alloc::string::String;
use alloc::vec::Vec;

use inputproto::code::*;

use crate::event::Response;
use crate::layout::ElemKind;
use crate::Ui;

/// Jak dlouho po poslednim znaku hledani psanim pokracuje (ns).
pub const TYPEAHEAD: u64 = 1_000_000_000;

impl<M: Clone> Ui<M> {
    /// Volby skupiny radio prvku `i` (poradi v okne), bez nedostupnych.
    pub(crate) fn radio_group(&self, i: usize) -> Vec<usize> {
        let p = self.elems[i].parent;
        (0..self.elems.len()).filter(|&j| self.elems[j].parent == p && matches!(self.elems[j].kind, ElemKind::Radio { .. }) && !self.elems[j].disabled).collect()
    }

    /// Je prvek zastavkou Tab? Radio jen vybrane (bez vybrane prvni ve
    /// skupine); volba s fokusem vzdy (at Tab ze skupiny pokracuje).
    pub(crate) fn tab_stop(&self, i: usize) -> bool {
        let e = &self.elems[i];
        if !e.focusable() || e.skip_tab {
            return false;
        }
        if !matches!(e.kind, ElemKind::Radio { .. }) || self.focus == Some(i) {
            return true;
        }
        let g = self.radio_group(i);
        let checked = g.iter().copied().find(|&j| matches!(self.elems[j].kind, ElemKind::Radio { checked: true, .. }));
        match checked {
            Some(c) => c == i && !g.iter().any(|&j| Some(j) == self.focus),
            None => g.first() == Some(&i) && !g.iter().any(|&j| Some(j) == self.focus),
        }
    }

    /// Sipka ve skupine radio: dalsi / predchozi volba dokola, vybrana.
    pub(crate) fn radio_key(&mut self, i: usize, code: u16, r: &mut Response<M>) -> bool {
        let dir: i32 = match code {
            KEY_RIGHT | KEY_DOWN => 1,
            KEY_LEFT | KEY_UP => -1,
            _ => return false,
        };
        let g = self.radio_group(i);
        let Some(p) = g.iter().position(|&j| j == i) else { return false };
        if g.len() < 2 {
            return false;
        }
        let n = g.len() as i32;
        let next = g[((p as i32 + dir).rem_euclid(n)) as usize];
        self.focus = Some(next);
        if let ElemKind::Radio { checked: false, on_click: Some(m), .. } = &self.elems[next].kind {
            r.msgs.push(m.clone());
        }
        r.redraw = true;
        true
    }

    /// Hledani psanim: prida znak k hledanemu textu (novy text po
    /// `TYPEAHEAD`) a vrati ho malymi pismeny.
    fn typeahead(&mut self, ch: char) -> String {
        let fresh = self.now.saturating_sub(self.typed.1) > TYPEAHEAD;
        if fresh {
            self.typed.0.clear();
        }
        self.typed.0.extend(ch.to_lowercase());
        self.typed.1 = self.now;
        self.typed.0.clone()
    }

    /// Prvni polozka od `start` (dokola), ktera zacina `pre`. Stejne
    /// pismeno opakovane = dalsi polozka na to pismeno (jako Windows).
    fn find_prefix(labels: &[String], start: usize, pre: &str) -> Option<usize> {
        let n = labels.len();
        if n == 0 {
            return None;
        }
        let mut cs = pre.chars();
        let first = cs.next()?;
        // "aaa" = cyklus po polozkach na "a".
        let same = cs.all(|c| c == first);
        let (want, from) = if same && pre.chars().count() > 1 { (first.to_lowercase().collect::<String>(), start + 1) } else { (String::from(pre), start) };
        (0..n).map(|d| (from + d) % n).find(|&k| labels[k].to_lowercase().starts_with(&want))
    }

    /// Klavesa na zavrenem vyberu (pismena, Home, End). `false` = neni
    /// pro vyber.
    pub(crate) fn select_key(&mut self, i: usize, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        let ElemKind::Select { options, selected, .. } = &self.elems[i].kind else { return false };
        let (n, cur) = (options.len(), *selected);
        let labels = options.clone();
        let k = match (code, ch) {
            (KEY_HOME, _) => Some(0),
            (KEY_END, _) => n.checked_sub(1),
            (_, Some(c)) if !c.is_control() && c != ' ' => {
                let pre = self.typeahead(c);
                Self::find_prefix(&labels, if pre.chars().count() == 1 { cur + 1 } else { cur }, &pre)
            }
            _ => return false,
        };
        if let Some(k) = k
            && k != cur
            && let ElemKind::Select { on_select: Some(f), .. } = &self.elems[i].kind
        {
            r.msgs.push(f(k));
        }
        r.redraw = true;
        true
    }

    /// Klavesa na seznamu (Home, End, PgUp, PgDn, pismena); sipky resi
    /// `event.rs`. `false` = nezpracovano.
    pub(crate) fn list_key(&mut self, i: usize, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        let ElemKind::List { count, .. } = &self.elems[i].kind else { return false };
        let n = *count;
        let items: Vec<(usize, String, bool)> = self
            .elems
            .iter()
            .filter_map(|e| match &e.kind {
                ElemKind::ListItem { index, text, selected, list, .. } if *list == i + 1 => Some((*index, text.clone(), *selected)),
                _ => None,
            })
            .collect();
        let cur = items.iter().find(|t| t.2).map(|t| t.0);
        let page = (self.elems[i].rect.h / self.look.row_h().max(1)).max(1) as usize;
        let k = match (code, ch) {
            (KEY_HOME, _) => Some(0),
            (KEY_END, _) => n.checked_sub(1),
            (KEY_PAGEUP, _) => Some(cur.unwrap_or(0).saturating_sub(page)),
            (KEY_PAGEDOWN, _) => Some((cur.map_or(0, |c| c + page)).min(n.saturating_sub(1))),
            (_, Some(c)) if !c.is_control() && c != ' ' => {
                let pre = self.typeahead(c);
                let labels: Vec<String> = items.iter().map(|t| t.1.clone()).collect();
                let start = cur.map_or(0, |c| if pre.chars().count() == 1 { c + 1 } else { c });
                Self::find_prefix(&labels, start, &pre).map(|p| items[p].0)
            }
            _ => return false,
        };
        if let Some(k) = k
            && n > 0
            && Some(k) != cur
            && let ElemKind::List { on_select: Some(f), .. } = &self.elems[i].kind
        {
            r.msgs.push(f(k));
        }
        r.redraw = true;
        true
    }
}
