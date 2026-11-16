//! Slozky exFAT: cteni polozek, hledani jmena (bez ohledu na velikost
//! pismen podle tabulky media), vlozeni sady (pripadne rozsireni slozky
//! o cluster), prepis sady na miste a uvolneni slotu.

use volproto::{Error, Result};

use super::entry::{self, ENTRY, FileSet, Raw, ty};
use super::{ExFatVolume, parts, read_at, split, write_at};
use crate::dev::Sectors;

/// Nejvetsi slozka (specifikace: 256 MiB).
const MAX_DIR: u64 = 256 << 20;

/// Sada polozek v nadrazene slozce.
#[derive(Clone, Debug)]
pub struct Node {
    /// Clustery slozky, ve ktere sada lezi.
    pub dir_clusters: Vec<u32>,
    pub dir_first: u32,
    /// Index prvni polozky sady ve slozce.
    pub slot: usize,
    pub set: FileSet,
}

/// Otevrena slozka: jeji clustery a vlastni sada (koren zadnou nema).
pub struct DirRef {
    pub clusters: Vec<u32>,
    pub node: Option<Node>,
}

impl<D: Sectors> ExFatVolume<D> {
    pub(super) fn root(&mut self) -> Result<DirRef> {
        Ok(DirRef { clusters: self.chain_end(self.g.root)?, node: None })
    }

    pub(super) fn read_dir_data(&mut self, cl: &[u32]) -> Result<Vec<u8>> {
        let len = cl.len() as u64 * self.g.cluster_size();
        if len > MAX_DIR {
            return Err(Error::Corrupted);
        }
        let mut data = vec![0u8; len as usize];
        self.read_clusters(cl, 0, &mut data)?;
        Ok(data)
    }

    /// Platne sady souboru ve slozce (poskozene se vynechaji).
    pub(super) fn entries(&mut self, dir: &DirRef) -> Result<Vec<Node>> {
        let data = self.read_dir_data(&dir.clusters)?;
        let slots = data.len() / ENTRY;
        let raw = |i: usize| -> Raw {
            let mut r = [0u8; ENTRY];
            r.copy_from_slice(&data[i * ENTRY..(i + 1) * ENTRY]);
            r
        };
        let mut out = Vec::new();
        let mut i = 0;
        while i < slots {
            let t = data[i * ENTRY];
            if t == ty::END {
                break;
            }
            if t == ty::FILE {
                let n = data[i * ENTRY + 1] as usize + 1;
                if i + n <= slots {
                    let set: Vec<Raw> = (i..i + n).map(raw).collect();
                    if let Ok(f) = entry::parse(&set, &self.up)
                        && !f.name.iter().any(|&c| c == 0 || c == b'/' as u16)
                        && f.name != [b'.' as u16]
                        && f.name != [b'.' as u16, b'.' as u16]
                    {
                        out.push(Node { dir_clusters: dir.clusters.clone(), dir_first: dir.clusters.first().copied().unwrap_or(0), slot: i, set: f });
                        i += n;
                        continue;
                    }
                }
            }
            i += 1;
        }
        Ok(out)
    }

    /// Slozka obsahuje nejakou pouzitou polozku (i poskozenou).
    pub(super) fn any_entry(&mut self, dir: &DirRef) -> Result<bool> {
        let data = self.read_dir_data(&dir.clusters)?;
        for e in data.chunks_exact(ENTRY) {
            if e[0] == ty::END {
                break;
            }
            if e[0] & ty::IN_USE != 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn find(&mut self, dir: &DirRef, name: &str) -> Result<Option<Node>> {
        let want: Vec<u16> = name.encode_utf16().collect();
        Ok(self.entries(dir)?.into_iter().find(|n| self.up.eq(&n.set.name, &want)))
    }

    pub(super) fn open_dir(&mut self, path: &str) -> Result<DirRef> {
        let mut cur = self.root()?;
        for p in parts(path) {
            let n = self.find(&cur, p)?.ok_or(Error::NotFound)?;
            if !n.set.is_dir() {
                return Err(Error::NotADirectory);
            }
            cur = DirRef { clusters: self.file_chain(&n.set)?, node: Some(n) };
        }
        Ok(cur)
    }

    pub(super) fn lookup(&mut self, path: &str) -> Result<Node> {
        let (parent, name) = split(path).ok_or(Error::InvalidArgs)?;
        let dir = self.open_dir(parent)?;
        self.find(&dir, name)?.ok_or(Error::NotFound)
    }

    fn slot_pos(&self, cl: &[u32], slot: usize) -> Result<u64> {
        let cs = self.g.cluster_size() as usize;
        let b = slot * ENTRY;
        let c = *cl.get(b / cs).ok_or(Error::Corrupted)?;
        Ok(self.g.cluster_pos(c) + (b % cs) as u64)
    }

    fn write_slots(&mut self, cl: &[u32], slot: usize, set: &[Raw]) -> Result<()> {
        for (k, e) in set.iter().enumerate() {
            let pos = self.slot_pos(cl, slot + k)?;
            write_at(&mut self.dev, pos, e)?;
        }
        Ok(())
    }

    /// Prepise sadu na jejim miste (stejny nebo mensi pocet polozek).
    pub(super) fn write_set(&mut self, n: &Node) -> Result<()> {
        let raw = entry::build(&n.set, &self.up);
        self.write_slots(&n.dir_clusters, n.slot, &raw)
    }

    /// Polozky `slot..slot+count` jako nepouzite (bit 7 pryc).
    pub(super) fn mark_slots_unused(&mut self, cl: &[u32], slot: usize, count: usize) -> Result<()> {
        for k in 0..count {
            let pos = self.slot_pos(cl, slot + k)?;
            let mut t = [0u8; 1];
            read_at(&mut self.dev, pos, &mut t)?;
            write_at(&mut self.dev, pos, &[t[0] & !ty::IN_USE])?;
        }
        Ok(())
    }

    pub(super) fn mark_unused(&mut self, n: &Node) -> Result<()> {
        self.mark_slots_unused(&n.dir_clusters, n.slot, n.set.entries())
    }

    /// Vlozi sadu do slozky (prvni dost dlouhy usek volnych slotu, jinak
    /// slozku rozsiri o cluster).
    pub(super) fn insert_set(&mut self, dir: &mut DirRef, set: &FileSet) -> Result<Node> {
        let need = set.entries();
        loop {
            let data = self.read_dir_data(&dir.clusters)?;
            let slots = data.len() / ENTRY;
            let mut run = 0;
            let mut end = false;
            let mut found = None;
            for i in 0..slots {
                let t = data[i * ENTRY];
                if t == ty::END {
                    end = true;
                }
                // Za koncem polozek je vse volne.
                if end || t & ty::IN_USE == 0 {
                    run += 1;
                    if run == need {
                        found = Some(i + 1 - need);
                        break;
                    }
                } else {
                    run = 0;
                }
            }
            if let Some(slot) = found {
                let raw = entry::build(set, &self.up);
                self.write_slots(&dir.clusters, slot, &raw)?;
                let node = Node { dir_clusters: dir.clusters.clone(), dir_first: dir.clusters.first().copied().unwrap_or(0), slot, set: set.clone() };
                return Ok(node);
            }
            self.grow_dir(dir)?;
        }
    }

    /// Slozka o cluster vetsi (nuly = konec polozek).
    fn grow_dir(&mut self, dir: &mut DirRef) -> Result<()> {
        let cs = self.g.cluster_size();
        if (dir.clusters.len() as u64 + 1) * cs > MAX_DIR {
            return Err(Error::NoSpace);
        }
        match &mut dir.node {
            None => {
                let c = self.extend_fat_chain(&mut dir.clusters)?;
                self.zero_clusters(&[c])?;
            }
            Some(node) => {
                let len = node.set.len + cs;
                let mut set = node.set.clone();
                let cl = self.resize(&mut set, len)?;
                let new = *cl.last().ok_or(Error::Corrupted)?;
                self.zero_clusters(&[new])?;
                set.len = len;
                set.valid_len = len;
                node.set = set;
                let n = node.clone();
                self.write_set(&n)?;
                dir.clusters = cl;
            }
        }
        Ok(())
    }
}
