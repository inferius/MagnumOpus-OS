//! Souborovy system v pameti (testy na hostiteli), vcetne datasetu,
//! klonu bokem a pohledu (viz [`super::mem_hist`]).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::path::{file_name, join, key, parent};
use crate::host::{Applied, Aside, DirEntry, Fs, FsChange, FsResult, Meta, NodeKind, ViewRef};

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Node {
    File(Vec<u8>),
    Dir,
}

/// FS v pameti: klic = cesta malymi pismeny, hodnota = (puvodni cesta, uzel).
#[derive(Debug, Clone, Default)]
pub struct MemFs {
    pub(super) nodes: BTreeMap<String, (String, Node)>,
    pub(super) hist: super::mem_hist::Hist,
    /// Pocet zapisu (testy: kolik se toho opravdu zapsalo).
    pub writes: usize,
    pub total: u64,
    /// Kolik konfliktu FS vrati (vic = zkraceny seznam, jako fsd).
    pub conflict_limit: usize,
    /// Pocet dotazu `dataset_of` (testy: cteni se na dataset nepta).
    pub dataset_queries: usize,
}

impl MemFs {
    pub fn new() -> Self {
        let mut fs = Self { total: 1 << 30, conflict_limit: usize::MAX, ..Default::default() };
        fs.nodes.insert("/".into(), ("/".into(), Node::Dir));
        fs
    }

    /// Pomocnik pro testy: vytvori soubor i s nadrazenymi slozkami.
    pub fn put(&mut self, path: &str, data: &[u8]) {
        let mut cur = String::new();
        let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        for p in &parts[..parts.len().saturating_sub(1)] {
            cur.push('/');
            cur.push_str(p);
            self.nodes.entry(key(&cur)).or_insert((cur.clone(), Node::Dir));
        }
        self.nodes.insert(key(path), (path.into(), Node::File(data.into())));
    }

    pub fn get(&self, path: &str) -> Option<&[u8]> {
        match self.nodes.get(&key(path)) {
            Some((_, Node::File(d))) => Some(d),
            _ => None,
        }
    }

    pub fn is_dir(&self, path: &str) -> bool {
        matches!(self.nodes.get(&key(path)), Some((_, Node::Dir)))
    }

    pub fn exists(&self, path: &str) -> bool {
        self.nodes.contains_key(&key(path))
    }

    /// Metadata se souhrnem podstromu u slozky (jako fsd).
    pub(super) fn meta_in(nodes: &BTreeMap<String, (String, Node)>, k: &str, n: &Node) -> Meta {
        let mut m = Self::meta(n);
        if matches!(n, Node::Dir) {
            let pre = alloc::format!("{}/", k.trim_end_matches('/'));
            let (mut bytes, mut files, mut dirs) = (0u64, 0u64, 0u64);
            for (sub, (_, x)) in nodes.range(pre.clone()..) {
                if !sub.starts_with(&pre) {
                    break;
                }
                match x {
                    Node::File(d) => {
                        bytes += d.len() as u64;
                        files += 1;
                    }
                    Node::Dir => dirs += 1,
                }
            }
            m.size = bytes;
            m.tree = Some((files, dirs));
        }
        m
    }

    pub(super) fn meta(n: &Node) -> Meta {
        match n {
            Node::File(d) => Meta::basic(NodeKind::File, d.len() as u64),
            Node::Dir => Meta::basic(NodeKind::Dir, 0),
        }
    }

    pub(super) fn list_in(nodes: &BTreeMap<String, (String, Node)>, path: &str) -> FsResult<Vec<DirEntry>> {
        match nodes.get(&key(path)) {
            Some((_, Node::Dir)) => {}
            Some(_) => return Err(Error::NotADirectory),
            None => return Err(Error::NotFound),
        }
        let k = key(path);
        Ok(nodes
            .iter()
            .filter(|(nk, _)| nk.as_str() != "/" && parent(nk) == Some(k.as_str()))
            .map(|(nk, (orig, n))| DirEntry { name: file_name(orig).into(), meta: Self::meta_in(nodes, nk, n) })
            .collect())
    }

    fn parent_dir(&self, path: &str) -> FsResult<()> {
        let p = parent(path).ok_or(Error::InvalidArgs)?;
        match self.nodes.get(&key(p)) {
            Some((_, Node::Dir)) => Ok(()),
            Some(_) => Err(Error::NotADirectory),
            None => Err(Error::NotFound),
        }
    }
}

impl Fs for MemFs {
    fn stat(&mut self, path: &str) -> FsResult<Meta> {
        self.guard_inside(path)?;
        let k = key(path);
        self.nodes.get(&k).map(|(_, n)| Self::meta_in(&self.nodes, &k, n)).ok_or(Error::NotFound)
    }

    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        self.guard(path)?;
        Self::list_in(&self.nodes, path)
    }

    fn read(&mut self, path: &str) -> FsResult<Vec<u8>> {
        self.guard_inside(path)?;
        match self.nodes.get(&key(path)) {
            Some((_, Node::File(d))) => Ok(d.clone()),
            Some(_) => Err(Error::IsADirectory),
            None => Err(Error::NotFound),
        }
    }

    fn write(&mut self, path: &str, data: &[u8]) -> FsResult<()> {
        self.guard(path)?;
        self.parent_dir(path)?;
        if let Some((_, Node::Dir)) = self.nodes.get(&key(path)) {
            return Err(Error::IsADirectory);
        }
        self.writes += 1;
        self.bump(path);
        self.nodes.insert(key(path), (path.into(), Node::File(data.into())));
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> FsResult<()> {
        self.guard(path)?;
        self.parent_dir(path)?;
        if self.nodes.contains_key(&key(path)) {
            return Err(Error::AlreadyExists);
        }
        self.writes += 1;
        self.bump(path);
        self.nodes.insert(key(path), (path.into(), Node::Dir));
        Ok(())
    }

    fn remove(&mut self, path: &str) -> FsResult<()> {
        if path == "/" {
            return Err(Error::AccessDenied);
        }
        self.guard(path)?;
        match self.nodes.get(&key(path)) {
            None => return Err(Error::NotFound),
            Some((_, Node::Dir)) if !Self::list_in(&self.nodes, path)?.is_empty() => return Err(Error::NotEmpty),
            _ => {}
        }
        self.writes += 1;
        self.bump(path);
        self.nodes.remove(&key(path));
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> FsResult<()> {
        self.guard(from)?;
        self.guard(to)?;
        if !self.nodes.contains_key(&key(from)) {
            return Err(Error::NotFound);
        }
        self.parent_dir(to)?;
        if self.nodes.contains_key(&key(to)) {
            return Err(Error::AlreadyExists);
        }
        let fk = key(from);
        let moved: Vec<(String, (String, Node))> = self
            .nodes
            .iter()
            .filter(|(k, _)| **k == fk || k.starts_with(&alloc::format!("{fk}/")))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (k, (orig, n)) in moved {
            self.nodes.remove(&k);
            let rest = &orig[from.len()..];
            let new = alloc::format!("{to}{rest}");
            self.nodes.insert(key(&new), (new, n));
        }
        self.writes += 1;
        self.bump(from);
        self.bump(to);
        Ok(())
    }

    fn space(&mut self) -> FsResult<(u64, u64)> {
        let used: u64 = self
            .nodes
            .values()
            .map(|(_, n)| match n {
                Node::File(d) => d.len() as u64,
                Node::Dir => 0,
            })
            .sum();
        Ok((self.total, self.total.saturating_sub(used)))
    }

    /// Jako fsd: neexistujici cesta = `NOT_FOUND`.
    fn dataset_of(&mut self, path: &str) -> FsResult<String> {
        self.dataset_queries += 1;
        if !self.exists(path) {
            return Err(Error::NotFound);
        }
        Ok(self.dataset_root(path))
    }

    fn aside_begin(&mut self, dataset: &str) -> FsResult<Aside> {
        self.h_aside_begin(dataset)
    }

    fn aside_changes(&mut self, aside: &Aside) -> FsResult<Vec<FsChange>> {
        self.h_aside_changes(aside)
    }

    fn aside_apply(&mut self, aside: &Aside) -> FsResult<Applied> {
        let conflicts = self.h_aside_apply(aside)?;
        let complete = conflicts.len() <= self.conflict_limit;
        let conflicts = conflicts.into_iter().take(self.conflict_limit).collect();
        Ok(Applied { conflicts, complete })
    }

    fn aside_discard(&mut self, aside: &Aside) -> FsResult<()> {
        self.h_aside_discard(aside)
    }

    fn view(&mut self, dataset: &str) -> FsResult<ViewRef> {
        self.h_view(dataset)
    }

    fn release_view(&mut self, view: &ViewRef) -> FsResult<()> {
        self.hist.views.remove(&view.snapshot).map(|_| ()).ok_or(Error::NotFound)
    }

    fn changed_since(&mut self, view: &ViewRef) -> FsResult<Vec<FsChange>> {
        self.h_changed_since(view)
    }

    fn view_stat(&mut self, view: &ViewRef, path: &str) -> FsResult<Meta> {
        self.view_guard(view, path, false)?;
        let s = self.hist.views.get(&view.snapshot).ok_or(Error::NotFound)?;
        let k = key(path);
        s.get(&k).map(|(_, n)| Self::meta_in(s, &k, n)).ok_or(Error::NotFound)
    }

    fn view_list(&mut self, view: &ViewRef, path: &str) -> FsResult<Vec<DirEntry>> {
        self.view_guard(view, path, true)?;
        let s = self.hist.views.get(&view.snapshot).ok_or(Error::NotFound)?;
        Self::list_in(s, path)
    }

    fn view_read(&mut self, view: &ViewRef, path: &str) -> FsResult<Vec<u8>> {
        self.view_guard(view, path, false)?;
        let s = self.hist.views.get(&view.snapshot).ok_or(Error::NotFound)?;
        match s.get(&key(path)) {
            Some((_, Node::File(d))) => Ok(d.clone()),
            Some(_) => Err(Error::IsADirectory),
            None => Err(Error::NotFound),
        }
    }
}

/// Pomocnik: cesta ke klici.
pub fn path_of(dir: &str, name: &str) -> String {
    join(dir, name)
}
