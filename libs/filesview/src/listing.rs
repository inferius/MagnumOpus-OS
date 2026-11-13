//! Vypis slozky: polozky z `LIST_STAT_PAGE`, razeni (slozky prvni,
//! prirozene poradi jmen s cisly), filtr hledani a predpripravene texty
//! radku. Razeni a filtr stoji O(n log n) jednou pri zmene, ne pri
//! kazdem prekresleni: tabulka cte jen viditelne radky.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use abi::time::WallTime;
use fsproto::{StatEntry, avail, kind};
use tz::Zone;

use crate::format::{self, Avail, DASH};

/// Sloupec razeni (poradi sloupcu tabulky).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Name,
    Modified,
    Size,
    Kind,
}

impl SortBy {
    pub fn from_col(c: usize) -> Self {
        match c {
            1 => SortBy::Modified,
            2 => SortBy::Size,
            3 => SortBy::Kind,
            _ => SortBy::Name,
        }
    }

    pub fn col(self) -> usize {
        match self {
            SortBy::Name => 0,
            SortBy::Modified => 1,
            SortBy::Size => 2,
            SortBy::Kind => 3,
        }
    }
}

/// Polozka vypisu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub e: StatEntry,
    /// Jmeno pro razeni a hledani (mala pismena).
    pub fold: String,
}

impl Item {
    pub fn new(e: StatEntry) -> Self {
        let fold = e.name.to_lowercase();
        Self { e, fold }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.e.kind, kind::DIR | kind::DATASET)
    }

    pub fn is_dataset(&self) -> bool {
        self.e.kind == kind::DATASET
    }

    /// Velikost pro razeni (`None` = neni znama).
    fn size_key(&self) -> Option<u64> {
        let bit = if self.is_dir() { avail::TREE } else { avail::SIZE };
        (format::avail(self.e.known, self.e.hidden, bit) == Avail::Known).then(|| if self.is_dir() { self.e.tree_bytes } else { self.e.size })
    }

    fn mtime_key(&self) -> Option<WallTime> {
        (self.e.known & avail::MTIME != 0).then_some(self.e.mtime)
    }

    /// Druh pro zobrazeni.
    pub fn kind_text(&self) -> &'static str {
        match self.e.kind {
            kind::DATASET => "Dataset s historií",
            kind::DIR => "Složka",
            kind::SYMLINK => "Odkaz",
            _ => crate::preview::type_name(&self.e.name),
        }
    }

    /// Ikona radku (sada `gfx::icon`).
    pub fn icon(&self) -> &'static str {
        match self.e.kind {
            kind::DATASET => "dataset",
            kind::DIR => "folder",
            kind::SYMLINK => "link",
            _ => crate::preview::icon(&self.e.name),
        }
    }
}

/// Text velikosti radku: soubor velikost, slozka souhrn podstromu; chybejici
/// hodnota pomlckou.
pub fn size_text(i: &Item) -> String {
    let bit = if i.is_dir() { avail::TREE } else { avail::SIZE };
    match format::avail(i.e.known, i.e.hidden, bit) {
        Avail::Known if i.is_dir() => format::size(i.e.tree_bytes),
        Avail::Known => format::size(i.e.size),
        _ => String::from(DASH),
    }
}

/// Prirozene poradi: cisla v jmenech podle hodnoty (`2` pred `10`).
pub fn natural(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.as_bytes(), b.as_bytes());
    loop {
        match (x.first(), y.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(p), Some(q)) if p.is_ascii_digit() && q.is_ascii_digit() => {
                let n = x.iter().take_while(|c| c.is_ascii_digit()).count();
                let m = y.iter().take_while(|c| c.is_ascii_digit()).count();
                let (dx, dy) = (&x[..n], &y[..m]);
                // Bez nul na zacatku, pak delka, pak cislice.
                let tx = &dx[dx.iter().take_while(|c| **c == b'0').count().min(n - 1)..];
                let ty = &dy[dy.iter().take_while(|c| **c == b'0').count().min(m - 1)..];
                let o = tx.len().cmp(&ty.len()).then(tx.cmp(ty));
                if o != Ordering::Equal {
                    return o;
                }
                x = &x[n..];
                y = &y[m..];
            }
            (Some(p), Some(q)) => {
                if p != q {
                    return p.cmp(q);
                }
                x = &x[1..];
                y = &y[1..];
            }
        }
    }
}

/// Predpripraveny radek tabulky.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub cells: [String; 4],
    pub icon: &'static str,
}

/// Vypis slozky s razenim a filtrem.
#[derive(Debug, Clone)]
pub struct Listing {
    pub items: Vec<Item>,
    /// Indexy `items` po filtru a razeni.
    pub order: Vec<usize>,
    pub sort: SortBy,
    pub ascending: bool,
    pub filter: String,
    /// Texty viditelnych radku (v poradi `order`), sdilene s tabulkou.
    pub rows: Rc<Vec<Row>>,
}

impl Default for Listing {
    fn default() -> Self {
        Self { items: Vec::new(), order: Vec::new(), sort: SortBy::Name, ascending: true, filter: String::new(), rows: Rc::new(Vec::new()) }
    }
}

impl Listing {
    /// Nove polozky (docasna jmena uloh kopirovani se nezobrazuji).
    pub fn set(&mut self, entries: Vec<StatEntry>, now: Option<WallTime>, zone: Option<&Zone>) {
        self.items = entries.into_iter().filter(|e| !fileops::job::Job::is_temp_name(&e.name)).map(Item::new).collect();
        self.apply(now, zone);
    }

    pub fn set_sort(&mut self, by: SortBy, ascending: bool, now: Option<WallTime>, zone: Option<&Zone>) {
        self.sort = by;
        self.ascending = ascending;
        self.apply(now, zone);
    }

    pub fn set_filter(&mut self, f: &str, now: Option<WallTime>, zone: Option<&Zone>) {
        self.filter = f.trim().to_lowercase();
        self.apply(now, zone);
    }

    /// Filtr, razeni a texty radku.
    pub fn apply(&mut self, now: Option<WallTime>, zone: Option<&Zone>) {
        let items = &self.items;
        let f = &self.filter;
        let mut order: Vec<usize> = (0..items.len()).filter(|&i| f.is_empty() || items[i].fold.contains(f.as_str())).collect();
        let (by, asc) = (self.sort, self.ascending);
        order.sort_by(|&a, &b| {
            let (x, y) = (&items[a], &items[b]);
            // Slozky vzdy prvni (i pri sestupnem razeni).
            let dirs = y.is_dir().cmp(&x.is_dir());
            if dirs != Ordering::Equal {
                return dirs;
            }
            let o = match by {
                SortBy::Name => Ordering::Equal,
                // Neznama hodnota vzdy na konci.
                SortBy::Modified => cmp_opt(x.mtime_key(), y.mtime_key(), asc),
                SortBy::Size => cmp_opt(x.size_key(), y.size_key(), asc),
                SortBy::Kind => {
                    let o = x.kind_text().cmp(y.kind_text());
                    if asc { o } else { o.reverse() }
                }
            };
            let n = natural(&x.fold, &y.fold).then_with(|| x.e.name.cmp(&y.e.name));
            o.then(if by == SortBy::Name && !asc { n.reverse() } else { n })
        });
        let rows = order
            .iter()
            .map(|&i| {
                let it = &items[i];
                let mtime = match format::avail(it.e.known, it.e.hidden, avail::MTIME) {
                    Avail::Known => format::date(it.e.mtime, now, zone),
                    _ => String::from(DASH),
                };
                Row { id: it.e.oid, cells: [it.e.name.clone(), mtime, size_text(it), String::from(it.kind_text())], icon: it.icon() }
            })
            .collect();
        self.order = order;
        self.rows = Rc::new(rows);
    }

    /// Polozka podle id radku (oid).
    pub fn by_id(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.e.oid == id)
    }

    /// Viditelna polozka `i` (v poradi tabulky).
    pub fn visible(&self, i: usize) -> Option<&Item> {
        self.order.get(i).map(|&k| &self.items[k])
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

fn cmp_opt<T: Ord>(a: Option<T>, b: Option<T>, asc: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if asc {
                a.cmp(&b)
            } else {
                b.cmp(&a)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}
