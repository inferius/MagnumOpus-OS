//! Zmeny nanecisto nad souborovym systemem.
//!
//! Kazdy prikaz zapisuje nejdriv do prekryvu (cteni vidi zmeny prekryvu
//! i FS pod nim). Na konci prikazu shell spocita diff: male zmeny prevezme
//! hned (s linym undo), velke (zamek nebezpecnych operaci) a `preview { }`
//! ukaze a prevezme az po potvrzeni.
//!
//! Omezeni: prekryv drzi zapsana data v pameti az do prevzeti. Programy
//! mimo shell (s vlastni relaci fsd) do prekryvu nevidi - shell je pred
//! jejich spustenim prevezme a v `preview` je odmitne.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::path::{join, key, parent};
use crate::host::{DirEntry, Fs, FsResult, Meta, NodeKind};
use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    File(Vec<u8>),
    Dir,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Removed,
    DirAdded,
    DirRemoved,
    /// Soubor nahrazen slozkou nebo naopak.
    Replaced,
}

impl ChangeKind {
    pub fn name(self) -> &'static str {
        match self {
            ChangeKind::Added => "added",
            ChangeKind::Modified => "modified",
            ChangeKind::Removed => "removed",
            ChangeKind::DirAdded => "dir-added",
            ChangeKind::DirRemoved => "dir-removed",
            ChangeKind::Replaced => "replaced",
        }
    }
}

/// Jedna polozka diffu.
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    pub path: String,
    pub change: ChangeKind,
    pub before: Option<u64>,
    pub after: Option<u64>,
}

/// Diff jako tabulka (`{change, path, before, after}`).
pub fn diff_value(diff: &[DiffEntry]) -> Value {
    Value::List(
        diff.iter()
            .map(|d| {
                Value::record(&[
                    ("change", Value::str(d.change.name())),
                    ("path", Value::str(d.path.clone())),
                    ("before", d.before.map(|b| Value::Size(b as i64)).unwrap_or(Value::Null)),
                    ("after", d.after.map(|b| Value::Size(b as i64)).unwrap_or(Value::Null)),
                ])
            })
            .collect(),
    )
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    map: BTreeMap<String, (String, Change)>,
}

impl Overlay {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn own(&self, path: &str) -> Option<&Change> {
        self.map.get(&key(path)).map(|(_, c)| c)
    }

    /// Je nektera nadrazena slozka v prekryvu smazana nebo prepsana souborem?
    fn hidden(&self, path: &str) -> bool {
        let mut p = parent(path);
        while let Some(d) = p {
            if matches!(self.own(d), Some(Change::Removed | Change::File(_))) {
                return true;
            }
            p = parent(d);
        }
        false
    }

    pub fn stat(&self, base: &mut dyn Fs, path: &str) -> FsResult<Meta> {
        match self.own(path) {
            Some(Change::File(d)) => return Ok(Meta { kind: NodeKind::File, size: d.len() as u64, mtime: None }),
            Some(Change::Dir) => return Ok(Meta { kind: NodeKind::Dir, size: 0, mtime: None }),
            Some(Change::Removed) => return Err(Error::NotFound),
            None => {}
        }
        if self.hidden(path) {
            return Err(Error::NotFound);
        }
        base.stat(path)
    }

    pub fn list(&self, base: &mut dyn Fs, path: &str) -> FsResult<Vec<DirEntry>> {
        let m = self.stat(base, path)?;
        if m.kind != NodeKind::Dir {
            return Err(Error::NotADirectory);
        }
        let mut out: BTreeMap<String, DirEntry> = BTreeMap::new();
        let in_base = self.own(path).is_none() || matches!(base.stat(path), Ok(m) if m.kind == NodeKind::Dir);
        if in_base && !self.hidden(path) {
            for e in base.list(path).unwrap_or_default() {
                out.insert(key(&e.name), e);
            }
        }
        let k = key(path);
        for (ck, (orig, ch)) in &self.map {
            if parent(ck) != Some(k.as_str()) || ck == "/" {
                continue;
            }
            let name: String = super::path::file_name(orig).into();
            match ch {
                Change::File(d) => {
                    out.insert(
                        key(&name),
                        DirEntry { name, meta: Meta { kind: NodeKind::File, size: d.len() as u64, mtime: None } },
                    );
                }
                Change::Dir => {
                    out.insert(key(&name), DirEntry { name, meta: Meta { kind: NodeKind::Dir, size: 0, mtime: None } });
                }
                Change::Removed => {
                    out.remove(&key(&name));
                }
            }
        }
        Ok(out.into_values().collect())
    }

    pub fn read(&self, base: &mut dyn Fs, path: &str) -> FsResult<Vec<u8>> {
        match self.own(path) {
            Some(Change::File(d)) => Ok(d.clone()),
            Some(Change::Dir) => Err(Error::IsADirectory),
            Some(Change::Removed) => Err(Error::NotFound),
            None if self.hidden(path) => Err(Error::NotFound),
            None => base.read(path),
        }
    }

    fn parent_is_dir(&self, base: &mut dyn Fs, path: &str) -> FsResult<()> {
        let p = parent(path).ok_or(Error::InvalidArgs)?;
        match self.stat(base, p) {
            Ok(m) if m.kind == NodeKind::Dir => Ok(()),
            Ok(_) => Err(Error::NotADirectory),
            Err(e) => Err(e),
        }
    }

    pub fn write(&mut self, base: &mut dyn Fs, path: &str, data: Vec<u8>) -> FsResult<()> {
        self.parent_is_dir(base, path)?;
        if let Ok(m) = self.stat(base, path)
            && m.kind == NodeKind::Dir
        {
            return Err(Error::IsADirectory);
        }
        self.map.insert(key(path), (path.into(), Change::File(data)));
        Ok(())
    }

    pub fn mkdir(&mut self, base: &mut dyn Fs, path: &str) -> FsResult<()> {
        self.parent_is_dir(base, path)?;
        if self.stat(base, path).is_ok() {
            return Err(Error::AlreadyExists);
        }
        self.map.insert(key(path), (path.into(), Change::Dir));
        Ok(())
    }

    pub fn remove(&mut self, base: &mut dyn Fs, path: &str) -> FsResult<()> {
        if path == "/" {
            return Err(Error::AccessDenied);
        }
        let m = self.stat(base, path)?;
        if m.kind == NodeKind::Dir && !self.list(base, path)?.is_empty() {
            return Err(Error::NotEmpty);
        }
        let in_base = !self.hidden(path) && base.stat(path).is_ok();
        if in_base {
            self.map.insert(key(path), (path.into(), Change::Removed));
        } else {
            self.map.remove(&key(path));
        }
        Ok(())
    }

    /// Smaze i slozku s obsahem.
    pub fn remove_all(&mut self, base: &mut dyn Fs, path: &str) -> FsResult<()> {
        let m = self.stat(base, path)?;
        if m.kind == NodeKind::Dir {
            for e in self.list(base, path)? {
                self.remove_all(base, &join(path, &e.name))?;
            }
        }
        self.remove(base, path)
    }

    pub fn copy(&mut self, base: &mut dyn Fs, from: &str, to: &str) -> FsResult<()> {
        // Sam do sebe (ani do sve podslozky) kopirovat nejde.
        if super::path::within(to, from) {
            return Err(Error::InvalidArgs);
        }
        let m = self.stat(base, from)?;
        if m.kind == NodeKind::Dir {
            self.mkdir(base, to)?;
            for e in self.list(base, from)? {
                self.copy(base, &join(from, &e.name), &join(to, &e.name))?;
            }
            return Ok(());
        }
        let data = self.read(base, from)?;
        self.write(base, to, data)
    }

    pub fn rename(&mut self, base: &mut dyn Fs, from: &str, to: &str) -> FsResult<()> {
        if self.stat(base, to).is_ok() {
            return Err(Error::AlreadyExists);
        }
        if super::path::within(to, from) {
            return Err(Error::InvalidArgs);
        }
        self.copy(base, from, to)?;
        self.remove_all(base, from)
    }

    /// Co se proti FS zmeni (serazeno podle cesty).
    pub fn changes(&self, base: &mut dyn Fs) -> FsResult<Vec<DiffEntry>> {
        let mut out = Vec::new();
        for (orig, ch) in self.map.values() {
            let before = base.stat(orig);
            let entry = match (ch, before) {
                (Change::File(d), Ok(m)) if m.kind == NodeKind::File => {
                    if base.read(orig).ok().as_deref() == Some(d.as_slice()) {
                        continue;
                    }
                    DiffEntry { path: orig.clone(), change: ChangeKind::Modified, before: Some(m.size), after: Some(d.len() as u64) }
                }
                (Change::File(d), Ok(_)) => {
                    DiffEntry { path: orig.clone(), change: ChangeKind::Replaced, before: None, after: Some(d.len() as u64) }
                }
                (Change::File(d), Err(_)) => {
                    DiffEntry { path: orig.clone(), change: ChangeKind::Added, before: None, after: Some(d.len() as u64) }
                }
                (Change::Dir, Ok(m)) if m.kind == NodeKind::Dir => continue,
                (Change::Dir, Ok(m)) => {
                    DiffEntry { path: orig.clone(), change: ChangeKind::Replaced, before: Some(m.size), after: None }
                }
                (Change::Dir, Err(_)) => {
                    DiffEntry { path: orig.clone(), change: ChangeKind::DirAdded, before: None, after: None }
                }
                (Change::Removed, Ok(m)) => DiffEntry {
                    path: orig.clone(),
                    change: if m.kind == NodeKind::Dir { ChangeKind::DirRemoved } else { ChangeKind::Removed },
                    before: (m.kind != NodeKind::Dir).then_some(m.size),
                    after: None,
                },
                (Change::Removed, Err(_)) => continue,
            };
            out.push(entry);
        }
        Ok(out)
    }

    /// Prevezme zmeny do FS: nejdriv mazani (od listu; smazana slozka je
    /// v prekryvu prazdna, takze uvnitr nic noveho byt nemuze), pak uvolneni
    /// cest, kde se soubor meni na slozku a naopak, nove slozky (od korene)
    /// a nakonec soubory.
    pub fn commit(&self, base: &mut dyn Fs) -> FsResult<()> {
        let mut removed: Vec<&String> =
            self.map.values().filter(|(_, c)| *c == Change::Removed).map(|(p, _)| p).collect();
        removed.sort_by_key(|p| core::cmp::Reverse(p.len()));
        for p in removed {
            match base.remove(p) {
                Ok(()) | Err(Error::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        for (p, c) in self.map.values() {
            let Ok(m) = base.stat(p) else { continue };
            match (c, m.kind) {
                (Change::File(_), NodeKind::Dir) => super::undo::remove_all(base, p)?,
                (Change::Dir, k) if k != NodeKind::Dir => base.remove(p)?,
                _ => {}
            }
        }
        let mut dirs: Vec<&String> =
            self.map.values().filter(|(_, c)| *c == Change::Dir).map(|(p, _)| p).collect();
        dirs.sort_by_key(|p| p.len());
        for d in dirs {
            match base.stat(d) {
                Ok(m) if m.kind == NodeKind::Dir => {}
                _ => base.mkdir(d)?,
            }
        }
        for (p, c) in self.map.values() {
            if let Change::File(data) = c {
                base.write(p, data)?;
            }
        }
        Ok(())
    }
}
