//! Interaktivni tabulka vysledku: posun, vyber radku, razeni podle sloupce
//! a filtr (bez dotazu na sluzbu - data uz okno ma).

use alloc::string::String;
use alloc::vec::Vec;

use shlang::table::{cell_text, columns};
use shlang::value::Value;

#[derive(Debug, Clone)]
pub struct TableView {
    pub rows: Vec<Value>,
    pub cols: Vec<String>,
    /// Indexy radku po filtru a razeni.
    pub view: Vec<usize>,
    /// Vybrany radek (index do `view`).
    pub sel: usize,
    /// Prvni zobrazeny radek (index do `view`).
    pub top: usize,
    /// Razeni: (sloupec, sestupne).
    pub sort: Option<(usize, bool)>,
    pub filter: String,
}

/// Porovnani bunek: cisla, velikosti a casy podle hodnoty, text bez
/// ohledu na velikost pismen.
fn cmp_value(a: &Value, b: &Value) -> core::cmp::Ordering {
    use Value::*;
    let num = |v: &Value| match v {
        Int(i) | Size(i) | Duration(i) | Time(i) => Some(*i as f64),
        Float(f) => Some(*f),
        _ => None,
    };
    match (num(a), num(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(core::cmp::Ordering::Equal),
        _ => cell_text(a).to_lowercase().cmp(&cell_text(b).to_lowercase()),
    }
}

impl TableView {
    /// Tabulka z hodnoty (seznam zaznamu); jinak `None`.
    pub fn of(v: &Value) -> Option<TableView> {
        let Value::List(rows) = v else { return None };
        if rows.is_empty() || !rows.iter().all(|r| matches!(r, Value::Record(_))) {
            return None;
        }
        let cols = columns(rows);
        let mut t = TableView { rows: rows.clone(), cols, view: Vec::new(), sel: 0, top: 0, sort: None, filter: String::new() };
        t.refresh();
        Some(t)
    }

    fn refresh(&mut self) {
        let f = self.filter.to_lowercase();
        let mut view: Vec<usize> = (0..self.rows.len())
            .filter(|i| {
                f.is_empty()
                    || self.cols.iter().any(|c| {
                        self.rows[*i].field(c).is_some_and(|v| cell_text(&v).to_lowercase().contains(&f))
                    })
            })
            .collect();
        if let Some((c, desc)) = self.sort {
            let col = self.cols[c].clone();
            view.sort_by(|a, b| {
                let (x, y) = (self.rows[*a].field(&col), self.rows[*b].field(&col));
                let o = match (x, y) {
                    (Some(x), Some(y)) => cmp_value(&x, &y),
                    (Some(_), None) => core::cmp::Ordering::Less,
                    (None, Some(_)) => core::cmp::Ordering::Greater,
                    (None, None) => core::cmp::Ordering::Equal,
                };
                if desc { o.reverse() } else { o }
            });
        }
        self.view = view;
        self.sel = self.sel.min(self.view.len().saturating_sub(1));
        self.top = self.top.min(self.sel);
    }

    /// Klavesa `s`: razeni podle dalsiho sloupce; `S` obrati smer.
    pub fn sort_next(&mut self) {
        let c = match self.sort {
            None => 0,
            Some((c, _)) => (c + 1) % self.cols.len().max(1),
        };
        self.sort = Some((c, false));
        self.refresh();
    }

    pub fn sort_reverse(&mut self) {
        let (c, d) = self.sort.unwrap_or((0, false));
        self.sort = Some((c, !d));
        self.refresh();
    }

    pub fn sort_by(&mut self, col: &str, desc: bool) {
        if let Some(c) = self.cols.iter().position(|x| x == col) {
            self.sort = Some((c, desc));
            self.refresh();
        }
    }

    pub fn set_filter(&mut self, f: &str) {
        self.filter = f.into();
        self.sel = 0;
        self.top = 0;
        self.refresh();
    }

    /// Posun vyberu o `d` radku; `height` = kolik radku je videt.
    pub fn move_by(&mut self, d: isize, height: usize) {
        if self.view.is_empty() {
            return;
        }
        let max = self.view.len() as isize - 1;
        self.sel = (self.sel as isize + d).clamp(0, max) as usize;
        let h = height.max(1);
        if self.sel < self.top {
            self.top = self.sel;
        } else if self.sel >= self.top + h {
            self.top = self.sel + 1 - h;
        }
    }

    pub fn selected(&self) -> Option<&Value> {
        self.view.get(self.sel).map(|i| &self.rows[*i])
    }

    /// Viditelne radky (zaznamy) od `top`.
    pub fn visible(&self, height: usize) -> impl Iterator<Item = (bool, &Value)> {
        self.view.iter().enumerate().skip(self.top).take(height).map(move |(k, i)| (k == self.sel, &self.rows[*i]))
    }

    /// Popis stavu (`5 / 12 · razeno: size v · filtr: jpg`).
    pub fn status(&self) -> String {
        let mut s = alloc::format!("{} / {}", self.view.len(), self.rows.len());
        if let Some((c, d)) = self.sort {
            s.push_str(&alloc::format!(" · řazeno: {} {}", self.cols[c], if d { "sestupně" } else { "vzestupně" }));
        }
        if !self.filter.is_empty() {
            s.push_str(&alloc::format!(" · filtr: {}", self.filter));
        }
        s
    }
}
