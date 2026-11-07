//! Undo: "vrat posledni prikaz" nad pohledy FS.
//!
//! Prikaz, ktery nic nezmenil, nestoji nic. Kdyz zmenil, shell pred
//! prevzetim konceptu radku vytvori pohled (`VIEW`: levny docasny snapshot
//! datasetu a pozice v zurnalu) kazdeho datasetu se zmenami. `undo` vrati
//! zmenene cesty do stavu z pohledu.
//!
//! Programy mimo shell meni FS samy: pred jejich spustenim se vytvori
//! pohledy datasetu, kam smi zapisovat. Po skonceni zurnal rekne, jestli
//! se neco zmenilo (bez pruchodu stromem); jen kdyz ano, FS vrati seznam
//! zmenenych cest (`DIFF`). Nic nezmeneno = zadny zaznam (undo vrati
//! predchozi prikaz).

use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::diff::{ChangeKind, DiffEntry};
use super::draft::Draft;
use super::path::{join, key, within};
use crate::host::{Fs, FsChangeKind, FsResult, NodeKind, ViewRef};

#[derive(Debug, Clone, PartialEq)]
pub struct UndoEntry {
    pub id: u64,
    /// Prikaz, ktery zmeny udelal.
    pub line: String,
    /// Pohledy datasetu pred zmenou.
    pub views: Vec<ViewRef>,
    /// Program: slozky, kam smel zapisovat (zmeny mimo ne nejsou jeho).
    pub trees: Vec<String>,
    pub diff: Vec<DiffEntry>,
}

/// Historie vratitelnych prikazu.
#[derive(Debug, Default)]
pub struct UndoLog {
    pub entries: Vec<UndoEntry>,
    next: u64,
    /// Kolik prikazu jde vratit (starsi pohledy se uvolni).
    pub keep: usize,
    /// Zaznam programu, ktery prave bezi (doplni ho `after_external`).
    pending: Option<UndoEntry>,
}

/// Vysledek prevzeti konceptu.
#[derive(Debug, Default)]
pub struct Committed {
    pub diff: Vec<DiffEntry>,
    /// Cesty, ktere se neprevzaly (puvod se mezitim zmenil jinak).
    pub conflicts: Vec<String>,
    /// FS neumi pohledy: zmeny jsou zapsane, ale nepujdou vratit.
    pub no_undo: bool,
}

fn release(fs: &mut dyn Fs, views: &[ViewRef]) {
    for v in views {
        let _ = fs.release_view(v);
    }
}

/// Pohledy na datasety (bez duplicit). `None` = FS pohledy neumi.
fn views_of(fs: &mut dyn Fs, datasets: &[String]) -> Option<Vec<ViewRef>> {
    let mut out: Vec<ViewRef> = Vec::new();
    for ds in datasets {
        if out.iter().any(|v| key(&v.dataset) == key(ds)) {
            continue;
        }
        match fs.view(ds) {
            Ok(v) => out.push(v),
            Err(_) => {
                release(fs, &out);
                return None;
            }
        }
    }
    Some(out)
}

/// Pohled datasetu, ve kterem `path` lezi (nejdelsi koren).
fn view_for<'a>(views: &'a [ViewRef], path: &str) -> Option<&'a ViewRef> {
    views.iter().filter(|v| within(path, &v.dataset)).max_by_key(|v| v.dataset.len())
}

impl UndoLog {
    pub fn new() -> Self {
        Self { entries: Vec::new(), next: 1, keep: 16, pending: None }
    }

    fn push(&mut self, fs: &mut dyn Fs, e: UndoEntry) {
        self.entries.push(e);
        while self.entries.len() > self.keep {
            let old = self.entries.remove(0);
            release(fs, &old.views);
        }
    }

    fn entry(&mut self, line: &str, views: Vec<ViewRef>, trees: Vec<String>, diff: Vec<DiffEntry>) -> UndoEntry {
        let id = self.next;
        self.next += 1;
        UndoEntry { id, line: line.into(), views, trees, diff }
    }

    /// Prevezme koncept radku (`diff` uz spocitany) do FS a zapamatuje si,
    /// jak ho vratit. Prazdny diff = klony se jen zahodi.
    pub fn commit(&mut self, fs: &mut dyn Fs, draft: &mut Draft, diff: Vec<DiffEntry>, line: &str)
        -> FsResult<Committed> {
        if diff.is_empty() {
            draft.discard_top(fs);
            return Ok(Committed::default());
        }
        let views = views_of(fs, &draft.datasets());
        let conflicts = match draft.apply_top(fs) {
            Ok(c) => c,
            Err(e) => {
                release(fs, views.as_deref().unwrap_or(&[]));
                return Err(e);
            }
        };
        // Vratit jde jen to, co se opravdu prevzalo.
        let applied: Vec<DiffEntry> =
            diff.iter().filter(|d| !d.paths().any(|p| conflicts.iter().any(|c| key(c) == key(p)))).cloned().collect();
        let no_undo = views.is_none();
        match views {
            Some(v) if !applied.is_empty() => {
                let e = self.entry(line, v, Vec::new(), applied);
                self.push(fs, e);
            }
            Some(v) => release(fs, &v),
            None => {}
        }
        Ok(Committed { diff, conflicts, no_undo })
    }

    /// Pred spustenim programu, ktery smi menit `datasets`/`trees`:
    /// pohledy. `false` = FS pohledy neumi (undo programu nepujde).
    pub fn before_external(&mut self, fs: &mut dyn Fs, line: &str, datasets: &[String], trees: &[String]) -> bool {
        if trees.is_empty() {
            return true;
        }
        let Some(views) = views_of(fs, datasets) else { return false };
        let e = self.entry(line, views, trees.into(), Vec::new());
        self.pending = Some(e);
        true
    }

    /// Po skonceni programu: co se zmenilo v jeho slozkach. Nic = zaznam
    /// se zahodi (undo vrati predchozi prikaz).
    pub fn after_external(&mut self, fs: &mut dyn Fs) -> Vec<DiffEntry> {
        let Some(mut e) = self.pending.take() else { return Vec::new() };
        let mut diff = Vec::new();
        for v in e.views.clone() {
            // Neznamy stav (chyba) = radsi zaznam ponechat s celou slozkou.
            let changes = match fs.changed_since(&v) {
                Ok(c) => c,
                Err(_) => {
                    diff.push(DiffEntry { path: v.dataset.clone(), change: ChangeKind::Modified, from: None, before: None, after: None });
                    continue;
                }
            };
            for c in changes {
                if !e.trees.iter().any(|t| within(&c.path, t) || c.old_path.as_deref().is_some_and(|o| within(o, t))) {
                    continue;
                }
                let before = fs.view_stat(&v, c.old_path.as_deref().unwrap_or(&c.path)).ok();
                let after = fs.stat(&c.path).ok();
                let size = |m: Option<crate::host::Meta>| m.filter(|m| m.kind != NodeKind::Dir).map(|m| m.size);
                let change = match c.kind {
                    FsChangeKind::Added if c.is_dir => ChangeKind::DirAdded,
                    FsChangeKind::Added => ChangeKind::Added,
                    FsChangeKind::Removed if c.is_dir => ChangeKind::DirRemoved,
                    FsChangeKind::Removed => ChangeKind::Removed,
                    FsChangeKind::Modified => ChangeKind::Modified,
                    FsChangeKind::Moved => ChangeKind::Moved,
                };
                diff.push(DiffEntry { path: c.path, change, from: c.old_path, before: size(before), after: size(after) });
            }
        }
        if diff.is_empty() {
            release(fs, &e.views);
            return diff;
        }
        e.diff = super::diff::normalize(diff);
        let out = e.diff.clone();
        self.push(fs, e);
        out
    }

    /// Vrati posledni prikaz a vrati jeho zaznam. Kdyz vraceni selze,
    /// zaznam zustane (jde to zkusit znovu) a pohledy se neuvolni.
    pub fn undo(&mut self, fs: &mut dyn Fs) -> Result<UndoEntry, Error> {
        let e = self.entries.last().cloned().ok_or(Error::NotFound)?;
        Self::restore(fs, &e)?;
        self.entries.pop();
        release(fs, &e.views);
        Ok(e)
    }

    /// Kazdou zmenenou cestu srovna se stavem v pohledu: od korene (nejdriv
    /// slozky, pak jejich obsah); co v pohledu neni, smaze.
    fn restore(fs: &mut dyn Fs, e: &UndoEntry) -> Result<(), Error> {
        let mut paths: Vec<&String> = e.diff.iter().flat_map(|d| d.paths()).collect();
        paths.sort_by_key(|p| (p.matches('/').count(), key(p)));
        paths.dedup_by(|a, b| key(a) == key(b));
        for p in paths {
            let v = view_for(&e.views, p).ok_or(Error::NotFound)?;
            sync_from_view(fs, v, p)?;
        }
        Ok(())
    }
}

/// Smaze soubor nebo slozku i s obsahem.
pub fn remove_all(fs: &mut dyn Fs, path: &str) -> FsResult<()> {
    let m = fs.stat(path)?;
    if m.kind == NodeKind::Dir {
        for e in fs.list_names(path)? {
            remove_all(fs, &join(path, &e.name))?;
        }
    }
    fs.remove(path)
}

/// Srovna `path` se stavem v pohledu `v` (i s obsahem slozky).
pub fn sync_from_view(fs: &mut dyn Fs, v: &ViewRef, path: &str) -> FsResult<()> {
    let old = fs.view_stat(v, path);
    let cur = fs.stat(path);
    match (old, cur) {
        (Err(_), Ok(_)) => remove_all(fs, path),
        (Err(_), Err(_)) => Ok(()),
        (Ok(o), cur) if o.kind == NodeKind::Dir => {
            match cur {
                Ok(c) if c.kind == NodeKind::Dir => {}
                Ok(_) => {
                    fs.remove(path)?;
                    fs.mkdir(path)?;
                }
                Err(_) => fs.mkdir(path)?,
            }
            let mut names: Vec<String> = fs.view_list(v, path)?.into_iter().map(|e| e.name).collect();
            for e in fs.list_names(path)? {
                if !names.iter().any(|n| n.eq_ignore_ascii_case(&e.name)) {
                    names.push(e.name);
                }
            }
            for n in names {
                sync_from_view(fs, v, &join(path, &n))?;
            }
            Ok(())
        }
        (Ok(_), cur) => {
            let data = fs.view_read(v, path)?;
            match cur {
                Ok(c) if c.kind == NodeKind::Dir => remove_all(fs, path)?,
                Ok(_) if fs.read(path).ok().as_deref() == Some(data.as_slice()) => return Ok(()),
                _ => {}
            }
            fs.write(path, &data)
        }
    }
}
