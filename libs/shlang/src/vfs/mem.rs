//! Souborovy system v pameti (testy na hostiteli), vcetne snapshotu.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::path::{file_name, join, key, parent};
use crate::host::{DirEntry, Fs, FsResult, Meta, NodeKind};

#[derive(Debug, Clone, PartialEq)]
enum Node {
    File(Vec<u8>),
    Dir,
}

/// FS v pameti: klic = cesta malymi pismeny, hodnota = (puvodni cesta, uzel).
#[derive(Debug, Clone, Default)]
pub struct MemFs {
    nodes: BTreeMap<String, (String, Node)>,
    snapshots: BTreeMap<String, BTreeMap<String, (String, Node)>>,
    /// Pocet zapisu (testy: kolik se toho opravdu zapsalo).
    pub writes: usize,
    pub total: u64,
}

impl MemFs {
    pub fn new() -> Self {
        let mut fs = Self { total: 1 << 30, ..Default::default() };
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

    fn meta(n: &Node) -> Meta {
        match n {
            Node::File(d) => Meta { kind: NodeKind::File, size: d.len() as u64, mtime: Some(0) },
            Node::Dir => Meta { kind: NodeKind::Dir, size: 0, mtime: Some(0) },
        }
    }

    fn list_in(nodes: &BTreeMap<String, (String, Node)>, path: &str) -> FsResult<Vec<DirEntry>> {
        match nodes.get(&key(path)) {
            Some((_, Node::Dir)) => {}
            Some(_) => return Err(Error::NotADirectory),
            None => return Err(Error::NotFound),
        }
        let k = key(path);
        Ok(nodes
            .iter()
            .filter(|(nk, _)| nk.as_str() != "/" && parent(nk) == Some(k.as_str()))
            .map(|(_, (orig, n))| DirEntry { name: file_name(orig).into(), meta: Self::meta(n) })
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
        self.nodes.get(&key(path)).map(|(_, n)| Self::meta(n)).ok_or(Error::NotFound)
    }

    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        Self::list_in(&self.nodes, path)
    }

    fn read(&mut self, path: &str) -> FsResult<Vec<u8>> {
        match self.nodes.get(&key(path)) {
            Some((_, Node::File(d))) => Ok(d.clone()),
            Some(_) => Err(Error::IsADirectory),
            None => Err(Error::NotFound),
        }
    }

    fn write(&mut self, path: &str, data: &[u8]) -> FsResult<()> {
        self.parent_dir(path)?;
        if let Some((_, Node::Dir)) = self.nodes.get(&key(path)) {
            return Err(Error::IsADirectory);
        }
        self.writes += 1;
        self.nodes.insert(key(path), (path.into(), Node::File(data.into())));
        Ok(())
    }

    fn mkdir(&mut self, path: &str) -> FsResult<()> {
        self.parent_dir(path)?;
        if self.nodes.contains_key(&key(path)) {
            return Err(Error::AlreadyExists);
        }
        self.writes += 1;
        self.nodes.insert(key(path), (path.into(), Node::Dir));
        Ok(())
    }

    fn remove(&mut self, path: &str) -> FsResult<()> {
        if path == "/" {
            return Err(Error::AccessDenied);
        }
        match self.nodes.get(&key(path)) {
            None => return Err(Error::NotFound),
            Some((_, Node::Dir)) if !Self::list_in(&self.nodes, path)?.is_empty() => return Err(Error::NotEmpty),
            _ => {}
        }
        self.writes += 1;
        self.nodes.remove(&key(path));
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> FsResult<()> {
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

    fn snapshot(&mut self, name: &str) -> FsResult<()> {
        if self.snapshots.contains_key(name) {
            return Err(Error::AlreadyExists);
        }
        self.snapshots.insert(name.into(), self.nodes.clone());
        Ok(())
    }

    fn delete_snapshot(&mut self, name: &str) -> FsResult<()> {
        self.snapshots.remove(name).map(|_| ()).ok_or(Error::NotFound)
    }

    fn snapshot_stat(&mut self, snap: &str, path: &str) -> FsResult<Meta> {
        let s = self.snapshots.get(snap).ok_or(Error::NotFound)?;
        s.get(&key(path)).map(|(_, n)| Self::meta(n)).ok_or(Error::NotFound)
    }

    fn snapshot_list(&mut self, snap: &str, path: &str) -> FsResult<Vec<DirEntry>> {
        let s = self.snapshots.get(snap).ok_or(Error::NotFound)?;
        Self::list_in(s, path)
    }

    fn snapshot_read(&mut self, snap: &str, path: &str) -> FsResult<Vec<u8>> {
        let s = self.snapshots.get(snap).ok_or(Error::NotFound)?;
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
