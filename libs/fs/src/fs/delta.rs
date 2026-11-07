//! Rozdil dvou verzi stromu datasetu umerny zmene (jadro `Fs::diff`).
//!
//! 1. `treediff::diff` vrati jen klice, ktere se mezi stromy lisi (shodne
//!    podstromy se preskoci podle ukazatele).
//! 2. Ze zmenenych klicu plyne, ktere objekty mohou mit zmenu:
//!    - `INODE` -> pridany, odebrany nebo jina velikost,
//!    - `INLINE`/`EXTENT`/`SYMLINK` -> zmeneny obsah (otisk obsahu by se
//!      lisil prave tehdy, kdyz se lisi nektera z techto polozek),
//!    - `DIRENT` -> objekty uvedene ve stare nebo nove hodnote mohly byt
//!      presunute/prejmenovane (jine misto = jina polozka adresare).
//! 3. Pro tyto objekty bodove dotazy: zaznam objektu v obou verzich, jmeno
//!    (ze zmenenych polozek adresare, jinak z rodice `Inode::parent` a
//!    pruchodu jeho polozek) a cesta z retezu rodicu (vse s cache).
//!
//! Predpoklad (plati v konzistentnim FS, kontroluje ho `check`): objekt ma
//! prave jednu polozku adresare a `Inode::parent` ukazuje na slozku s ni.
//! Cena: zmenene klice + vypisy slozek, ve kterych zmeny lezi (a jejich
//! predku pro cesty) - ne velikost datasetu.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use abi::Result;

use super::Fs;
use super::diff::{Change, ChangeKind};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{Inode, KIND_DATASET, KIND_DIR, Key, META_OID, ROOT_OID, decode_dirents, item};
use crate::node::Child;
use crate::treediff;

/// (druh, velikost, rodic) ze zaznamu objektu.
type InodeInfo = (u8, u64, u64);

/// Verze: 0 = starsi, 1 = novejsi, `SHARED` = stejna v obou (cache).
const SHARED: usize = 2;

#[derive(Default)]
struct Delta {
    /// Objekty, ktere mohou mit zmenu.
    cand: BTreeSet<u64>,
    /// Objekty se zmenenou polozkou obsahu.
    content: BTreeSet<u64>,
    /// Objekty se zmenenym zaznamem `INODE`.
    inode_changed: BTreeSet<u64>,
    /// Slozky se zmenenou polozkou `DIRENT`.
    dirs_changed: BTreeSet<u64>,
    /// Objekty uvedene ve zmenenych polozkach adresaru (v kterekoli verzi).
    renamed: BTreeSet<u64>,
    /// Jmena ze zmenenych polozek adresaru: oid -> (rodic, jmeno), po verzich.
    names: [BTreeMap<u64, (u64, String)>; 2],
    /// Cache: (verze, oid) -> (druh, velikost, rodic).
    inodes: BTreeMap<(usize, u64), Option<InodeInfo>>,
    /// Cache: (verze, slozka) -> oid -> jmeno.
    lists: BTreeMap<(usize, u64), BTreeMap<u64, String>>,
    /// Cache: (verze, oid) -> (rodic, jmeno).
    resolved: BTreeMap<(usize, u64), Option<(u64, String)>>,
}

impl Delta {
    fn record(&mut self, k: &Key, vals: [Option<&[u8]>; 2]) -> Result<()> {
        if k.oid == META_OID {
            return Ok(());
        }
        match k.kind {
            item::INODE => {
                self.cand.insert(k.oid);
                self.inode_changed.insert(k.oid);
            }
            item::DIRENT => {
                self.dirs_changed.insert(k.oid);
                for (v, val) in vals.iter().enumerate() {
                    let Some(val) = val else { continue };
                    for d in decode_dirents(val)? {
                        // Vstup do jineho datasetu nema objekt v tomto strome.
                        if d.kind != KIND_DATASET {
                            self.cand.insert(d.oid);
                            self.renamed.insert(d.oid);
                            self.names[v].insert(d.oid, (k.oid, d.name));
                        }
                    }
                }
            }
            item::INLINE | item::EXTENT | item::SYMLINK => {
                self.cand.insert(k.oid);
                self.content.insert(k.oid);
            }
            _ => {}
        }
        Ok(())
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Rozdil stromu `old_root` -> `new_root` (serazeno podle cesty).
    pub(super) fn diff_roots(&mut self, old_root: &Child, new_root: &Child) -> Result<Vec<Change>> {
        #[cfg(test)]
        if super::reference_mode() {
            return self.diff_roots_full(old_root, new_root);
        }
        let roots = [*old_root, *new_root];
        let mut d = Delta::default();
        treediff::diff(&mut self.s, old_root, new_root, &mut |k, a, b| d.record(k, [a, b]))?;
        let cand: Vec<u64> = d.cand.iter().copied().filter(|&x| x != ROOT_OID).collect();
        let mut out = Vec::new();
        // Poradi jako u plneho pruchodu: objekty nove verze, pak odebrane
        // (razeni podle cesty je stabilni).
        let mut removed = Vec::new();
        for &oid in &cand {
            let new = self.d_inode(&mut d, &roots, 1, oid)?;
            let old = self.d_inode(&mut d, &roots, 0, oid)?;
            let Some((kind, size, _)) = new else {
                if let Some((okind, _, _)) = old {
                    removed.push((oid, okind));
                }
                continue;
            };
            let is_dir = kind == KIND_DIR;
            let Some((_, osize, _)) = old else {
                let path = self.d_path(&mut d, &roots, 1, oid)?;
                out.push(Change { kind: ChangeKind::Added, path, old_path: None, oid, is_dir });
                continue;
            };
            let moved = d.renamed.contains(&oid)
                && self.d_name(&mut d, &roots, 0, oid)? != self.d_name(&mut d, &roots, 1, oid)?;
            let modified = !is_dir && (osize != size || d.content.contains(&oid));
            let ck = match (moved, modified) {
                (true, true) => ChangeKind::MovedModified,
                (true, false) => ChangeKind::Moved,
                (false, true) => ChangeKind::Modified,
                (false, false) => continue,
            };
            let path = self.d_path(&mut d, &roots, 1, oid)?;
            let old_path = if moved { Some(self.d_path(&mut d, &roots, 0, oid)?) } else { None };
            out.push(Change { kind: ck, path, old_path, oid, is_dir });
        }
        for (oid, okind) in removed {
            let path = self.d_path(&mut d, &roots, 0, oid)?;
            out.push(Change { kind: ChangeKind::Removed, path, old_path: None, oid, is_dir: okind == KIND_DIR });
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// (druh, velikost, rodic) objektu ve verzi `v`.
    fn d_inode(&mut self, d: &mut Delta, roots: &[Child; 2], v: usize, oid: u64) -> Result<Option<InodeInfo>> {
        let ck = if d.inode_changed.contains(&oid) { v } else { SHARED };
        if let Some(x) = d.inodes.get(&(ck, oid)) {
            return Ok(*x);
        }
        let tf = self.s.time_format();
        let r = match btree::get(&mut self.s, &roots[v], &Key::new(oid, item::INODE, 0))? {
            Some(raw) => {
                let i = Inode::decode(&raw, tf)?;
                Some((i.kind, i.size, i.parent))
            }
            None => None,
        };
        d.inodes.insert((ck, oid), r);
        Ok(r)
    }

    /// (rodic, jmeno) objektu ve verzi `v` (`None` = bez jmena).
    fn d_name(&mut self, d: &mut Delta, roots: &[Child; 2], v: usize, oid: u64) -> Result<Option<(u64, String)>> {
        if let Some(x) = d.names[v].get(&oid) {
            return Ok(Some(x.clone()));
        }
        if d.renamed.contains(&oid) {
            // Ve zmenenych polozkach jen druhe verze: v teto verzi ho zadna
            // polozka nema (nezmenena polozka by ho mela i v druhe verzi).
            return Ok(None);
        }
        if let Some(x) = d.resolved.get(&(v, oid)) {
            return Ok(x.clone());
        }
        let r = match self.d_inode(d, roots, v, oid)? {
            Some((_, _, parent)) if parent != 0 => {
                self.d_list(d, roots, v, parent)?;
                let ck = if d.dirs_changed.contains(&parent) { v } else { SHARED };
                d.lists[&(ck, parent)].get(&oid).map(|n| (parent, n.clone()))
            }
            _ => None,
        };
        d.resolved.insert((v, oid), r.clone());
        Ok(r)
    }

    /// Nacte polozky slozky `dir` ve verzi `v` do cache.
    fn d_list(&mut self, d: &mut Delta, roots: &[Child; 2], v: usize, dir: u64) -> Result<()> {
        let ck = if d.dirs_changed.contains(&dir) { v } else { SHARED };
        if d.lists.contains_key(&(ck, dir)) {
            return Ok(());
        }
        let mut m = BTreeMap::new();
        for (_, val) in btree::collect(&mut self.s, &roots[v], &Key::new(dir, item::DIRENT, 0), &Key::new(dir, item::DIRENT + 1, 0))? {
            for e in decode_dirents(&val)? {
                if e.kind != KIND_DATASET {
                    m.insert(e.oid, e.name);
                }
            }
        }
        d.lists.insert((ck, dir), m);
        Ok(())
    }

    /// Cesta objektu ve verzi `v` (s mezi pro poskozeny cyklus rodicu).
    fn d_path(&mut self, d: &mut Delta, roots: &[Child; 2], v: usize, mut oid: u64) -> Result<String> {
        let mut parts: Vec<String> = Vec::new();
        for _ in 0..super::MAX_DEPTH {
            if oid == ROOT_OID {
                break;
            }
            let Some((parent, name)) = self.d_name(d, roots, v, oid)? else {
                break;
            };
            parts.push(name);
            oid = parent;
        }
        let mut p = String::new();
        for part in parts.iter().rev() {
            p.push('/');
            p.push_str(part);
        }
        if p.is_empty() {
            p.push('/');
        }
        Ok(p)
    }
}
