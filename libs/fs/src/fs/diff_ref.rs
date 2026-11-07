//! Puvodni plny rozdil (projde oba stromy cele) - jen pro testy jako
//! reference rychleho rozdilu (`delta.rs`).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Result;

use super::Fs;
use super::diff::{Change, ChangeKind};
use crate::btree;
use crate::device::BlockDevice;
use crate::hash::{Csum, xxh3};
use crate::layout::{Inode, KIND_DATASET, KIND_DIR, Key, META_OID, ROOT_OID, decode_dirents, item};
use crate::node::Child;

/// Souhrn jedne verze stromu potrebny pro porovnani.
#[derive(Default)]
struct Version {
    /// oid -> (druh, velikost, otisk obsahu)
    objects: BTreeMap<u64, (u8, u64, Csum)>,
    /// oid -> (rodic, jmeno)
    names: BTreeMap<u64, (u64, String)>,
}

impl Version {
    /// Cesta objektu ve verzi (s mezi pro poskozeny cyklus rodicu).
    fn path(&self, mut oid: u64) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for _ in 0..super::MAX_DEPTH {
            if oid == ROOT_OID {
                break;
            }
            let Some((parent, name)) = self.names.get(&oid) else {
                break;
            };
            parts.push(name);
            oid = *parent;
        }
        let mut p = String::new();
        for part in parts.iter().rev() {
            p.push('/');
            p.push_str(part);
        }
        if p.is_empty() {
            p.push('/');
        }
        p
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Nacte souhrn stromu `root` (vsechny objekty, jmena, otisky obsahu).
    fn version_of(&mut self, root: &Child) -> Result<Version> {
        let mut v = Version::default();
        let mut content: BTreeMap<u64, Csum> = BTreeMap::new();
        let mut err = None;
        let tf = self.s.time_format();
        btree::range(
            &mut self.s,
            root,
            &Key::new(ROOT_OID, 0, 0),
            &Key::new(u64::MAX, u8::MAX, u64::MAX),
            &mut |k, val| {
                if k.oid == META_OID {
                    return Ok(true);
                }
                match k.kind {
                    item::INODE => match Inode::decode(val, tf) {
                        Ok(i) => {
                            v.objects.insert(k.oid, (i.kind, i.size, Csum::default()));
                        }
                        Err(e) => {
                            err = Some(e);
                            return Ok(false);
                        }
                    },
                    item::DIRENT => match decode_dirents(val) {
                        Ok(list) => {
                            for d in list {
                                // Vstup do jineho datasetu nema objekt v tomto strome.
                                if d.kind != KIND_DATASET {
                                    v.names.insert(d.oid, (k.oid, d.name));
                                }
                            }
                        }
                        Err(e) => {
                            err = Some(e);
                            return Ok(false);
                        }
                    },
                    item::INLINE | item::EXTENT | item::SYMLINK => {
                        // Otisk = soucet (predchozi otisk || klic || hodnota).
                        let prev = content.get(&k.oid).copied().unwrap_or_default();
                        let mut buf = Vec::with_capacity(16 + 9 + val.len());
                        buf.extend_from_slice(&prev.0);
                        buf.push(k.kind);
                        buf.extend_from_slice(&k.offset.to_le_bytes());
                        buf.extend_from_slice(val);
                        content.insert(k.oid, xxh3(&buf));
                    }
                    _ => {}
                }
                Ok(true)
            },
        )?;
        if let Some(e) = err {
            return Err(e);
        }
        for (oid, sum) in content {
            if let Some(o) = v.objects.get_mut(&oid) {
                o.2 = sum;
            }
        }
        Ok(v)
    }

    pub(crate) fn diff_roots_full(&mut self, old_root: &Child, new_root: &Child) -> Result<Vec<Change>> {
        let old = self.version_of(old_root)?;
        let new = self.version_of(new_root)?;
        let mut out = Vec::new();
        for (&oid, &(kind, size, sum)) in &new.objects {
            if oid == ROOT_OID {
                continue;
            }
            let is_dir = kind == KIND_DIR;
            let path = new.path(oid);
            match old.objects.get(&oid) {
                None => out.push(Change { kind: ChangeKind::Added, path, old_path: None, oid, is_dir }),
                Some(&(_, osize, osum)) => {
                    let moved = old.names.get(&oid) != new.names.get(&oid);
                    let modified = !is_dir && (osize != size || osum != sum);
                    let kind = match (moved, modified) {
                        (true, true) => ChangeKind::MovedModified,
                        (true, false) => ChangeKind::Moved,
                        (false, true) => ChangeKind::Modified,
                        (false, false) => continue,
                    };
                    let old_path = moved.then(|| old.path(oid));
                    out.push(Change { kind, path, old_path, oid, is_dir });
                }
            }
        }
        for (&oid, &(kind, _, _)) in &old.objects {
            if oid != ROOT_OID && !new.objects.contains_key(&oid) {
                out.push(Change {
                    kind: ChangeKind::Removed,
                    path: old.path(oid),
                    old_path: None,
                    oid,
                    is_dir: kind == KIND_DIR,
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}
