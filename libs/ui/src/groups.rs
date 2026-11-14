//! Klavesnice skupin prvku jako na Windows / macOS / GNOME:
//!
//! - **radio**: sourozenci se stejnym rodicem jsou skupina a jedna
//!   zastavka Tab (vybrana volba, jinak prvni); sipky presunou fokus na
//!   dalsi / predchozi volbu (dokola) a rovnou ji vyberou;
//! - **zavreny vyber** (`select`): pismena skoci na volbu zacinajici
//!   napsanym textem (hledani psanim, `TYPEAHEAD` od posledniho znaku),
//!   Home / End prvni a posledni volba;
//! - **seznam** (`list`): Home / End, PgUp / PgDn (stranka), hledani
//!   psanim; s `multi_select` vyber vic radku jako v tabulce (Shift
//!   rozsah, Ctrl prepne / jen kurzor, Ctrl+mezernik, Ctrl+A).

use alloc::string::String;
use alloc::vec::Vec;

use inputproto::code::*;
use inputproto::mods;

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

    /// Klavesa na seznamu: Home, End, PgUp, PgDn, hledani psanim; s
    /// vicenasobnym vyberem i sipky (Shift rozsah, Ctrl jen kurzor),
    /// mezernik (Ctrl+mezernik prepne radek) a Ctrl+A. Sipky jednoducheho
    /// seznamu resi `event.rs`. `false` = nezpracovano.
    pub(crate) fn list_key(&mut self, i: usize, code: u16, ch: Option<char>, r: &mut Response<M>) -> bool {
        let ElemKind::List { count, multi, .. } = &self.elems[i].kind else { return false };
        let (n, multi) = (*count, *multi);
        let key = self.elems[i].key;
        let (ctrl, shift) = (self.mods & mods::CTRL != 0, self.mods & mods::SHIFT != 0);
        let items: Vec<(usize, String, bool)> = self
            .elems
            .iter()
            .filter_map(|e| match &e.kind {
                ElemKind::ListItem { index, text, selected, list, .. } if *list == i + 1 => Some((*index, text.clone(), *selected)),
                _ => None,
            })
            .collect();
        let first_sel = items.iter().find(|t| t.2).map(|t| t.0);
        let cur = if multi { self.lists.get(&key).and_then(|s| s.0).filter(|&c| c < n).or(first_sel) } else { first_sel };
        let page = (self.elems[i].rect.h / self.look.row_h().max(1)).max(1) as usize;
        let last = n.saturating_sub(1);
        let to = match code {
            KEY_UP if multi => Some(cur.map_or(0, |c| c.saturating_sub(1))),
            KEY_DOWN if multi => Some(cur.map_or(0, |c| (c + 1).min(last))),
            KEY_HOME => Some(0),
            KEY_END => Some(last),
            KEY_PAGEUP => Some(cur.unwrap_or(0).saturating_sub(page)),
            KEY_PAGEDOWN => Some(cur.map_or(0, |c| c + page).min(last)),
            _ => None,
        };
        if n == 0 {
            return to.is_some();
        }
        if let Some(k) = to {
            if multi && ctrl {
                // Jen kurzor (vyber zustane), mezernikem pak prepnout.
                self.lists.entry(key).or_default().0 = Some(k);
                r.redraw = true;
            } else if multi {
                self.list_select(i, k, false, shift, r);
            } else {
                self.list_pick_single(i, k, cur, r);
            }
            return true;
        }
        if multi && code == KEY_SPACE {
            self.list_select(i, cur.unwrap_or(0), ctrl, false, r);
            return true;
        }
        let letter = ch.filter(|c| c.is_alphabetic()).map(|c| c.to_lowercase().next().unwrap_or(c)).or((code == KEY_A).then_some('a'));
        if multi && ctrl && letter == Some('a') {
            let all: Vec<u64> = (0..n as u64).collect();
            self.list_emit(i, all, r);
            return true;
        }
        if ctrl {
            return false;
        }
        let Some(c) = ch.filter(|c| !c.is_control() && *c != ' ') else { return false };
        let pre = self.typeahead(c);
        let labels: Vec<String> = items.iter().map(|t| t.1.clone()).collect();
        let start = cur.map_or(0, |c| if pre.chars().count() == 1 { c + 1 } else { c });
        if let Some(p) = Self::find_prefix(&labels, start, &pre) {
            let k = items[p].0;
            if multi {
                self.list_select(i, k, false, false, r);
            } else {
                self.list_pick_single(i, k, cur, r);
            }
        }
        r.redraw = true;
        true
    }

    /// Jednoduchy seznam: vybrat radek `k` (`on_select`), kdyz se zmeni.
    fn list_pick_single(&mut self, i: usize, k: usize, cur: Option<usize>, r: &mut Response<M>) {
        if Some(k) != cur
            && let ElemKind::List { on_select: Some(f), .. } = &self.elems[i].kind
        {
            r.msgs.push(f(k));
        }
        r.redraw = true;
    }

    /// Vyber radku `k` seznamu `i` s vicenasobnym vyberem (klik, sipky,
    /// mezernik) podle modifikatoru; logika jako u tabulky
    /// (`selection.rs`).
    pub(crate) fn list_select(&mut self, i: usize, k: usize, ctrl: bool, shift: bool, r: &mut Response<M>) {
        let key = self.elems[i].key;
        let cur: Vec<u64> = self
            .elems
            .iter()
            .filter_map(|e| match e.kind {
                ElemKind::ListItem { index, selected: true, list, .. } if list == i + 1 => Some(index as u64),
                _ => None,
            })
            .collect();
        let anchor = self.lists.get(&key).and_then(|s| s.1);
        let range = |a: usize, b: usize| (a.min(b) as u64..=a.max(b) as u64).collect::<Vec<u64>>();
        let sel = crate::selection::pick(&cur, &range, anchor, k, k as u64, ctrl, shift, true);
        let s = self.lists.entry(key).or_default();
        s.0 = Some(k);
        if crate::selection::moves_anchor(shift, true, s.1.is_some()) {
            s.1 = Some(k);
        }
        self.list_emit(i, sel, r);
    }

    fn list_emit(&mut self, i: usize, sel: Vec<u64>, r: &mut Response<M>) {
        if let ElemKind::List { on_selection: Some(f), .. } = &self.elems[i].kind {
            r.msgs.push(f(sel));
        }
        r.redraw = true;
    }

    /// Kurzor seznamu s vicenasobnym vyberem (radek s prstencem).
    pub(crate) fn list_cursor(&self, i: usize) -> Option<usize> {
        self.lists.get(&self.elems[i].key).and_then(|s| s.0)
    }
}
