//! MemFs: datasety, klony bokem, pohledy a zurnal - napodoba FS v3 pro
//! testy na hostiteli.
//!
//! - Dataset = koren s vlastnim zurnalem (pocitadlo zmen). "/" je vzdy.
//! - Klon bokem lezi v `/tmp/.aside/N` a je sam datasetem. Vnorene
//!   datasety puvodu jsou v klonu prazdne slozky - hranice: zapis do nich
//!   selze `NOT_SUPPORTED`.
//! - Prevzeti klonu je trojcestne: co se v puvodu mezitim zmenilo jinak,
//!   je konflikt a neprevezme se.
//! - Pohled = kopie celeho stromu + pozice zurnalu.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::mem::{MemFs, Node};
use super::path::{key, parent, rebase, within};
use crate::host::{Aside, FsChange, FsChangeKind, FsResult, ViewRef};

pub(super) type Nodes = BTreeMap<String, (String, Node)>;

#[derive(Debug, Clone)]
pub(super) struct AsideRec {
    dataset: String,
    /// Stav puvodu pri zalozeni (relativni cesty).
    base: Nodes,
    /// Hranice v klonu (absolutni klice).
    boundaries: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Hist {
    /// Koreny datasetu krome "/" (klice), vcetne klonu.
    datasets: Vec<String>,
    asides: BTreeMap<String, AsideRec>,
    pub(super) views: BTreeMap<String, Nodes>,
    journal: BTreeMap<String, u64>,
    next: u64,
    /// Kolik klonu vzniklo (testy: radek, ktery jen cte, klon nezaklada).
    clones_made: usize,
}

/// Uzly datasetu `ds` s relativnimi cestami; obsah vnorenych datasetu se
/// vynecha (jejich koren zustane jako slozka).
fn rel_nodes(nodes: &Nodes, datasets: &[String], ds: &str) -> Nodes {
    let nested: Vec<&String> = datasets.iter().filter(|d| within(d, ds) && key(d) != key(ds)).collect();
    let mut out = Nodes::new();
    for (k, (orig, n)) in nodes {
        if !within(k, ds) || nested.iter().any(|d| within(k, d) && key(k) != key(d)) {
            continue;
        }
        let node = if nested.iter().any(|d| key(d) == *k) { Node::Dir } else { n.clone() };
        let rel = rebase(orig, ds, "/");
        out.insert(key(&rel), (rel, node));
    }
    out
}

/// Rozdil dvou stavu datasetu (relativni uzly) -> zmeny s cestami v `ds`.
fn compare(old: &Nodes, new: &Nodes, ds: &str) -> Vec<FsChange> {
    let mut keys: Vec<&String> = old.keys().chain(new.keys()).collect();
    keys.sort();
    keys.dedup();
    let mut out = Vec::new();
    for k in keys {
        let (o, n) = (old.get(k), new.get(k));
        let orig = n.or(o).map(|(p, _)| p.as_str()).unwrap_or("/");
        let path = rebase(orig, "/", ds);
        let ch = |kind, is_dir| FsChange { kind, path: path.clone(), old_path: None, is_dir };
        match (o.map(|x| &x.1), n.map(|x| &x.1)) {
            (Some(a), None) => out.push(ch(FsChangeKind::Removed, *a == Node::Dir)),
            (None, Some(b)) => out.push(ch(FsChangeKind::Added, *b == Node::Dir)),
            (Some(Node::File(a)), Some(Node::File(b))) if a != b => out.push(ch(FsChangeKind::Modified, false)),
            (Some(a), Some(b)) if (*a == Node::Dir) != (*b == Node::Dir) => {
                out.push(ch(FsChangeKind::Removed, *a == Node::Dir));
                out.push(ch(FsChangeKind::Added, *b == Node::Dir));
            }
            _ => {}
        }
    }
    out
}

impl MemFs {
    /// Pomocnik pro testy: slozka `path` jako samostatny dataset.
    pub fn add_dataset(&mut self, path: &str) {
        if !self.is_dir(path) {
            self.put(&alloc::format!("{path}/.keep"), b"");
        }
        self.hist.datasets.push(key(path));
    }

    /// Pocet klonu bokem, ktere prave existuji.
    pub fn asides_open(&self) -> usize {
        self.hist.asides.len()
    }

    pub fn clones_made(&self) -> usize {
        self.hist.clones_made
    }

    pub fn views_open(&self) -> usize {
        self.hist.views.len()
    }

    pub(super) fn dataset_root(&self, path: &str) -> String {
        let best = self.hist.datasets.iter().filter(|d| within(path, d)).max_by_key(|d| d.len());
        match best {
            Some(k) => self.nodes.get(k).map(|(o, _)| o.clone()).unwrap_or_else(|| k.clone()),
            None => String::from("/"),
        }
    }

    /// Zapis do hranice klonu (vnoreny dataset) nejde.
    pub(super) fn guard(&self, path: &str) -> FsResult<()> {
        let hit = self.hist.asides.values().any(|a| a.boundaries.iter().any(|b| within(path, b)));
        if hit { Err(Error::NotSupported) } else { Ok(()) }
    }

    /// Cteni uvnitr hranice klonu (ne hranice sama) = `NOT_SUPPORTED`, jako
    /// pruchod pres polozku datasetu ve fsd.
    pub(super) fn guard_inside(&self, path: &str) -> FsResult<()> {
        let hit = self
            .hist
            .asides
            .values()
            .any(|a| a.boundaries.iter().any(|b| within(path, b) && key(path) != *b));
        if hit { Err(Error::NotSupported) } else { Ok(()) }
    }

    /// Pohled vidi jen svuj dataset: cesta uvnitr vnoreneho datasetu (nebo
    /// vypis jeho korene) = `NOT_SUPPORTED`, jako snapshot datasetu ve fsd.
    pub(super) fn view_guard(&self, v: &ViewRef, path: &str, list: bool) -> FsResult<()> {
        let nested = self
            .hist
            .datasets
            .iter()
            .filter(|d| within(d, &v.dataset) && key(d) != key(&v.dataset))
            .any(|d| within(path, d) && (list || key(path) != *d));
        if nested { Err(Error::NotSupported) } else { Ok(()) }
    }

    /// Zaznam do zurnalu datasetu, ve kterem `path` lezi.
    pub(super) fn bump(&mut self, path: &str) {
        let ds = key(&self.dataset_root(path));
        *self.hist.journal.entry(ds).or_insert(0) += 1;
    }

    fn insert_dir(&mut self, path: &str) {
        if let alloc::collections::btree_map::Entry::Vacant(e) = self.nodes.entry(key(path)) {
            e.insert((path.into(), Node::Dir));
            self.bump(path);
        }
    }

    pub(super) fn h_aside_begin(&mut self, dataset: &str) -> FsResult<Aside> {
        if key(&self.dataset_root(dataset)) != key(dataset) {
            return Err(Error::InvalidArgs);
        }
        self.insert_dir("/tmp");
        self.insert_dir("/tmp/.aside");
        self.hist.next += 1;
        self.hist.clones_made += 1;
        let root = alloc::format!("/tmp/.aside/{}", self.hist.next);
        let base = rel_nodes(&self.nodes, &self.hist.datasets, dataset);
        let boundaries: Vec<String> = self
            .hist
            .datasets
            .iter()
            .filter(|d| within(d, dataset) && key(d) != key(dataset))
            .map(|d| key(&rebase(d, dataset, &root)))
            .collect();
        for (rel, n) in base.values() {
            let abs = rebase(rel, "/", &root);
            self.nodes.insert(key(&abs), (abs, n.clone()));
        }
        self.hist.datasets.push(key(&root));
        self.hist.asides.insert(key(&root), AsideRec { dataset: dataset.into(), base, boundaries });
        Ok(Aside { dataset: dataset.into(), root })
    }

    fn rec(&self, a: &Aside) -> FsResult<AsideRec> {
        self.hist.asides.get(&key(&a.root)).cloned().ok_or(Error::NotFound)
    }

    pub(super) fn h_aside_changes(&mut self, a: &Aside) -> FsResult<Vec<FsChange>> {
        let rec = self.rec(a)?;
        let cur = rel_nodes(&self.nodes, &self.hist.datasets, &a.root);
        Ok(compare(&rec.base, &cur, &rec.dataset))
    }

    pub(super) fn h_aside_apply(&mut self, a: &Aside) -> FsResult<Vec<String>> {
        let rec = self.rec(a)?;
        let mut changes = self.h_aside_changes(a)?;
        let mut conflicts = Vec::new();
        // Stav puvodu pred prevzetim (nahrazeni = smazani + pridani na
        // stejne ceste se porovnava se stavem pred prvnim krokem).
        let start = self.nodes.clone();
        // Mazani od listu, pridani od korene.
        changes.sort_by_key(|c| {
            let depth = c.path.matches('/').count() as i64;
            if c.kind == FsChangeKind::Removed { (0, -depth) } else { (1, depth) }
        });
        for c in changes {
            let rel = key(&rebase(&c.path, &rec.dataset, "/"));
            let base = rec.base.get(&rel).map(|x| x.1.clone());
            let theirs = self.nodes.get(&key(&rebase(&c.path, &rec.dataset, &a.root))).map(|x| x.1.clone());
            let origin = start.get(&key(&c.path)).map(|x| x.1.clone());
            if origin != base && origin != theirs {
                conflicts.push(c.path);
                continue;
            }
            match (c.kind, theirs) {
                (FsChangeKind::Removed, _) => {
                    let k = key(&c.path);
                    let busy = self.nodes.keys().any(|n| parent(n) == Some(k.as_str()));
                    if busy {
                        conflicts.push(c.path);
                    } else {
                        self.nodes.remove(&k);
                    }
                }
                (_, Some(n)) => {
                    self.nodes.insert(key(&c.path), (c.path.clone(), n));
                }
                (_, None) => {}
            }
        }
        self.bump(&rec.dataset);
        Ok(conflicts)
    }

    pub(super) fn h_aside_discard(&mut self, a: &Aside) -> FsResult<()> {
        self.hist.asides.remove(&key(&a.root)).ok_or(Error::NotFound)?;
        let rk = key(&a.root);
        self.nodes.retain(|k, _| !within(k, &rk));
        self.hist.datasets.retain(|d| *d != rk);
        self.hist.journal.remove(&rk);
        Ok(())
    }

    pub(super) fn h_view(&mut self, dataset: &str) -> FsResult<ViewRef> {
        self.hist.next += 1;
        let snapshot = alloc::format!("view-{}", self.hist.next);
        self.hist.views.insert(snapshot.clone(), self.nodes.clone());
        let seq = self.hist.journal.get(&key(dataset)).copied().unwrap_or(0);
        Ok(ViewRef { dataset: dataset.into(), snapshot, seq })
    }

    pub(super) fn h_changed_since(&mut self, v: &ViewRef) -> FsResult<Vec<FsChange>> {
        let seq = self.hist.journal.get(&key(&v.dataset)).copied().unwrap_or(0);
        if seq == v.seq {
            return Ok(Vec::new());
        }
        let snap = self.hist.views.get(&v.snapshot).ok_or(Error::NotFound)?;
        let old = rel_nodes(snap, &self.hist.datasets, &v.dataset);
        let new = rel_nodes(&self.nodes, &self.hist.datasets, &v.dataset);
        Ok(compare(&old, &new, &v.dataset))
    }
}
