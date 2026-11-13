//! FS v pameti pro testy (stroj kopirovani, model spravce souboru na
//! hostiteli). Datasety jsou predpony cest; okamzitou kopii jde vypnout
//! (jako sifrovany trezor) a zapisy jde po N operacich selhat (vypadek).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::path::{join, parent};
use crate::{Error, Kind, Meta, Result, Vfs};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    File(Vec<u8>),
    Dir,
    Link(String),
}

#[derive(Debug, Clone)]
pub struct MemFs {
    pub nodes: BTreeMap<String, Node>,
    /// Koreny datasetu (krome `/`).
    pub datasets: Vec<String>,
    /// Okamzita kopie funguje.
    pub clone: bool,
    /// Polozky v kosi (puvodni cesta, obsah podstromu).
    pub trashed: Vec<(String, Vec<(String, Node)>)>,
    /// Po tolika menicich operacich vsechny dalsi selzou `IO_ERROR`.
    pub fail_after: Option<u32>,
    pub ops: u32,
    pub syncs: u32,
    /// Pocet okamzitych kopii (test, ze se nekopirovalo ctenim).
    pub clones: u32,
}

impl Default for MemFs {
    fn default() -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(String::from("/"), Node::Dir);
        Self { nodes, datasets: Vec::new(), clone: true, trashed: Vec::new(), fail_after: None, ops: 0, syncs: 0, clones: 0 }
    }
}

impl MemFs {
    /// Vytvori slozky po ceste.
    pub fn dir(&mut self, path: &str) -> &mut Self {
        let mut p = String::new();
        for part in path.split('/').filter(|s| !s.is_empty()) {
            p.push('/');
            p.push_str(part);
            self.nodes.entry(p.clone()).or_insert(Node::Dir);
        }
        self
    }

    pub fn file(&mut self, path: &str, data: &[u8]) -> &mut Self {
        self.dir(parent(path));
        self.nodes.insert(String::from(path), Node::File(data.to_vec()));
        self
    }

    pub fn dataset(&mut self, path: &str) -> &mut Self {
        self.dir(path);
        self.datasets.push(String::from(path));
        self
    }

    pub fn data(&self, path: &str) -> Option<&[u8]> {
        match self.nodes.get(path) {
            Some(Node::File(d)) => Some(d),
            _ => None,
        }
    }

    pub fn exists(&self, path: &str) -> bool {
        self.nodes.contains_key(path)
    }

    /// Cesty pod `dir` (bez nej), serazene.
    pub fn below(&self, dir: &str) -> Vec<String> {
        let pre = join(dir, "");
        self.nodes.keys().filter(|k| k.starts_with(&pre)).cloned().collect()
    }

    fn change(&mut self) -> Result<()> {
        self.ops += 1;
        match self.fail_after {
            Some(n) if self.ops > n => Err(Error::IoError),
            _ => Ok(()),
        }
    }

    fn need_parent(&self, path: &str) -> Result<()> {
        match self.nodes.get(parent(path)) {
            Some(Node::Dir) => Ok(()),
            Some(_) => Err(Error::WrongType),
            None => Err(Error::NotFound),
        }
    }

    fn need_free(&self, path: &str) -> Result<()> {
        if self.nodes.contains_key(path) { Err(Error::AlreadyExists) } else { Ok(()) }
    }

    /// Polozka a jeji podstrom (relativni cesty, `""` = polozka).
    fn subtree(&self, path: &str) -> Vec<(String, Node)> {
        let mut v = Vec::new();
        if let Some(n) = self.nodes.get(path) {
            v.push((String::new(), n.clone()));
        }
        let pre = join(path, "");
        for (k, n) in &self.nodes {
            if let Some(rest) = k.strip_prefix(&pre) {
                v.push((String::from(rest), n.clone()));
            }
        }
        v
    }

    fn drop_tree(&mut self, path: &str) {
        let pre = join(path, "");
        self.nodes.retain(|k, _| k != path && !k.starts_with(&pre));
    }

    fn put_tree(&mut self, path: &str, tree: Vec<(String, Node)>) {
        for (rel, n) in tree {
            let p = if rel.is_empty() { String::from(path) } else { join(path, &rel) };
            self.nodes.insert(p, n);
        }
    }
}

impl Vfs for MemFs {
    fn stat(&mut self, path: &str) -> Result<Meta> {
        match self.nodes.get(path).ok_or(Error::NotFound)? {
            Node::File(d) => Ok(Meta { kind: Kind::File, size: d.len() as u64, tree_bytes: None }),
            Node::Link(t) => Ok(Meta { kind: Kind::Link, size: t.len() as u64, tree_bytes: None }),
            Node::Dir => {
                let total = self.subtree(path).iter().map(|(_, n)| if let Node::File(d) = n { d.len() as u64 } else { 0 }).sum();
                Ok(Meta { kind: Kind::Dir, size: 0, tree_bytes: Some(total) })
            }
        }
    }

    fn list(&mut self, path: &str) -> Result<Vec<(String, Kind)>> {
        if self.nodes.get(path) != Some(&Node::Dir) {
            return Err(Error::NotFound);
        }
        let pre = join(path, "");
        Ok(self
            .nodes
            .iter()
            .filter_map(|(k, n)| {
                let rest = k.strip_prefix(&pre)?;
                (!rest.is_empty() && !rest.contains('/')).then(|| {
                    let kind = match n {
                        Node::File(_) => Kind::File,
                        Node::Dir => Kind::Dir,
                        Node::Link(_) => Kind::Link,
                    };
                    (String::from(rest), kind)
                })
            })
            .collect())
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        self.need_parent(path)?;
        self.need_free(path)?;
        self.change()?;
        self.nodes.insert(String::from(path), Node::Dir);
        Ok(())
    }

    fn create_file(&mut self, path: &str) -> Result<()> {
        self.need_parent(path)?;
        self.need_free(path)?;
        self.change()?;
        self.nodes.insert(String::from(path), Node::File(Vec::new()));
        Ok(())
    }

    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let d = match self.nodes.get(path) {
            Some(Node::File(d)) => d,
            Some(_) => return Err(Error::WrongType),
            None => return Err(Error::NotFound),
        };
        let start = (offset as usize).min(d.len());
        Ok(d[start..(start + len).min(d.len())].to_vec())
    }

    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        self.change()?;
        let Some(Node::File(d)) = self.nodes.get_mut(path) else { return Err(Error::NotFound) };
        let end = offset as usize + data.len();
        if d.len() < end {
            d.resize(end, 0);
        }
        d[offset as usize..end].copy_from_slice(data);
        Ok(())
    }

    fn clone_path(&mut self, src: &str, dst: &str) -> Result<()> {
        if !self.clone {
            return Err(Error::NotSupported);
        }
        self.need_parent(dst)?;
        self.need_free(dst)?;
        let tree = self.subtree(src);
        if tree.is_empty() {
            return Err(Error::NotFound);
        }
        self.change()?;
        self.clones += 1;
        self.put_tree(dst, tree);
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.need_parent(to)?;
        self.need_free(to)?;
        let tree = self.subtree(from);
        if tree.is_empty() {
            return Err(Error::NotFound);
        }
        self.change()?;
        self.drop_tree(from);
        self.put_tree(to, tree);
        Ok(())
    }

    fn trash(&mut self, path: &str) -> Result<()> {
        let tree = self.subtree(path);
        if tree.is_empty() {
            return Err(Error::NotFound);
        }
        self.change()?;
        self.drop_tree(path);
        self.trashed.push((String::from(path), tree));
        Ok(())
    }

    fn remove_tree(&mut self, path: &str) -> Result<()> {
        if !self.nodes.contains_key(path) {
            return Err(Error::NotFound);
        }
        self.change()?;
        self.drop_tree(path);
        Ok(())
    }

    fn dataset_of(&mut self, path: &str) -> Result<String> {
        if !self.nodes.contains_key(path) {
            return Err(Error::NotFound);
        }
        Ok(self
            .datasets
            .iter()
            .filter(|d| crate::path::is_within(path, d))
            .max_by_key(|d| d.len())
            .cloned()
            .unwrap_or_else(|| String::from("/")))
    }

    fn sync(&mut self) -> Result<()> {
        self.syncs += 1;
        Ok(())
    }
}
