//! Indexy verzi stromu pro trojcestne slouceni (`merge.rs`).
//!
//! Planovac slouceni potrebuje o objektech (rodic, jmeno, druh, generace,
//! velikost, otisk obsahu) a vypisy slozek ciloveho stromu. Ucinek na
//! vysledek maji jen objekty, ktere se zmenily mezi predkem B a jejich
//! verzi T (objekt nezmeneny v T se neprevezme, at je u nas jakykoli).
//! Proto se:
//! 1. porovnaji stromy B a T (`treediff`, shodne podstromy se preskoci) ->
//!    kandidati = oid zmenenych klicu a objekty z hodnot zmenenych polozek
//!    adresaru (presuny),
//! 2. kandidati nactou bodove ve vsech trech verzich (B, nase O, T) i se
//!    vsemi predky (cesty, hloubky, cilove slozky, kontrola cyklu) a vypisy
//!    jejich slozek v O (kolize jmen, neprazdne slozky, datasety),
//! 3. co planovac potrebuje navic (shoda jmena noveho objektu s nasim),
//!    se nacte za behu (`ensure`).
//!
//! Index tak obsahuje jen nactene objekty; pristup k nenactenemu je chyba
//! programu (v ladicim sestaveni assert). V testech jde zapnout puvodni
//! uplny index (`super::reference_mode`) jako referenci.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use abi::Result;

use super::Fs;
use crate::btree;
use crate::device::BlockDevice;
use crate::hash::{Csum, xxh3};
use crate::layout::{Inode, KIND_DATASET, KIND_DIR, Key, META_OID, ROOT_OID, TRASH_OID, decode_dirents, item};
use crate::node::Child;
use crate::treediff;

/// Objekt jedne verze stromu.
#[derive(Clone)]
pub(super) struct Obj {
    pub kind: u8,
    pub generation: u64,
    pub size: u64,
    /// Otisk polozek s obsahem (fyzicky - stejny blok = stejny obsah).
    pub fp: Csum,
    pub parent: u64,
    pub name: String,
}

/// Polozka vypisu slozky.
pub(super) struct Entry {
    pub oid: u64,
    pub name: String,
    /// Vstup do jineho datasetu (do slouceni nepatri, ale zabira jmeno
    /// a slozku dela neprazdnou).
    pub dataset: bool,
}

/// Souhrn jedne verze stromu (bez kose a vstupu do jinych datasetu).
pub(super) struct Index {
    pub root: Child,
    /// Uplny index: chybejici objekt = neexistuje (reference v testech).
    complete: bool,
    /// Nactene objekty (`None` = neexistuje nebo nema jmeno).
    objs: BTreeMap<u64, Option<Obj>>,
    /// Nactene vypisy slozek.
    dirs: BTreeMap<u64, Vec<Entry>>,
}

impl Index {
    fn lazy(root: Child) -> Self {
        Self { root, complete: false, objs: BTreeMap::new(), dirs: BTreeMap::new() }
    }

    pub fn obj(&self, oid: u64) -> Option<&Obj> {
        debug_assert!(self.complete || self.objs.contains_key(&oid), "slouceni: objekt {oid} neni v indexu nacteny");
        self.objs.get(&oid)?.as_ref()
    }

    pub fn has(&self, oid: u64) -> bool {
        self.obj(oid).is_some()
    }

    /// Polozky slozky (prazdne, kdyz slozka neni nebo nema polozky).
    pub fn entries(&self, dir: u64) -> &[Entry] {
        debug_assert!(
            self.complete || self.dirs.contains_key(&dir) || self.objs.get(&dir).is_some_and(|o| o.as_ref().is_none_or(|o| o.kind != KIND_DIR)),
            "slouceni: slozka {dir} neni v indexu nactena"
        );
        self.dirs.get(&dir).map_or(&[], |v| v.as_slice())
    }

    /// Ma slozka vstup do jineho datasetu?
    pub fn has_datasets(&self, dir: u64) -> bool {
        self.entries(dir).iter().any(|e| e.dataset)
    }

    pub fn path(&self, mut oid: u64) -> String {
        let mut parts: Vec<&str> = Vec::new();
        for _ in 0..super::MAX_DEPTH {
            if oid == ROOT_OID {
                break;
            }
            let Some(o) = self.obj(oid) else { break };
            parts.push(&o.name);
            oid = o.parent;
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

    pub fn depth(&self, mut oid: u64) -> usize {
        let mut d = 0;
        while oid != ROOT_OID && d < super::MAX_DEPTH {
            match self.obj(oid) {
                Some(o) => oid = o.parent,
                None => break,
            }
            d += 1;
        }
        d
    }
}

/// Tri indexy slouceni a kandidati (vzestupne podle oid).
pub(super) struct Indexes {
    pub b: Index,
    pub o: Index,
    pub t: Index,
    pub cands: Vec<u64>,
    /// Objekty uz nactene ve vsech trech indexech (i s predky).
    pub ensured: BTreeSet<u64>,
}

/// Retezeny otisk polozky obsahu.
fn chain(prev: Csum, k: &Key, v: &[u8]) -> Csum {
    let mut buf = Vec::with_capacity(16 + 9 + v.len());
    buf.extend_from_slice(&prev.0);
    buf.push(k.kind);
    buf.extend_from_slice(&k.offset.to_le_bytes());
    buf.extend_from_slice(v);
    xxh3(&buf)
}

impl<D: BlockDevice> Fs<D> {
    /// Indexy pro slouceni `theirs` do `ours` proti `base`.
    pub(super) fn merge_indexes(&mut self, base: &Child, theirs: &Child, ours: &Child) -> Result<Indexes> {
        if super::reference_mode() {
            let b = self.merge_index_full(base)?;
            let t = self.merge_index_full(theirs)?;
            let o = self.merge_index_full(ours)?;
            let mut all: BTreeSet<u64> = b.objs.keys().copied().collect();
            all.extend(t.objs.keys().copied());
            return Ok(Indexes { b, o, t, cands: all.into_iter().collect(), ensured: BTreeSet::new() });
        }
        let mut cands = BTreeSet::new();
        treediff::diff(&mut self.s, base, theirs, &mut |k, a, b| {
            if k.oid == META_OID || k.oid == TRASH_OID {
                return Ok(());
            }
            cands.insert(k.oid);
            if k.kind == item::DIRENT {
                for v in [a, b].into_iter().flatten() {
                    for d in decode_dirents(v)? {
                        if d.kind != KIND_DATASET {
                            cands.insert(d.oid);
                        }
                    }
                }
            }
            Ok(())
        })?;
        let mut ix = Indexes {
            b: Index::lazy(*base),
            o: Index::lazy(*ours),
            t: Index::lazy(*theirs),
            cands: cands.into_iter().collect(),
            ensured: BTreeSet::new(),
        };
        for &x in &ix.cands {
            self.ensure_in(&mut ix.b, &mut ix.o, &mut ix.t, &mut ix.ensured, x)?;
        }
        Ok(ix)
    }

    /// Nacte objekt `oid` do vsech tri indexu, s predky (ve vsech verzich)
    /// a u nasich slozek s vypisem.
    pub(super) fn ensure_in(&mut self, b: &mut Index, o: &mut Index, t: &mut Index, ensured: &mut BTreeSet<u64>, oid: u64) -> Result<()> {
        if b.complete {
            return Ok(());
        }
        let mut work = alloc::vec![oid];
        while let Some(x) = work.pop() {
            if !ensured.insert(x) {
                continue;
            }
            for idx in [&mut *b, &mut *o, &mut *t] {
                self.load_obj(idx, x)?;
                if let Some(o) = idx.objs[&x].as_ref()
                    && o.parent != 0
                {
                    work.push(o.parent);
                }
            }
            if o.objs[&x].as_ref().is_some_and(|ob| ob.kind == KIND_DIR) {
                self.load_dir(o, x)?;
            }
        }
        Ok(())
    }

    /// Vypis slozky `dir` do indexu (polozky kose do slouceni nepatri).
    fn load_dir(&mut self, idx: &mut Index, dir: u64) -> Result<()> {
        if idx.dirs.contains_key(&dir) {
            return Ok(());
        }
        let mut list = Vec::new();
        if dir != TRASH_OID && dir != META_OID {
            for (_, v) in btree::collect(&mut self.s, &idx.root, &Key::new(dir, item::DIRENT, 0), &Key::new(dir, item::DIRENT + 1, 0))? {
                for d in decode_dirents(&v)? {
                    list.push(Entry { oid: d.oid, name: d.name, dataset: d.kind == KIND_DATASET });
                }
            }
        }
        idx.dirs.insert(dir, list);
        Ok(())
    }

    /// Nacte objekt `oid` (zaznam, jmeno z polozek rodice, otisk obsahu).
    fn load_obj(&mut self, idx: &mut Index, oid: u64) -> Result<()> {
        if idx.objs.contains_key(&oid) {
            return Ok(());
        }
        let obj = if oid == META_OID || oid == TRASH_OID {
            None
        } else {
            match btree::get(&mut self.s, &idx.root, &Key::new(oid, item::INODE, 0))? {
                None => None,
                Some(raw) => {
                    let i = Inode::decode(&raw, self.s.time_format())?;
                    let place = if oid == ROOT_OID {
                        Some((0, String::new()))
                    } else if i.parent == 0 {
                        // Bez jmena (sirotek): do slouceni nepatri.
                        None
                    } else {
                        self.load_dir(idx, i.parent)?;
                        idx.dirs[&i.parent].iter().find(|e| !e.dataset && e.oid == oid).map(|e| (i.parent, e.name.clone()))
                    };
                    match place {
                        Some((parent, name)) => {
                            let mut fp = Csum::default();
                            btree::range(
                                &mut self.s,
                                &idx.root,
                                &Key::new(oid, item::INLINE, 0),
                                &Key::new(oid, item::SYMLINK + 1, 0),
                                &mut |k, v| {
                                    if matches!(k.kind, item::INLINE | item::EXTENT | item::SYMLINK) {
                                        fp = chain(fp, k, v);
                                    }
                                    Ok(true)
                                },
                            )?;
                            Some(Obj { kind: i.kind, generation: i.generation, size: i.size, fp, parent, name })
                        }
                        None => None,
                    }
                }
            }
        };
        idx.objs.insert(oid, obj);
        Ok(())
    }

    /// Uplny index stromu (puvodni pruchod celym stromem; reference).
    fn merge_index_full(&mut self, root: &Child) -> Result<Index> {
        let mut idx = Index { root: *root, complete: true, objs: BTreeMap::new(), dirs: BTreeMap::new() };
        let mut names: BTreeMap<u64, (u64, String)> = BTreeMap::new();
        let mut datasets: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        let mut content: BTreeMap<u64, Csum> = BTreeMap::new();
        let mut inodes: BTreeMap<u64, Inode> = BTreeMap::new();
        let tf = self.s.time_format();
        let mut err = None;
        btree::range(&mut self.s, root, &Key::new(ROOT_OID, 0, 0), &Key::new(u64::MAX, u8::MAX, u64::MAX), &mut |k, v| {
            if k.oid == META_OID || k.oid == TRASH_OID {
                return Ok(true);
            }
            let r: Result<()> = (|| {
                match k.kind {
                    item::INODE => {
                        inodes.insert(k.oid, Inode::decode(v, tf)?);
                    }
                    item::DIRENT => {
                        for d in decode_dirents(v)? {
                            if d.kind != KIND_DATASET {
                                names.insert(d.oid, (k.oid, d.name));
                            } else {
                                datasets.entry(k.oid).or_default().push(d.name);
                            }
                        }
                    }
                    item::INLINE | item::EXTENT | item::SYMLINK => {
                        let prev = content.get(&k.oid).copied().unwrap_or_default();
                        content.insert(k.oid, chain(prev, k, v));
                    }
                    _ => {}
                }
                Ok(())
            })();
            if let Err(e) = r {
                err = Some(e);
                return Ok(false);
            }
            Ok(true)
        })?;
        if let Some(e) = err {
            return Err(e);
        }
        for (dir, list) in datasets {
            idx.dirs.entry(dir).or_default().extend(list.into_iter().map(|name| Entry { oid: 0, name, dataset: true }));
        }
        for (oid, i) in inodes {
            let (parent, name) = if oid == ROOT_OID {
                (0, String::new())
            } else {
                match names.remove(&oid) {
                    Some(x) => x,
                    None => continue,
                }
            };
            if oid != ROOT_OID {
                idx.dirs.entry(parent).or_default().push(Entry { oid, name: name.clone(), dataset: false });
            }
            idx.objs.insert(
                oid,
                Some(Obj { kind: i.kind, generation: i.generation, size: i.size, fp: content.get(&oid).copied().unwrap_or_default(), parent, name }),
            );
        }
        Ok(idx)
    }
}
