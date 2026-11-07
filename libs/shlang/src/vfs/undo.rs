//! Liny undo: snapshot az pri prvni zmene, "vrat posledni prikaz".
//!
//! Prikaz, ktery nic nezmenil, nestoji nic. Kdyz zmenil, shell pred
//! prevzetim zmen vytvori snapshot FS (`shell-undo-N`; na FS v3 je to
//! levny CoW snapshot) a zapamatuje si, ktere cesty se meni. `undo` vrati
//! tyto cesty do stavu ze snapshotu. FS bez snapshotu: shell si puvodni
//! obsah zmenenych souboru drzi v pameti.
//!
//! Programy mimo shell meni FS samy; pro ne se pamatuji cele slozky, kam
//! smi zapisovat (z manifestu), a `undo` je srovna se snapshotem.

use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::overlay::{DiffEntry, Overlay};
use super::path::join;
use crate::host::{Fs, FsResult, NodeKind};

#[derive(Debug, Clone, PartialEq)]
pub enum Prior {
    /// Cesta predtim neexistovala.
    Missing,
    Dir,
    /// Soubor; `None` = obsah je ve snapshotu.
    File(Option<Vec<u8>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UndoEntry {
    pub id: u64,
    /// Prikaz, ktery zmeny udelal.
    pub line: String,
    pub snapshot: Option<String>,
    pub prior: Vec<(String, Prior)>,
    /// Slozky zmenene programy mimo shell (srovnaji se se snapshotem).
    pub trees: Vec<String>,
    pub diff: Vec<DiffEntry>,
}

/// Historie vratitelnych prikazu.
#[derive(Debug, Default)]
pub struct UndoLog {
    pub entries: Vec<UndoEntry>,
    next: u64,
    /// Kolik prikazu jde vratit (starsi snapshoty se mazou).
    pub keep: usize,
}

fn snap_name(id: u64) -> String {
    alloc::format!("shell-undo-{id}")
}

impl UndoLog {
    pub fn new() -> Self {
        Self { entries: Vec::new(), next: 1, keep: 16 }
    }

    fn start(&mut self, base: &mut dyn Fs) -> (u64, Option<String>) {
        let id = self.next;
        self.next += 1;
        let name = snap_name(id);
        let snap = base.snapshot(&name).is_ok().then_some(name);
        (id, snap)
    }

    fn push(&mut self, base: &mut dyn Fs, e: UndoEntry) {
        self.entries.push(e);
        while self.entries.len() > self.keep {
            let old = self.entries.remove(0);
            if let Some(s) = old.snapshot {
                let _ = base.delete_snapshot(&s);
            }
        }
    }

    /// Prevezme zmeny prekryvu do FS a zapamatuje si, jak je vratit.
    /// Vrati diff (prazdny = nic se nezmenilo, snapshot se nedela).
    pub fn commit(&mut self, base: &mut dyn Fs, overlay: &Overlay, line: &str) -> FsResult<Vec<DiffEntry>> {
        let diff = overlay.changes(base)?;
        self.commit_diff(base, overlay, diff, line)
    }

    /// Jako [`commit`](Self::commit) s uz spocitanym diffem prekryvu.
    pub fn commit_diff(&mut self, base: &mut dyn Fs, overlay: &Overlay, diff: Vec<DiffEntry>, line: &str)
        -> FsResult<Vec<DiffEntry>> {
        if diff.is_empty() {
            return Ok(diff);
        }
        let (id, snapshot) = self.start(base);
        // Puvodni stav podle FS (ne podle druhu zmeny): u nahrazeni souboru
        // slozkou se tak pamatuje soubor a naopak.
        let mut prior = Vec::new();
        for d in &diff {
            let p = match base.stat(&d.path) {
                Err(_) => Prior::Missing,
                Ok(m) if m.kind == NodeKind::Dir => Prior::Dir,
                Ok(_) => Prior::File(if snapshot.is_some() { None } else { Some(base.read(&d.path)?) }),
            };
            prior.push((d.path.clone(), p));
        }
        let entry = UndoEntry { id, line: line.into(), snapshot, prior, trees: Vec::new(), diff: diff.clone() };
        let r = overlay.commit(base);
        // I po castecnem selhani jde vratit, co se zmenit stihlo.
        self.push(base, entry);
        r.map(|_| diff)
    }

    /// Pred spustenim programu, ktery smi menit `trees`: snapshot (bez nej
    /// undo programu neumi - vrati `false`).
    pub fn before_external(&mut self, base: &mut dyn Fs, line: &str, trees: &[String]) -> bool {
        if trees.is_empty() {
            return true;
        }
        let (id, snapshot) = self.start(base);
        if snapshot.is_none() {
            return false;
        }
        let entry = UndoEntry {
            id,
            line: line.into(),
            snapshot,
            prior: Vec::new(),
            trees: trees.into(),
            diff: Vec::new(),
        };
        self.push(base, entry);
        true
    }

    /// Po skonceni programu: kdyz se jeho slozky proti snapshotu nezmenily,
    /// zaznam undo i snapshot zahodi.
    pub fn after_external(&mut self, base: &mut dyn Fs) {
        let Some(e) = self.entries.last() else { return };
        let (Some(snap), false) = (e.snapshot.clone(), e.trees.is_empty()) else { return };
        let changed = e.trees.clone().iter().any(|t| tree_differs(base, &snap, t).unwrap_or(true));
        if !changed {
            self.entries.pop();
            let _ = base.delete_snapshot(&snap);
        }
    }

    /// Vrati posledni prikaz a vrati jeho zaznam. Kdyz vraceni selze,
    /// zaznam zustane (jde to zkusit znovu) a snapshot se nesmaze.
    pub fn undo(&mut self, base: &mut dyn Fs) -> Result<UndoEntry, Error> {
        let e = self.entries.last().cloned().ok_or(Error::NotFound)?;
        Self::restore(base, &e)?;
        self.entries.pop();
        if let Some(s) = &e.snapshot {
            let _ = base.delete_snapshot(s);
        }
        Ok(e)
    }

    fn restore(base: &mut dyn Fs, e: &UndoEntry) -> Result<(), Error> {
        let snap = e.snapshot.as_deref();
        // 1. smazane slozky zpet (od korene); soubor na jejich miste pryc
        let mut dirs: Vec<&String> = e.prior.iter().filter(|(_, p)| *p == Prior::Dir).map(|(p, _)| p).collect();
        dirs.sort_by_key(|p| p.len());
        for d in dirs {
            match base.stat(d) {
                Ok(m) if m.kind == NodeKind::Dir => continue,
                Ok(_) => base.remove(d)?,
                Err(_) => {}
            }
            base.mkdir(d)?;
        }
        // 2. puvodni obsah souboru (slozka na jejich miste pryc)
        for (path, p) in &e.prior {
            if let Prior::File(data) = p {
                let content = match data {
                    Some(d) => d.clone(),
                    None => base.snapshot_read(snap.ok_or(Error::NotFound)?, path)?,
                };
                if matches!(base.stat(path), Ok(m) if m.kind == NodeKind::Dir) {
                    remove_all(base, path)?;
                }
                base.write(path, &content)?;
            }
        }
        // 3. nove vytvorene veci pryc (od listu)
        let mut created: Vec<&String> = e.prior.iter().filter(|(_, p)| *p == Prior::Missing).map(|(p, _)| p).collect();
        created.sort_by_key(|p| core::cmp::Reverse(p.len()));
        for c in created {
            match remove_all(base, c) {
                Ok(()) | Err(Error::NotFound) => {}
                Err(err) => return Err(err),
            }
        }
        // 4. slozky zmenene programy
        if let Some(s) = snap {
            for t in &e.trees {
                sync_tree(base, s, t)?;
            }
        }
        Ok(())
    }
}

/// Smaze soubor nebo slozku i s obsahem.
pub fn remove_all(base: &mut dyn Fs, path: &str) -> FsResult<()> {
    let m = base.stat(path)?;
    if m.kind == NodeKind::Dir {
        for e in base.list(path)? {
            remove_all(base, &join(path, &e.name))?;
        }
    }
    base.remove(path)
}

/// Lisi se `path` od stavu ve snapshotu `snap`? Porovnava druh, velikost
/// a cas zmeny; obsah cte jen tam, kde FS cas nezna.
pub fn tree_differs(base: &mut dyn Fs, snap: &str, path: &str) -> FsResult<bool> {
    let (old, cur) = match (base.snapshot_stat(snap, path), base.stat(path)) {
        (Err(_), Err(_)) => return Ok(false),
        (Ok(o), Ok(c)) => (o, c),
        _ => return Ok(true),
    };
    if old.kind != cur.kind {
        return Ok(true);
    }
    if old.kind == NodeKind::Dir {
        let mut a: Vec<String> = base.snapshot_list(snap, path)?.into_iter().map(|e| e.name.to_lowercase()).collect();
        let mut b: Vec<String> = base.list(path)?.into_iter().map(|e| e.name.to_lowercase()).collect();
        a.sort();
        b.sort();
        if a != b {
            return Ok(true);
        }
        for n in a {
            if tree_differs(base, snap, &join(path, &n))? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    if old.size != cur.size {
        return Ok(true);
    }
    match (old.mtime, cur.mtime) {
        (Some(a), Some(b)) => Ok(a != b),
        _ => Ok(base.snapshot_read(snap, path)? != base.read(path)?),
    }
}

/// Srovna `path` se stavem ve snapshotu `snap`.
pub fn sync_tree(base: &mut dyn Fs, snap: &str, path: &str) -> FsResult<()> {
    let old = base.snapshot_stat(snap, path);
    let cur = base.stat(path);
    match (old, cur) {
        (Err(_), Ok(_)) => remove_all(base, path),
        (Err(_), Err(_)) => Ok(()),
        (Ok(o), cur) if o.kind == NodeKind::Dir => {
            match cur {
                Ok(c) if c.kind == NodeKind::Dir => {}
                Ok(_) => {
                    base.remove(path)?;
                    base.mkdir(path)?;
                }
                Err(_) => base.mkdir(path)?,
            }
            let mut names: Vec<String> = base.snapshot_list(snap, path)?.into_iter().map(|e| e.name).collect();
            for e in base.list(path)? {
                if !names.iter().any(|n| n.eq_ignore_ascii_case(&e.name)) {
                    names.push(e.name);
                }
            }
            for n in names {
                sync_tree(base, snap, &join(path, &n))?;
            }
            Ok(())
        }
        (Ok(_), cur) => {
            let data = base.snapshot_read(snap, path)?;
            match cur {
                Ok(c) if c.kind == NodeKind::Dir => remove_all(base, path)?,
                Ok(_) if base.read(path).ok().as_deref() == Some(data.as_slice()) => return Ok(()),
                _ => {}
            }
            base.write(path, &data)
        }
    }
}
