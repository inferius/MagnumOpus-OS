//! Koncept radku: zmeny nanecisto v klonech FS ("bokem").
//!
//! Prvni zapis radku do datasetu zalozi jeho klon (`aside_begin`); dalsi
//! zapisy i cteni toho datasetu jdou do klonu. Na konci radku shell
//! z klonu zjisti diff (`aside_changes`): male zmeny prevezme hned
//! (`aside_apply`, s undo), velke (zamek nebezpecnych operaci) az po
//! potvrzeni; odmitnuti = `aside_discard`. Radek, ktery jen cte, klon
//! nezaklada a nestoji nic navic.
//!
//! `preview { }` prida vrstvu: klony klonu. Potvrzeni prevezme vrstvu do
//! vrstvy pod ni, odmitnuti ji zahodi. Program spusteny v nahledu dostane
//! strom z klonu (viz [`Draft::exec_map`]).
//!
//! Data jsou ve FS (na disku), ne v pameti shellu. Vnorene datasety jsou
//! v klonu hranice; zapis do vnoreneho datasetu dostane vlastni klon.
//!
//! Cteni nevola `dataset_of` (dotaz na fsd): vezme klon s nejdelsim
//! korenem nad cestou. Kdyz cesta lezi ve vnorenem datasetu (v klonu
//! hranice, FS vrati `NOT_SUPPORTED`), zopakuje se s presnym urcenim
//! datasetu. Zahozeni klonu (dlouhe uvolneni bloku) se odklada do chvile,
//! kdy je shell volny ([`Draft::flush`]).

use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

use super::diff::{ChangeKind, DiffEntry};
use super::path::{join, key, rebase, within};
use super::undo::remove_all;
use crate::host::{Applied, Aside, DirEntry, Fs, FsChangeKind, FsResult, Meta, NodeKind};

#[derive(Debug, Default)]
pub struct Draft {
    /// Vrstva 0 = radek, dalsi = vnorene `preview`. Klony vrstvy `i` jsou
    /// klony datasetu ve jmennem prostoru vrstvy `i - 1` (vrstva 0 ze
    /// skutecneho FS).
    layers: Vec<Vec<Aside>>,
    /// Klony k zahozeni (po prevzeti nebo odmitnuti), az bude shell volny.
    trash: Vec<Aside>,
    /// Zalozeni klonu selhalo (rada u chyby zapisu).
    pub aside_failed: bool,
}

/// Kolik klonu smi cekat na zahozeni (pak se zahodi hned).
const TRASH_MAX: usize = 16;

impl Draft {
    pub fn new() -> Self {
        Self { layers: alloc::vec![Vec::new()], trash: Vec::new(), aside_failed: false }
    }

    /// Zadny klon (radek zatim nic nezapsal).
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| l.is_empty())
    }

    /// Hloubka vnoreni `preview` (0 = jen radek).
    pub fn depth(&self) -> usize {
        self.layers.len() - 1
    }

    /// Nova vrstva (zacatek `preview`).
    pub fn push(&mut self) {
        self.layers.push(Vec::new());
    }

    /// Koreny datasetu se zmenami ve vrstve 0 (skutecne cesty).
    pub fn datasets(&self) -> Vec<String> {
        self.layers[0].iter().map(|a| a.dataset.clone()).collect()
    }

    /// Skutecna cesta -> cesta v nejvyssi vrstve. `create` = chybejici
    /// klony zalozit (zapis); jinak cesta zustane, kde klon neni.
    fn resolve(&mut self, fs: &mut dyn Fs, real: &str, create: bool) -> FsResult<String> {
        let mut cur = String::from(real);
        if !create && self.is_empty() {
            return Ok(cur);
        }
        for li in 0..self.layers.len() {
            if !create && self.layers[li].is_empty() {
                continue;
            }
            let ds = dataset_of(fs, &cur)?;
            let found = self.layers[li].iter().find(|a| key(&a.dataset) == key(&ds)).map(|a| a.root.clone());
            let root = match found {
                Some(r) => r,
                None if create => {
                    let a = fs.aside_begin(&ds).inspect_err(|_| self.aside_failed = true)?;
                    let r = a.root.clone();
                    self.layers[li].push(a);
                    r
                }
                None => continue,
            };
            cur = rebase(&cur, &ds, &root);
        }
        Ok(cur)
    }

    /// Cteni: cesta v nejvyssi vrstve bez dotazu na FS (klon s nejdelsim
    /// korenem nad cestou v kazde vrstve).
    fn resolve_fast(&self, real: &str) -> String {
        let mut cur = String::from(real);
        for layer in &self.layers {
            if let Some(a) = layer.iter().filter(|a| within(&cur, &a.dataset)).max_by_key(|a| a.dataset.len()) {
                cur = rebase(&cur, &a.dataset, &a.root);
            }
        }
        cur
    }

    /// Cteci operace: rychla cesta, pri hranici vnoreneho datasetu presne.
    fn read_op<T>(&mut self, fs: &mut dyn Fs, path: &str, op: impl Fn(&mut dyn Fs, &str) -> FsResult<T>) -> FsResult<T> {
        if self.is_empty() {
            return op(fs, path);
        }
        match op(fs, &self.resolve_fast(path)) {
            Err(Error::NotSupported) => {
                let p = self.resolve(fs, path, false)?;
                op(fs, &p)
            }
            r => r,
        }
    }

    /// Cesta ve vrstve `layer` -> skutecna cesta.
    fn unresolve(&self, layer: usize, p: &str) -> String {
        let mut cur = String::from(p);
        for j in (0..layer).rev() {
            if let Some(a) = self.layers[j].iter().find(|a| within(&cur, &a.root)) {
                cur = rebase(&cur, &a.root, &a.dataset);
            }
        }
        cur
    }

    /// Lezi dve skutecne cesty ve stejnem datasetu (presun jde jednim krokem)?
    fn same_dataset(fs: &mut dyn Fs, a: &str, b: &str) -> FsResult<bool> {
        Ok(key(&dataset_of(fs, a)?) == key(&dataset_of(fs, b)?))
    }

    pub fn stat(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<Meta> {
        self.read_op(fs, path, |fs, p| fs.stat(p))
    }

    pub fn list(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<Vec<DirEntry>> {
        self.read_op(fs, path, |fs, p| fs.list(p))
    }

    pub fn list_names(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<Vec<DirEntry>> {
        self.read_op(fs, path, |fs, p| fs.list_names(p))
    }

    pub fn read(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<Vec<u8>> {
        self.read_op(fs, path, |fs, p| fs.read(p))
    }

    pub fn write(&mut self, fs: &mut dyn Fs, path: &str, data: &[u8]) -> FsResult<()> {
        let p = self.resolve(fs, path, true)?;
        fs.write(&p, data)
    }

    pub fn mkdir(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<()> {
        let p = self.resolve(fs, path, true)?;
        fs.mkdir(&p)
    }

    pub fn remove(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<()> {
        if path == "/" {
            return Err(Error::AccessDenied);
        }
        let p = self.resolve(fs, path, true)?;
        fs.remove(&p)
    }

    /// Smaze i slozku s obsahem.
    pub fn remove_all(&mut self, fs: &mut dyn Fs, path: &str) -> FsResult<()> {
        if path == "/" {
            return Err(Error::AccessDenied);
        }
        let p = self.resolve(fs, path, true)?;
        remove_all(fs, &p)
    }

    pub fn copy(&mut self, fs: &mut dyn Fs, from: &str, to: &str) -> FsResult<()> {
        // Sam do sebe (ani do sve podslozky) kopirovat nejde.
        if within(to, from) {
            return Err(Error::InvalidArgs);
        }
        // Nejdriv cil: zalozeni klonu muze zmenit, kde lezi zdroj.
        let dst = self.resolve(fs, to, true)?;
        let src = self.resolve(fs, from, false)?;
        copy_tree(fs, &src, &dst)
    }

    pub fn rename(&mut self, fs: &mut dyn Fs, from: &str, to: &str) -> FsResult<()> {
        if self.stat(fs, to).is_ok() {
            return Err(Error::AlreadyExists);
        }
        if within(to, from) {
            return Err(Error::InvalidArgs);
        }
        if Self::same_dataset(fs, from, to)? {
            let src = self.resolve(fs, from, true)?;
            let dst = self.resolve(fs, to, true)?;
            return fs.rename(&src, &dst);
        }
        // Mezi datasety: kopie a smazani.
        self.copy(fs, from, to)?;
        self.remove_all(fs, from)
    }

    /// Zmeny nejvyssi vrstvy proti vrstve pod ni (skutecne cesty).
    pub fn changes(&mut self, fs: &mut dyn Fs) -> FsResult<Vec<DiffEntry>> {
        let top = self.layers.len() - 1;
        let mut out: Vec<DiffEntry> = Vec::new();
        for a in self.layers[top].clone() {
            for c in fs.aside_changes(&a)? {
                let lower = c.path.clone();
                let in_clone = rebase(&lower, &a.dataset, &a.root);
                let before = fs.stat(&lower).ok();
                let after = fs.stat(&in_clone).ok();
                let size = |m: &Option<Meta>| m.filter(|m| m.kind != NodeKind::Dir).map(|m| m.size);
                let change = match c.kind {
                    FsChangeKind::Added if c.is_dir => ChangeKind::DirAdded,
                    FsChangeKind::Added => ChangeKind::Added,
                    FsChangeKind::Removed if c.is_dir => ChangeKind::DirRemoved,
                    FsChangeKind::Removed => ChangeKind::Removed,
                    FsChangeKind::Modified => ChangeKind::Modified,
                    FsChangeKind::Moved => ChangeKind::Moved,
                };
                let from = c.old_path.as_deref().map(|p| self.unresolve(top, p));
                let entry = DiffEntry {
                    path: self.unresolve(top, &lower),
                    change,
                    from,
                    before: if change == ChangeKind::Moved { None } else { size(&before) },
                    after: size(&after),
                };
                // Smazani a pridani na stejne ceste = nahrazeni (soubor <-> slozka).
                if entry.from.is_none()
                    && let Some(prev) = out.iter_mut().find(|d| key(&d.path) == key(&entry.path) && d.from.is_none())
                {
                    let removed = matches!(prev.change, ChangeKind::Removed | ChangeKind::DirRemoved);
                    let added = matches!(entry.change, ChangeKind::Added | ChangeKind::DirAdded);
                    if removed || added {
                        prev.before = prev.before.or(entry.before);
                        prev.after = entry.after.or(prev.after);
                        prev.change = ChangeKind::Replaced;
                        continue;
                    }
                }
                out.push(entry);
            }
        }
        Ok(super::diff::normalize(out))
    }

    /// Prevezme nejvyssi vrstvu do vrstvy pod ni (vrstva 0 do FS); jeji
    /// klony jdou k zahozeni. Vrati skutecne cesty v konfliktu (neprevzaly
    /// se) a zda je seznam konfliktu uplny.
    pub fn apply_top(&mut self, fs: &mut dyn Fs) -> FsResult<Applied> {
        let top = self.layers.len() - 1;
        let clones = core::mem::take(&mut self.layers[top]);
        let mut out = Applied { conflicts: Vec::new(), complete: true };
        let mut err = None;
        for a in &clones {
            if err.is_none() {
                match fs.aside_apply(a) {
                    Ok(r) => {
                        out.conflicts.extend(r.conflicts.iter().map(|p| self.unresolve(top, p)));
                        out.complete &= r.complete;
                    }
                    Err(e) => err = Some(e),
                }
            }
        }
        self.to_trash(fs, clones);
        if top > 0 {
            self.layers.pop();
        }
        match err {
            Some(e) => Err(e),
            None => Ok(out),
        }
    }

    fn to_trash(&mut self, fs: &mut dyn Fs, clones: Vec<Aside>) {
        self.trash.extend(clones);
        if self.trash.len() > TRASH_MAX {
            self.flush(fs);
        }
    }

    /// Zahodi nejvyssi vrstvu (odmitnuti nahledu nebo zmen radku).
    pub fn discard_top(&mut self, fs: &mut dyn Fs) {
        let top = self.layers.len() - 1;
        let clones = core::mem::take(&mut self.layers[top]);
        self.to_trash(fs, clones);
        if top > 0 {
            self.layers.pop();
        }
    }

    /// Zahodi vse (konec radku s chybou pred prevzetim, preruseni).
    pub fn discard_all(&mut self, fs: &mut dyn Fs) {
        while self.layers.len() > 1 {
            self.discard_top(fs);
        }
        self.discard_top(fs);
    }

    /// Opravdu zahodi klony, ktere cekaji (shell je volny).
    pub fn flush(&mut self, fs: &mut dyn Fs) {
        for a in core::mem::take(&mut self.trash) {
            let _ = fs.aside_discard(&a);
        }
    }

    /// Ceka nejaky klon na zahozeni?
    pub fn has_trash(&self) -> bool {
        !self.trash.is_empty()
    }

    /// Program v nahledu: zalozi klony pro datasety, kam smi zapisovat
    /// (`trees`), a vrati (skutecny koren datasetu, koren klonu v nejvyssi
    /// vrstve) pro pripojeni jeho stromu.
    pub fn exec_map(&mut self, fs: &mut dyn Fs, trees: &[String]) -> FsResult<Vec<(String, String)>> {
        for t in trees {
            self.resolve(fs, t, true)?;
        }
        let mut out = Vec::new();
        for ds in self.datasets() {
            let root = self.resolve(fs, &ds, false)?;
            out.push((ds, root));
        }
        // Vnorene datasety pred nadrazenymi (pripojuje se od nejdelsi cesty).
        out.sort_by_key(|(d, _)| core::cmp::Reverse(d.len()));
        Ok(out)
    }
}

/// Dataset cesty; cesta, ktera jeste neexistuje (novy soubor, `mkdir`),
/// patri datasetu nejblizsi existujici nadrazene slozky.
pub fn dataset_of(fs: &mut dyn Fs, path: &str) -> FsResult<String> {
    let mut p = path;
    loop {
        match fs.dataset_of(p) {
            Err(Error::NotFound) => match super::path::parent(p) {
                Some(up) => p = up,
                None => return Err(Error::NotFound),
            },
            r => return r,
        }
    }
}

/// Zkopiruje soubor nebo slozku i s obsahem (cesty uz v jedne vrstve).
fn copy_tree(fs: &mut dyn Fs, src: &str, dst: &str) -> FsResult<()> {
    let m = fs.stat(src)?;
    if m.kind == NodeKind::Dir {
        fs.mkdir(dst)?;
        for e in fs.list_names(src)? {
            copy_tree(fs, &join(src, &e.name), &join(dst, &e.name))?;
        }
        return Ok(());
    }
    let data = fs.read(src)?;
    fs.write(dst, &data)
}
