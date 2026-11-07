//! Rozhrani k prostredi: souborovy system, programy, cas, vystup, dotazy.
//!
//! Interpret nic z toho nedela sam. V systemu ho dodava sluzba relaci
//! (`user/termd`: FS pres `fsproto`, programy z initfs), v testech
//! [`crate::vfs::mem::MemFs`] a testovaci hostitel.

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::ShError;
use crate::value::Value;

pub type FsResult<T> = Result<T, abi::Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
}

impl NodeKind {
    pub fn name(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Dir => "dir",
            NodeKind::Symlink => "symlink",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub kind: NodeKind,
    pub size: u64,
    /// Cas zmeny v ns od 1970 (UTC), pokud ho FS zna.
    pub mtime: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub meta: Meta,
}

/// Souborovy system tak, jak ho vidi shell. Cesty jsou absolutni
/// a normalizovane (`/a/b`), bez `~` a `..` (to resi interpret).
pub trait Fs {
    fn stat(&mut self, path: &str) -> FsResult<Meta>;
    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>>;
    /// Vypis jen se jmeny a druhy (velikost 0, cas neznamy). FS, kde
    /// metadata stoji dalsi dotaz na kazdou polozku, ho umi levneji.
    fn list_names(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        self.list(path)
    }
    fn read(&mut self, path: &str) -> FsResult<Vec<u8>>;
    /// Vytvori nebo prepise soubor (nadrazena slozka musi existovat).
    fn write(&mut self, path: &str, data: &[u8]) -> FsResult<()>;
    fn mkdir(&mut self, path: &str) -> FsResult<()>;
    /// Smaze soubor nebo prazdnou slozku.
    fn remove(&mut self, path: &str) -> FsResult<()>;
    fn rename(&mut self, from: &str, to: &str) -> FsResult<()>;
    /// (celkem, volne) bajtu.
    fn space(&mut self) -> FsResult<(u64, u64)> {
        Err(abi::Error::NotSupported)
    }

    // ---- Verze (FS v3): datasety, klony bokem, pohledy ----
    //
    // Vsechny cesty jsou absolutni v jmennem prostoru FS (ne uvnitr
    // datasetu); prevod resi implementace.

    /// Koren datasetu, ve kterem `path` lezi. FS bez datasetu: "/".
    fn dataset_of(&mut self, _path: &str) -> FsResult<String> {
        Ok(String::from("/"))
    }
    /// Zapisovatelny klon datasetu "bokem" (zmeny nanecisto). Misto klonu
    /// vybere implementace. Vnorene datasety jsou v klonu hranice (zapis
    /// do nich selze).
    fn aside_begin(&mut self, _dataset: &str) -> FsResult<Aside> {
        Err(abi::Error::NotSupported)
    }
    /// Co klon zmenil proti puvodu (cesty v puvodu).
    fn aside_changes(&mut self, _aside: &Aside) -> FsResult<Vec<FsChange>> {
        Err(abi::Error::NotSupported)
    }
    /// Prevezme zmeny klonu do puvodu (atomicky). Vrati cesty v konfliktu
    /// (puvod se mezitim zmenil i jinak) - ty se neprevzaly.
    fn aside_apply(&mut self, _aside: &Aside) -> FsResult<Applied> {
        Err(abi::Error::NotSupported)
    }
    /// Zahodi klon (i po `aside_apply`).
    fn aside_discard(&mut self, _aside: &Aside) -> FsResult<()> {
        Err(abi::Error::NotSupported)
    }
    /// Pohled: levny docasny snapshot datasetu a pozice v zurnalu zmen.
    fn view(&mut self, _dataset: &str) -> FsResult<ViewRef> {
        Err(abi::Error::NotSupported)
    }
    fn release_view(&mut self, _view: &ViewRef) -> FsResult<()> {
        Err(abi::Error::NotSupported)
    }
    /// Co se v datasetu zmenilo od pohledu. Nic nezmeneno (podle zurnalu)
    /// = prazdny seznam bez pruchodu stromem.
    fn changed_since(&mut self, _view: &ViewRef) -> FsResult<Vec<FsChange>> {
        Err(abi::Error::NotSupported)
    }
    fn view_stat(&mut self, _view: &ViewRef, _path: &str) -> FsResult<Meta> {
        Err(abi::Error::NotSupported)
    }
    fn view_list(&mut self, _view: &ViewRef, _path: &str) -> FsResult<Vec<DirEntry>> {
        Err(abi::Error::NotSupported)
    }
    fn view_read(&mut self, _view: &ViewRef, _path: &str) -> FsResult<Vec<u8>> {
        Err(abi::Error::NotSupported)
    }
}

/// Klon datasetu "bokem".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aside {
    /// Koren puvodniho datasetu.
    pub dataset: String,
    /// Kde klon lezi (absolutni cesta).
    pub root: String,
}

/// Vysledek prevzeti klonu.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Cesty v konfliktu (neprevzaly se).
    pub conflicts: Vec<String>,
    /// `false` = FS seznam konfliktu zkratil (jich je vic): neni jiste,
    /// co se prevzalo, undo takoveho prevzeti nejde.
    pub complete: bool,
}

/// Pohled na dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewRef {
    pub dataset: String,
    pub snapshot: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsChangeKind {
    Added,
    Removed,
    Modified,
    /// Presun (obsah se mohl zmenit taky); puvodni cesta v `old_path`.
    Moved,
}

/// Zmena ve FS (z klonu nebo od pohledu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsChange {
    pub kind: FsChangeKind,
    pub path: String,
    pub old_path: Option<String>,
    pub is_dir: bool,
}

/// Prostredi interpretu.
pub trait Host {
    fn fs(&mut self) -> &mut dyn Fs;
    /// Vystup prikazu (hodnota celeho prikazu na nejvyssi urovni).
    fn emit(&mut self, value: Value);
    /// Poznamka do terminalu (`log`, varovani).
    fn note(&mut self, text: &str);
    /// Aktualni cas (ns od 1970 UTC), pokud je znamy.
    fn now(&mut self) -> Option<i64>;
    fn sleep(&mut self, ns: i64);
    /// Stiskl uzivatel Ctrl-C? (Interpret se pta pravidelne.)
    fn interrupted(&mut self) -> bool;
    /// Dotaz s potvrzenim (nahled zmen, opravneni skriptu). `detail` je
    /// tabulka zmen nebo popis naroku.
    fn confirm(&mut self, question: &str, detail: &Value) -> bool;
    /// Spusti program. `None` = takovy program neni.
    fn exec(&mut self, _name: &str, _args: &[(Option<String>, Value)], _input: Value, _needs: &ExecNeeds)
        -> Option<Result<Value, ShError>> {
        None
    }
    /// Domovska slozka uzivatele.
    fn home(&self) -> String {
        String::from("/home")
    }
}

/// Co smi spusteny program (z manifestu programu a omezeni skriptu).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExecNeeds {
    /// Cesty pro cteni (absolutni). Prazdne a `all = true` = vse.
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub net: bool,
    /// Bez omezeni (interaktivni relace bez `needs`).
    pub all: bool,
    /// Program bezi nanecisto (`preview`): (koren datasetu, koren klonu).
    /// Cesty uvnitr datasetu se programu pripoji z klonu (pod puvodnim
    /// jmenem). Zapis mimo klony se mu pripoji jen pro cteni - skutecny FS
    /// v nahledu menit nesmi.
    pub aside: Vec<(String, String)>,
}
