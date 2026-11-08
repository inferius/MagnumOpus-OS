//! Trojcestne slouceni stromu: merge vetve a prevzeti zmen "zkusit
//! zmeny bokem".
//!
//! Vstup: predek **B** (snapshot), **nase** O (ziva verze ciloveho
//! datasetu) a **jejich** T (commit vetve nebo zivy klon). Objekt se pozna
//! podle oid a generace jen tehdy, kdyz existoval v B (klon i vetev
//! z B pokracuji se stejnymi oid; nove objekty na obou stranach mohou mit
//! stejne oid, ale jsou to ruzne objekty - paruji se podle cesty).
//!
//! **Postup - nejdriv cely plan bez zmeny, pak provedeni:**
//! 1. plan: pro kazdy objekt z B misto (rodic, jmeno) a obsah trojcestne;
//!    nove objekty z T podle cesty; smazane v T do kose (ne trvale);
//!    simulace jmen v cilovych slozkach (kolize -> jmeno s "konflikt"),
//!    kontrola cyklu u presunu slozek,
//! 2. provedeni v O: nove slozky a soubory, presuny pres pomocnou slozku
//!    (vymeny jmen nevadi), obsah, kos; nakonec commit.
//!
//! **Konflikty:** v rezimu `Merge` (vetev) se resi hned - text se znackami
//! (`merge3`), binarni soubor nebo zmena proti smazani jako kopie vedle
//! ("jmeno (konflikt vetev).ext"); v rezimu `Aside` se konfliktni zmena
//! neprevezme vubec a jen se ohlasi.
//!
//! **Vnorene datasety** (polozky `KIND_DATASET`) slouceni nikdy nemeni:
//! v B a T jsou to jen hranice (kopie polozek, viz `dataset.rs`), v O
//! skutecne datasety (i klony jinych relaci, ktere mezitim vznikly). Jejich
//! jmena v O jsou obsazena (nova polozka z T stejneho jmena = konflikt)
//! a slozka v O, ktera obsahuje dataset, neni prazdna - smazani v T je
//! konflikt `DirNotEmpty` (do kose by odnesla i dataset).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::merge_index::{Index, Obj};
use super::{Fs, Kind, zil};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{
    BLOCK_SIZE, Inode, KIND_DIR, KIND_FILE, KIND_SYMLINK, Key, META_OID, ROOT_OID, item,
};
use crate::merge3;
use crate::names::fold_name;
use crate::node::Child;
use crate::perm::rights;
use crate::records::snapkind;

/// Druh konfliktu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictKind {
    /// Obsah zmeneny na obou stranach jinak.
    Content,
    /// U nas zmeneny, u nich smazany (nase verze zustala).
    ModifiedDeleted,
    /// U nas smazany, u nich zmeneny.
    DeletedModified,
    /// Stejne jmeno pridane na obou stranach (jiny obsah).
    BothAdded,
    /// Presunuty na obou stranach jinam (nase misto zustalo).
    BothMoved,
    /// Cilova slozka u nas uz neexistuje.
    ParentMissing,
    /// Slozka smazana u nich, ale u nas v ni neco zbylo.
    DirNotEmpty,
    /// Presun by udelal slozku svym vlastnim potomkem.
    Cycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeConflict {
    /// Cesta (uvnitr datasetu) u nich, u smazanych u predka.
    pub path: String,
    pub kind: ConflictKind,
    /// Kam se ulozila jejich verze (kopie vedle, soubor se znackami);
    /// prazdne v rezimu `Aside`.
    pub resolved_as: String,
}

/// Vysledek slouceni.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MergeReport {
    /// Prevzate zmeny (cesty u nich).
    pub applied: Vec<String>,
    pub conflicts: Vec<MergeConflict>,
    /// Commit slouceni (merge vetve bez konfliktu).
    pub commit: Option<super::CommitInfo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// Merge vetve: konflikty vyresit znackami nebo kopii vedle.
    Merge,
    /// Prevzeti zmen z klonu: konflikt neprevzit.
    Aside,
}

/// Cil v nasem strome: existujici slozka (oid) nebo nova slozka z T (jejich oid).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Dir {
    Ours(u64),
    New(u64),
}

/// Krok planu.
enum Step {
    /// Novy objekt z T (oid u nich) do slozky pod jmenem.
    Create { t_oid: u64, dir: Dir, name: String },
    /// Presun naseho objektu.
    Move { oid: u64, dir: Dir, name: String },
    /// Obsah naseho souboru z T (`None`) nebo slouceny.
    Content { oid: u64, t_oid: u64, data: Option<Vec<u8>> },
    /// Nas objekt do kose.
    Trash { oid: u64 },
}

/// Pripona souboru vcetne tecky ("" bez pripony; tecka na zacatku neni).
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    }
}

struct Planner<'a> {
    b: Index,
    o: Index,
    t: Index,
    /// Objekty, ktere se zmenily v T proti B (jen ty mohou neco zmenit),
    /// vzestupne podle oid; viz `merge_index.rs`.
    cands: Vec<u64>,
    /// Objekty nactene ve vsech trech indexech.
    ensured: BTreeSet<u64>,
    mode: Mode,
    label: String,
    select: Option<&'a [String]>,
    steps: Vec<Step>,
    report: MergeReport,
    /// Simulace jmen v cilovych slozkach (slozena jmena).
    names: BTreeMap<Dir, BTreeSet<String>>,
    /// Kam se presouva nas objekt (konecny rodic).
    moved: BTreeMap<u64, Dir>,
    /// Nove slozky z T, ktere se vytvori.
    created: BTreeSet<u64>,
    trashed: BTreeSet<u64>,
    /// Slozka pridana na obou stranach: jejich oid -> nase oid.
    alias: BTreeMap<u64, u64>,
    /// Presuny do novych slozek z T: (oid, nas rodic, nase jmeno, jejich
    /// rodic, jejich jmeno, cesta).
    deferred: Vec<(u64, u64, String, u64, String, String)>,
}

impl Planner<'_> {
    fn selected(&self, paths: &[&str]) -> bool {
        let Some(sel) = self.select else { return true };
        paths.iter().any(|p| {
            sel.iter().any(|s| {
                let s = s.trim_end_matches('/');
                s.is_empty() || *p == s || p.strip_prefix(s).is_some_and(|r| r.starts_with('/'))
            })
        })
    }

    fn base_obj(&self, side: &Index, oid: u64) -> Option<Obj> {
        let b = self.b.obj(oid)?;
        side.obj(oid).filter(|o| o.generation == b.generation && o.kind == b.kind).cloned()
    }

    fn dir_names(&mut self, d: Dir) -> &mut BTreeSet<String> {
        let o = &self.o;
        self.names.entry(d).or_insert_with(|| match d {
            Dir::Ours(oid) => o.entries(oid).iter().map(|e| fold_name(&e.name)).collect(),
            Dir::New(_) => BTreeSet::new(),
        })
    }

    /// Volne jmeno ve slozce: `name`, jinak "stem (konflikt label)ext"
    /// (pak s cislem). Jmeno se rovnou zabere.
    fn claim(&mut self, d: Dir, name: &str) -> (String, bool) {
        let label = self.label.clone();
        let set = self.dir_names(d);
        if set.insert(fold_name(name)) {
            return (name.into(), false);
        }
        let (stem, ext) = split_ext(name);
        let mut n = 1;
        loop {
            let cand = if n == 1 {
                alloc::format!("{stem} (konflikt {label}){ext}")
            } else {
                alloc::format!("{stem} (konflikt {label} {n}){ext}")
            };
            if set.insert(fold_name(&cand)) {
                return (cand, true);
            }
            n += 1;
        }
    }

    fn release(&mut self, d: Dir, name: &str) {
        self.dir_names(d).remove(&fold_name(name));
    }

    fn conflict(&mut self, path: String, kind: ConflictKind, resolved_as: String) {
        self.report.conflicts.push(MergeConflict { path, kind, resolved_as });
    }

    /// Nase slozka pro rodice `p` z T: existujici u nas, nebo nova z T.
    fn target_dir(&self, p: u64) -> Option<Dir> {
        if p == ROOT_OID {
            return Some(Dir::Ours(ROOT_OID));
        }
        if let Some(&o) = self.alias.get(&p) {
            return Some(Dir::Ours(o));
        }
        if self.b.has(p) {
            return self.base_obj(&self.o, p).filter(|o| o.kind == KIND_DIR && !self.trashed.contains(&p)).map(|_| Dir::Ours(p));
        }
        self.created.contains(&p).then_some(Dir::New(p))
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Nacte objekt `oid` do indexu planovace (viz `merge_index.rs`).
    fn ensure_planner(&mut self, p: &mut Planner<'_>, oid: u64) -> Result<()> {
        self.ensure_in(&mut p.b, &mut p.o, &mut p.t, &mut p.ensured, oid)
    }

    /// Cely obsah souboru (pro slouceni textu; jen do `TEXT_MAX`).
    fn read_all(&mut self, root: &Child, oid: u64, size: u64) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(size as usize);
        while (out.len() as u64) < size {
            let n = ((size - out.len() as u64) as usize).min(256 * BLOCK_SIZE);
            let d = self.read_oid(root, oid, out.len() as u64, n, false)?;
            if d.is_empty() {
                return Err(Error::Corrupted);
            }
            out.extend_from_slice(&d);
        }
        Ok(out)
    }

    /// Stejny obsah (logicky - jine bloky nevadi)?
    fn same_content(&mut self, ra: &Child, a: u64, rb: &Child, b: u64) -> Result<bool> {
        let (ia, ib) = (self.inode(ra, a)?, self.inode(rb, b)?);
        if ia.kind != ib.kind {
            return Ok(false);
        }
        match ia.kind {
            KIND_SYMLINK => Ok(self.readlink_oid(ra, a)? == self.readlink_oid(rb, b)?),
            KIND_FILE => {
                if ia.size != ib.size {
                    return Ok(false);
                }
                let mut tmp = Vec::new();
                let ha = self.content_hash(ra, a, &ia, &mut tmp)?;
                let hb = self.content_hash(rb, b, &ib, &mut tmp)?;
                Ok(ha == hb)
            }
            _ => Ok(true),
        }
    }

    /// Slouceni stromu `theirs` do zive verze aktivniho datasetu (O) proti
    /// predkovi `base`. Oba koreny musi byt na disku (commit pred volanim).
    pub(super) fn merge_trees(
        &mut self,
        base: &Child,
        theirs: &Child,
        mode: Mode,
        label: &str,
        select: Option<&[String]>,
    ) -> Result<MergeReport> {
        let ours = self.s.root;
        let ix = self.merge_indexes(base, theirs, &ours)?;
        let mut p = Planner {
            b: ix.b,
            o: ix.o,
            t: ix.t,
            cands: ix.cands,
            ensured: ix.ensured,
            mode,
            label: label.into(),
            select,
            steps: Vec::new(),
            report: MergeReport::default(),
            names: BTreeMap::new(),
            moved: BTreeMap::new(),
            created: BTreeSet::new(),
            trashed: BTreeSet::new(),
            alias: BTreeMap::new(),
            deferred: Vec::new(),
        };
        self.plan_base(&mut p, base, theirs)?;
        Self::plan_dir_deletes(&mut p);
        self.plan_new(&mut p, theirs)?;
        Self::plan_deferred(&mut p);
        Self::check_cycles(&mut p);
        let Planner { steps, report, t, .. } = p;
        // Odhad mista predem: nova data + uzly (chyba pred prvni zmenou).
        let mut blocks = 16u64;
        for s in &steps {
            blocks += match s {
                Step::Create { t_oid, .. } => t.obj(*t_oid).map_or(0, |x| x.size.div_ceil(BLOCK_SIZE as u64)) + 4,
                Step::Content { t_oid, data, .. } => {
                    data.as_ref().map_or_else(|| t.obj(*t_oid).map_or(0, |x| x.size), |d| d.len() as u64).div_ceil(BLOCK_SIZE as u64) + 4
                }
                _ => 4,
            };
        }
        self.ensure_room(blocks)?;
        self.guarded(|fs| fs.apply_steps(steps, theirs, &t))?;
        Ok(report)
    }

    fn plan_base(&mut self, p: &mut Planner<'_>, base: &Child, theirs: &Child) -> Result<()> {
        let ours = self.s.root;
        let base_ids: Vec<u64> = p.cands.iter().copied().filter(|&x| x != ROOT_OID && p.b.has(x)).collect();
        for x in base_ids {
            let Some(bo) = p.b.obj(x).cloned() else { continue };
            let oo = p.base_obj(&p.o, x);
            let to = p.base_obj(&p.t, x);
            let tpath = if to.is_some() { p.t.path(x) } else { p.b.path(x) };
            if !p.selected(&[&tpath, &p.b.path(x)]) {
                continue;
            }
            let ch = |s: &Obj| s.fp != bo.fp || s.size != bo.size;
            match (oo, to) {
                (None, None) => {}
                (Some(oo), None) => {
                    if bo.kind == KIND_DIR {
                        continue; // slozky az v `plan_dir_deletes`
                    }
                    if ch(&oo) {
                        p.conflict(tpath, ConflictKind::ModifiedDeleted, String::new());
                    } else {
                        p.trashed.insert(x);
                        p.release(Dir::Ours(oo.parent), &oo.name);
                        p.report.applied.push(tpath);
                    }
                }
                (None, Some(to)) => {
                    if bo.kind != KIND_DIR && ch(&to) {
                        let mut resolved = String::new();
                        if p.mode == Mode::Merge {
                            match p.target_dir(to.parent) {
                                Some(d) => {
                                    let (name, _) = p.claim(d, &to.name);
                                    resolved = name.clone();
                                    p.steps.push(Step::Create { t_oid: x, dir: d, name });
                                }
                                None => {
                                    p.conflict(tpath.clone(), ConflictKind::ParentMissing, String::new());
                                }
                            }
                        }
                        p.conflict(tpath, ConflictKind::DeletedModified, resolved);
                    }
                }
                (Some(oo), Some(to)) => {
                    // Misto.
                    let (lb, lo, lt) = ((bo.parent, &bo.name), (oo.parent, &oo.name), (to.parent, &to.name));
                    if lt != lb {
                        if lo == lb || lo == lt {
                            if lo != lt && !p.b.has(to.parent) && to.parent != ROOT_OID {
                                // Do nove slozky z T: az po jejim naplanovani.
                                p.moved.insert(x, Dir::New(to.parent));
                                p.deferred.push((x, oo.parent, oo.name.clone(), to.parent, to.name.clone(), tpath.clone()));
                            } else if lo != lt {
                                match p.target_dir(to.parent) {
                                    Some(d) => {
                                        p.release(Dir::Ours(oo.parent), &oo.name);
                                        let (name, renamed) = p.claim(d, &to.name);
                                        if renamed {
                                            p.conflict(tpath.clone(), ConflictKind::BothAdded, name.clone());
                                        }
                                        p.moved.insert(x, d);
                                        p.steps.push(Step::Move { oid: x, dir: d, name });
                                        p.report.applied.push(tpath.clone());
                                    }
                                    None => p.conflict(tpath.clone(), ConflictKind::ParentMissing, String::new()),
                                }
                            }
                        } else {
                            p.conflict(tpath.clone(), ConflictKind::BothMoved, String::new());
                        }
                    }
                    // Obsah.
                    if bo.kind == KIND_DIR || !ch(&to) {
                        continue;
                    }
                    if !ch(&oo) {
                        p.steps.push(Step::Content { oid: x, t_oid: x, data: None });
                        p.report.applied.push(tpath);
                        continue;
                    }
                    if self.same_content(&ours, x, theirs, x)? {
                        continue;
                    }
                    let text = bo.kind == KIND_FILE
                        && [bo.size, oo.size, to.size].iter().all(|&s| s as usize <= merge3::TEXT_MAX);
                    let merged = if text {
                        let (db, dour, dt) = (self.read_all(base, x, bo.size)?, self.read_all(&ours, x, oo.size)?, self.read_all(theirs, x, to.size)?);
                        (merge3::is_text(&db) && merge3::is_text(&dour) && merge3::is_text(&dt))
                            .then(|| merge3::merge(&db, &dour, &dt, "nase", &p.label))
                    } else {
                        None
                    };
                    match merged {
                        Some(m) if m.conflicts == 0 => {
                            p.steps.push(Step::Content { oid: x, t_oid: x, data: Some(m.data) });
                            p.report.applied.push(tpath);
                        }
                        Some(m) => {
                            if p.mode == Mode::Merge {
                                p.steps.push(Step::Content { oid: x, t_oid: x, data: Some(m.data) });
                                p.conflict(tpath.clone(), ConflictKind::Content, tpath);
                            } else {
                                p.conflict(tpath, ConflictKind::Content, String::new());
                            }
                        }
                        None => {
                            let mut resolved = String::new();
                            if p.mode == Mode::Merge {
                                let d = p.moved.get(&x).copied().unwrap_or(Dir::Ours(oo.parent));
                                let (name, _) = p.claim(d, &format_conflict(&oo.name, &p.label));
                                resolved = name.clone();
                                p.steps.push(Step::Create { t_oid: x, dir: d, name });
                            }
                            p.conflict(tpath, ConflictKind::Content, resolved);
                        }
                    }
                }
            }
        }
        // Mazani souboru uvolni jmena (az po vsech presunech - poradi
        // provedeni: presuny, pak kos).
        Ok(())
    }

    /// Nove objekty z T (rodice pred potomky).
    fn plan_new(&mut self, p: &mut Planner<'_>, theirs: &Child) -> Result<()> {
        let ours = self.s.root;
        let mut new: Vec<u64> = p.cands.iter().copied().filter(|&x| x != ROOT_OID && p.t.has(x) && p.base_obj(&p.t, x).is_none()).collect();
        new.sort_by_key(|&x| (p.t.depth(x), x));
        for x in new {
            let Some(to) = p.t.obj(x).cloned() else { continue };
            let tpath = p.t.path(x);
            if !p.selected(&[&tpath]) {
                continue;
            }
            let Some(d) = p.target_dir(to.parent) else {
                // Rodic chybi u nas (smazany) nebo nebyl vytvoren (konflikt vyse).
                if p.b.has(to.parent) {
                    p.conflict(tpath, ConflictKind::ParentMissing, String::new());
                }
                continue;
            };
            // Stejne jmeno pridane i u nas se stejnym obsahem: nic.
            if let Dir::Ours(od) = d {
                // Nase polozka stejneho jmena (nactena az ted - kandidati
                // jsou jen zmeny v T).
                let folded = fold_name(&to.name);
                let same: Vec<u64> = p
                    .o
                    .entries(od)
                    .iter()
                    .filter(|e| !e.dataset && fold_name(&e.name) == folded && !p.moved.contains_key(&e.oid))
                    .map(|e| e.oid)
                    .collect();
                let mut existing = None;
                for c in same {
                    self.ensure_planner(p, c)?;
                    if p.o.has(c) {
                        existing = Some(c);
                        break;
                    }
                }
                if let Some(e) = existing
                    && !p.b.has(e)
                    && p.o.obj(e).is_some_and(|o| o.kind == to.kind)
                {
                    if to.kind == KIND_DIR {
                        // Slozka pridana na obou stranach: obsah se slouci do ni.
                        p.alias.insert(x, e);
                        continue;
                    }
                    if self.same_content(&ours, e, theirs, x)? {
                        continue;
                    }
                }
            }
            if p.mode == Mode::Aside && p.dir_names(d).contains(&fold_name(&to.name)) {
                p.conflict(tpath, ConflictKind::BothAdded, String::new());
                continue;
            }
            let (name, renamed) = p.claim(d, &to.name);
            if renamed {
                p.conflict(tpath.clone(), ConflictKind::BothAdded, name.clone());
            } else {
                p.report.applied.push(tpath);
            }
            if to.kind == KIND_DIR {
                p.created.insert(x);
            }
            p.steps.push(Step::Create { t_oid: x, dir: d, name });
        }
        Ok(())
    }

    /// Slozky smazane u nich: do kose cela slozka, kdyz u nas obsahuje jen
    /// to, co se maze take; jinak konflikt.
    fn plan_dir_deletes(p: &mut Planner<'_>) {
        let mut dirs: Vec<u64> = p
            .cands
            .iter()
            .copied()
            .filter(|&x| x != ROOT_OID && p.b.obj(x).is_some_and(|o| o.kind == KIND_DIR))
            .filter(|&x| p.base_obj(&p.o, x).is_some() && p.base_obj(&p.t, x).is_none())
            .filter(|&x| p.selected(&[&p.b.path(x)]))
            .collect();
        // Nejhlubsi prvni.
        dirs.sort_by_key(|&x| core::cmp::Reverse(p.o.depth(x)));
        let mut whole: BTreeSet<u64> = BTreeSet::new();
        // Planovane presuny do slozky (nove objekty z T do slozky, kterou T
        // smazal, byt nemohou).
        let moved_in: BTreeSet<Dir> = p.moved.values().copied().collect();
        let created_in: BTreeSet<Dir> = p
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Create { dir, .. } => Some(*dir),
                _ => None,
            })
            .collect();
        for x in dirs {
            let kids: Vec<u64> = p.o.entries(x).iter().filter(|e| !e.dataset).map(|e| e.oid).collect();
            let ok = !moved_in.contains(&Dir::Ours(x))
                && !created_in.contains(&Dir::Ours(x))
                && !p.o.has_datasets(x)
                && kids.iter().all(|c| (p.trashed.contains(c) || whole.contains(c)) && !p.moved.contains_key(c));
            if ok {
                whole.insert(x);
            } else {
                let path = p.b.path(x);
                p.conflict(path, ConflictKind::DirNotEmpty, String::new());
            }
        }
        // Do kose jen nejvyssi cele slozky; jejich obsah s nimi.
        let mut top = Vec::new();
        for &x in &whole {
            let Some(parent) = p.o.obj(x).map(|o| o.parent) else { continue };
            if !whole.contains(&parent) {
                top.push(x);
            }
        }
        let inside = |p: &Planner<'_>, mut c: u64| -> bool {
            for _ in 0..super::MAX_DEPTH {
                let Some(o) = p.o.obj(c) else { return false };
                if whole.contains(&o.parent) {
                    return true;
                }
                if o.parent == ROOT_OID || o.parent == 0 {
                    return false;
                }
                c = o.parent;
            }
            false
        };
        let files: Vec<u64> = p.trashed.iter().copied().collect();
        for f in files {
            if !inside(p, f) {
                p.steps.push(Step::Trash { oid: f });
            }
        }
        for x in top {
            p.trashed.insert(x);
            if let Some(o) = p.o.obj(x).cloned() {
                p.release(Dir::Ours(o.parent), &o.name);
            }
            p.report.applied.push(p.b.path(x));
            p.steps.push(Step::Trash { oid: x });
        }
    }

    /// Presuny do novych slozek z T (ty uz jsou naplanovane).
    fn plan_deferred(p: &mut Planner<'_>) {
        for (x, oparent, oname, tparent, tname, tpath) in core::mem::take(&mut p.deferred) {
            match p.target_dir(tparent) {
                Some(d) => {
                    p.release(Dir::Ours(oparent), &oname);
                    let (name, renamed) = p.claim(d, &tname);
                    if renamed {
                        p.conflict(tpath.clone(), ConflictKind::BothAdded, name.clone());
                    }
                    p.moved.insert(x, d);
                    p.steps.push(Step::Move { oid: x, dir: d, name });
                    p.report.applied.push(tpath);
                }
                None => {
                    p.moved.remove(&x);
                    p.conflict(tpath, ConflictKind::ParentMissing, String::new());
                }
            }
        }
    }

    /// Presun slozky pod jejiho potomka: presun zrusit (nase misto zustane).
    fn check_cycles(p: &mut Planner<'_>) {
        let moved = p.moved.clone();
        for &x in moved.keys() {
            if p.o.obj(x).is_none_or(|o| o.kind != KIND_DIR) {
                continue;
            }
            let mut cur = Some(x);
            let mut steps = 0;
            let mut cycle = false;
            while let Some(c) = cur {
                let parent = match p.moved.get(&c) {
                    Some(Dir::Ours(d)) => Some(*d),
                    Some(Dir::New(_)) => None,
                    None => p.o.obj(c).map(|o| o.parent),
                };
                match parent {
                    Some(d) if d == x => {
                        cycle = true;
                        break;
                    }
                    Some(d) if d != ROOT_OID && d != 0 => cur = Some(d),
                    _ => cur = None,
                }
                steps += 1;
                if steps > super::MAX_DEPTH {
                    cycle = true;
                    break;
                }
            }
            if cycle {
                p.moved.remove(&x);
                p.steps.retain(|s| !matches!(s, Step::Move { oid, .. } if *oid == x));
                let path = p.o.path(x);
                p.conflict(path, ConflictKind::Cycle, String::new());
            }
        }
    }

    /// Provede plan v zive verzi aktivniho datasetu.
    fn apply_steps(&mut self, steps: Vec<Step>, theirs: &Child, t: &Index) -> Result<()> {
        let mut new_dirs: BTreeMap<u64, u64> = BTreeMap::new();
        let resolve = |d: Dir, new_dirs: &BTreeMap<u64, u64>| -> Result<u64> {
            match d {
                Dir::Ours(o) => Ok(o),
                Dir::New(t) => new_dirs.get(&t).copied().ok_or(Error::BadState),
            }
        };
        // Pomocna slozka pro presuny (vymeny jmen).
        let stage = alloc::format!(".slucovani-{}", self.s.next_oid);
        let stage_oid = self.s.next_oid;
        let generation = self.s.txg;
        let moves: Vec<&Step> = steps.iter().filter(|s| matches!(s, Step::Move { .. })).collect();
        if !moves.is_empty() {
            self.create_at(ROOT_OID, &stage, Kind::Dir, stage_oid, generation)?;
            for s in &moves {
                if let Step::Move { oid, .. } = s {
                    let root = self.s.root;
                    let parent = self.inode(&root, *oid)?.parent;
                    let name = self.name_of(parent, *oid)?;
                    self.rename_at(parent, &name, stage_oid, &alloc::format!("{oid}"))?;
                }
            }
        }
        // Kos pred novymi objekty: uvolni jmena, ktera plan znovu pouzil.
        for s in &steps {
            if let Step::Trash { oid } = s {
                let root = self.s.root;
                let Some(raw) = btree::get(&mut self.s, &root, &Key::new(*oid, item::INODE, 0))? else { continue };
                let parent = Inode::decode(&raw, self.s.time_format())?.parent;
                let name = self.name_of(parent, *oid)?;
                let id = self.next_trash_id()?;
                self.trash_at(parent, &name, id)?;
            }
        }
        // Nove objekty (rodice pred potomky - poradi planu).
        for s in &steps {
            if let Step::Create { t_oid, dir, name } = s {
                let parent = resolve(*dir, &new_dirs)?;
                let oid = self.copy_new(theirs, *t_oid, parent, name)?;
                if t.obj(*t_oid).is_some_and(|o| o.kind == KIND_DIR) {
                    new_dirs.insert(*t_oid, oid);
                }
            }
        }
        for s in &steps {
            if let Step::Move { oid, dir, name } = s {
                let parent = resolve(*dir, &new_dirs).unwrap_or(ROOT_OID);
                self.rename_at(stage_oid, &alloc::format!("{oid}"), parent, name)?;
            }
        }
        if !moves.is_empty() {
            self.remove_at(ROOT_OID, &stage)?;
        }
        for s in &steps {
            if let Step::Content { oid, t_oid, data } = s {
                let data = match data {
                    Some(d) => d.clone(),
                    None => {
                        let i = self.inode(theirs, *t_oid)?;
                        if i.kind == KIND_SYMLINK {
                            // Odkaz nejde prepsat na miste: novy se stejnym jmenem.
                            let root = self.s.root;
                            let parent = self.inode(&root, *oid)?.parent;
                            let name = self.name_of(parent, *oid)?;
                            let id = self.next_trash_id()?;
                            self.trash_at(parent, &name, id)?;
                            self.copy_new(theirs, *t_oid, parent, &name)?;
                            continue;
                        }
                        self.read_all(theirs, *t_oid, i.size)?
                    }
                };
                self.replace_content(*oid, &data)?;
            }
        }
        Ok(())
    }

    /// Nahradi obsah souboru `oid` aktivniho datasetu.
    fn replace_content(&mut self, oid: u64, data: &[u8]) -> Result<()> {
        self.truncate_oid(oid, 0)?;
        let mut off = 0usize;
        while off < data.len() {
            let n = (data.len() - off).min(256 * BLOCK_SIZE);
            let inode = self.live_inode(oid)?;
            self.write_oid(oid, inode, off as u64, &data[off..off + n])?;
            off += n;
        }
        Ok(())
    }

    /// Kopie objektu `t_oid` ze stromu `theirs` jako novy objekt v `parent`
    /// (slozka bez obsahu - potomci jsou vlastni kroky). Vraci nove oid.
    fn copy_new(&mut self, theirs: &Child, t_oid: u64, parent: u64, name: &str) -> Result<u64> {
        let i = self.inode(theirs, t_oid)?;
        let oid = self.s.next_oid;
        let generation = self.s.txg;
        match i.kind {
            KIND_DIR => self.create_at(parent, name, Kind::Dir, oid, generation)?,
            KIND_SYMLINK => {
                let target = self.readlink_oid(theirs, t_oid)?;
                self.symlink_at(parent, name, &target, oid, generation)?;
                self.zil_op(zil::Op::Symlink {
                    parent,
                    oid,
                    generation,
                    name: crate::names::stored_name(name),
                    target,
                    time: self.now,
                    owner: self.new_owner(),
                });
            }
            _ => {
                self.create_at(parent, name, Kind::File, oid, generation)?;
                self.copy_content(theirs, t_oid, oid)?;
            }
        }
        Ok(oid)
    }

    /// Koren snapshotu (dataset, snapshot) na disku.
    fn snapshot_root_at(&mut self, ds: u64, snap: u64) -> Result<Child> {
        let back = self.cur_ds;
        self.activate(ds)?;
        let root = self.s.root;
        let r = btree::get(&mut self.s, &root, &Key::new(META_OID, item::SNAPSHOT, snap));
        self.activate(back)?;
        let raw = r?.ok_or(Error::NotFound)?;
        Ok(Child::Disk(crate::layout::SnapshotRecord::decode(&raw)?.root))
    }

    /// Merge vetve: slouci posledni commit datasetu `source` do zive verze
    /// datasetu `target` (trojcestne proti spolecnemu predkovi v DAG
    /// commitu). Bez konfliktu hned commit slouceni (dva rodice); s konflikty
    /// zustanou zmeny v zive verzi se znackami nebo kopiemi vedle a pristi
    /// `commit_version` dostane druheho rodice. Vyzaduje `MANAGE` k obema.
    pub fn merge(&mut self, target: &str, source: &str) -> Result<MergeReport> {
        self.require(target, rights::MANAGE)?;
        self.require(source, rights::MANAGE)?;
        self.enter_dataset(source)?;
        let theirs_head = self.head_active()?.head.ok_or(Error::BadState)?;
        let label = String::from(source.rsplit('/').next().unwrap_or(source));
        self.enter_dataset(target)?;
        let tds = self.cur_ds;
        let head = self.head_active()?;
        let ours_head = head.head.ok_or(Error::BadState)?;
        if ours_head.id == theirs_head.id {
            return Ok(MergeReport::default());
        }
        let base = self.merge_base(&ours_head, &theirs_head)?.ok_or(Error::NotFound)?;
        if base.id == theirs_head.id {
            // Vetev nema nic noveho.
            return Ok(MergeReport::default());
        }
        self.guarded(|fs| {
            fs.commit()?;
            let base_root = fs.snapshot_root_at(base.dataset, base.snapshot)?;
            let theirs_root = fs.snapshot_root_at(theirs_head.dataset, theirs_head.snapshot)?;
            fs.activate(tds)?;
            let mut report = fs.merge_trees(&base_root, &theirs_root, Mode::Merge, &label, None)?;
            fs.activate(tds)?;
            let mut h = fs.head_active()?;
            h.merging = Some(theirs_head);
            fs.put_head(h)?;
            if report.conflicts.is_empty() {
                report.commit = Some(fs.commit_active(&alloc::format!("Slouceni vetve {label}"))?);
            } else {
                fs.commit()?;
            }
            Ok(report)
        })
    }

    /// Zrusi rozpracovany merge (pristi commit bude mit jednoho rodice).
    /// Zmeny v zive verzi zustanou.
    pub fn merge_abort(&mut self, target: &str) -> Result<()> {
        self.require(target, rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(target)?;
            let mut h = fs.head_active()?;
            h.merging = None;
            fs.put_head(h)?;
            fs.commit()
        })
    }

    /// "Zkusit zmeny bokem": snapshot datasetu `dataset` a z nej
    /// zapisovatelny klon na ceste `path` (pracovni kopie - aplikace dostane
    /// capability jen k ni). Vraci cislo datasetu klonu. Vyzaduje `MANAGE`.
    pub fn aside_begin(&mut self, dataset: &str, path: &str) -> Result<u64> {
        Ok(self.aside_begin_ex(dataset, path)?.dataset)
    }

    /// Jako `aside_begin`, navic jmeno snapshotu puvodu a posledni zaznam
    /// zurnalu, ktery obsahuje (shell z nej dela undo misto `view`, viz
    /// `aside_apply_with`).
    pub fn aside_begin_ex(&mut self, dataset: &str, path: &str) -> Result<AsideBegun> {
        self.require(dataset, rights::MANAGE)?;
        // Obsazeny cil predem: selhani az uprostred vytvareni klonu by po
        // zmenach (snapshot, pocet klonu) system zablokovalo (`guarded`).
        match self.enter(path) {
            Err(Error::NotFound) => {}
            Ok(_) | Err(Error::NotSupported) => return Err(Error::AlreadyExists),
            Err(e) => return Err(e),
        }
        // Dva commity: stav (koren snapshotu) a pak zaznam snapshotu
        // spolu s klonem (commit klonu). Pad mezi nimi = nic nevzniklo.
        let name = self.guarded(|fs| {
            // Vnorene datasety (i jine klony "bokem" uvnitr originalu) jsou
            // v klonu hranice: videt, ale nejde do nich (`dataset.rs`).
            fs.enter_dataset(dataset)?;
            let name = alloc::format!("aside-{}", fs.s.next_snapshot);
            let (_, meta) = fs.snapshot_temporary(&name, snapkind::ASIDE)?;
            Ok((name, meta.journal_seq))
        })?;
        let (name, seq) = name;
        self.clone_snapshot(dataset, &name, path).map(|id| AsideBegun { dataset: id, snapshot: name.clone(), seq }).inspect_err(|_| {
            // Klon nevznikl (cil existuje, malo mista): snapshot by jinak
            // drzel bloky az do uklidu pri pripojeni.
            if !self.s.poisoned && self.enter_dataset(dataset).is_ok() {
                let _ = self.guarded(|fs| fs.delete_snapshot_active(&name));
            }
        })
    }

    /// Puvod klonu `clone` "bokem": (dataset puvodu, snapshot) a cesta puvodu.
    fn aside_origin(&mut self, clone: &str) -> Result<(u64, u64, String)> {
        self.require(clone, rights::MANAGE)?;
        self.enter_dataset(clone)?;
        let (ods, osnap, _) = self.cur_origin;
        if ods == 0 {
            return Err(Error::InvalidArgs);
        }
        self.activate(ods)?;
        // VIEW: snapshot po prevzeti ponechany jako pohled (`keep_view`).
        if !matches!(self.snap_meta(osnap)?.kind, snapkind::ASIDE | snapkind::VIEW) {
            return Err(Error::InvalidArgs);
        }
        let path = self.dataset_path(ods)?;
        self.require(&path, rights::MANAGE)?;
        Ok((ods, osnap, path))
    }

    /// Cesta originalu klonu "bokem" (pro kontrolu capability volajiciho:
    /// sluzba overi, ze relace smi menit i original, nejen klon).
    pub fn aside_original(&mut self, clone: &str) -> Result<String> {
        Ok(self.aside_origin(clone)?.2)
    }

    /// Prevezme zmeny klonu "bokem" do originalu: vse (`select = None`)
    /// nebo jen cesty v `select` (uvnitr datasetu, vcetne podstromu).
    /// Trojcestne proti snapshotu, ze ktereho klon vznikl; co se mezitim
    /// zmenilo i v originalu, se neprevezme a vrati jako konflikt.
    /// Prevzeti je atomicke (commit). Vyzaduje `MANAGE` k obema.
    pub fn aside_apply(&mut self, clone: &str, select: Option<&[String]>) -> Result<MergeReport> {
        self.aside_apply_with(clone, select, false)
    }

    /// Jako `aside_apply`; `keep_view` = snapshot puvodu (stav pred
    /// radkem) po prevzeti zustane jako pohled (`view`) pro undo: uvolni
    /// ho `release_view`, `aside_discard` ho uz nesmaze. Pro cesty, ktere se
    /// prevzaly bez konfliktu, je stav ve snapshotu stejny jako v originalu
    /// tesne pred prevzetim (jinak by sly do konfliktu) - pohled navic
    /// (dalsi commit na radek) neni potreba.
    pub fn aside_apply_with(&mut self, clone: &str, select: Option<&[String]>, keep_view: bool) -> Result<MergeReport> {
        let (ods, osnap, _) = self.aside_origin(clone)?;
        self.enter_dataset(clone)?;
        let cds = self.cur_ds;
        self.guarded(|fs| {
            // Koreny na disk (zivy klon i original), pak cist jen z disku.
            fs.commit()?;
            fs.activate(cds)?;
            let theirs = fs.s.root;
            let base = fs.snapshot_root_at(ods, osnap)?;
            fs.activate(ods)?;
            let report = fs.merge_trees(&base, &theirs, Mode::Aside, "bokem", select)?;
            if keep_view {
                fs.activate(ods)?;
                let mut meta = fs.snap_meta(osnap)?;
                meta.kind = snapkind::VIEW;
                fs.put(Key::new(META_OID, item::SNAPMETA, osnap), meta.encode())?;
            }
            fs.commit()?;
            Ok(report)
        })
    }

    /// Uklid po padu (pri pripojeni): snapshot "bokem" bez klonu. Zacatek
    /// (snapshot, pak klon) i zahozeni (klon, pak snapshot) jsou dva
    /// commity; pad mezi nimi necha snapshot, ktery uz nikdo nepouzije
    /// (klon vznika jen v `aside_begin`), ale drzel by bloky. Klon sam
    /// vznika i zanika jednim commitem - po padu je cely, nebo neni.
    pub(super) fn cleanup_asides(&mut self) -> Result<()> {
        let mut changed = false;
        for id in self.all_dataset_ids()? {
            self.activate(id)?;
            for (sid, rec) in self.records_active()? {
                if rec.clones == 0 && self.snap_meta(sid)?.kind == snapkind::ASIDE {
                    self.destroy_snapshot(sid)?;
                    changed = true;
                }
            }
        }
        self.activate(crate::layout::ROOT_DATASET)?;
        if changed {
            self.commit()?;
        }
        Ok(())
    }

    /// Testy: jen snapshot "bokem" bez klonu (stav po padu v `aside_begin`).
    #[cfg(test)]
    pub(crate) fn aside_snapshot_only(&mut self, dataset: &str) -> Result<()> {
        self.enter_dataset(dataset)?;
        let name = alloc::format!("aside-{}", self.s.next_snapshot);
        self.snapshot_internal(&name, snapkind::ASIDE)?;
        Ok(())
    }

    /// Zahodi klon "bokem" i jeho vychozi snapshot (jednim krokem).
    /// Snapshot ponechany jako pohled (`aside_apply_with`) zustane do
    /// `release_view` (uz uvolneny, skryty zanikne s klonem).
    pub fn aside_discard(&mut self, clone: &str) -> Result<()> {
        let (_, osnap, path) = self.aside_origin(clone)?;
        let root = self.s.root;
        let raw = btree::get(&mut self.s, &root, &Key::new(META_OID, item::SNAPSHOT, osnap))?.ok_or(Error::Corrupted)?;
        let name = crate::layout::SnapshotRecord::decode(&raw)?.name;
        let aside = self.snap_meta(osnap)?.kind == snapkind::ASIDE;
        self.delete_dataset(clone)?;
        if aside {
            self.delete_snapshot_of(&path, &name)?;
        }
        self.commit()
    }

}

/// Zacatek "bokem" (`aside_begin_ex`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsideBegun {
    /// Cislo datasetu klonu.
    pub dataset: u64,
    /// Snapshot puvodu (v puvodnim datasetu).
    pub snapshot: String,
    /// Posledni zaznam zurnalu, ktery snapshot obsahuje (jako `View::seq`).
    pub seq: u64,
}

/// Jmeno kopie konfliktu: "stem (konflikt label).ext".
fn format_conflict(name: &str, label: &str) -> String {
    let (stem, ext) = split_ext(name);
    alloc::format!("{stem} (konflikt {label}){ext}")
}
