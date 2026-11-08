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
    /// Prava relace k polozce ([`access`]); `None` = FS je nehlasi.
    pub access: Option<u8>,
    /// Slozka je koren vnoreneho datasetu (vlastni historie).
    pub dataset: bool,
    /// U slozky obsah podstromu (soubory, slozky bez sebe), kdyz ho FS
    /// zna; `size` je pak logicka velikost podstromu (soucet souboru).
    pub tree: Option<(u64, u64)>,
}

impl Meta {
    /// Polozka jen s druhem a velikosti (bez casu a prav).
    pub const fn basic(kind: NodeKind, size: u64) -> Self {
        Self { kind, size, mtime: None, access: None, dataset: false, tree: None }
    }
}

/// Prava relace k polozce (bity `Meta::access`).
pub mod access {
    pub const READ: u8 = 1;
    pub const WRITE: u8 = 2;
    pub const EXEC: u8 = 4;

    /// Text `rwx` (pomlcka = chybi).
    pub fn text(a: u8) -> alloc::string::String {
        let mut s = alloc::string::String::new();
        s.push(if a & READ != 0 { 'r' } else { '-' });
        s.push(if a & WRITE != 0 { 'w' } else { '-' });
        s.push(if a & EXEC != 0 { 'x' } else { '-' });
        s
    }
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
    /// Cast souboru: nejvys `len` bajtu od `offset` (za koncem prazdne).
    /// Vychozi implementace cte cely soubor; FS, ktery umi cist po
    /// castech, ji prepise (velke soubory, napr. `audio play`).
    fn read_range(&mut self, path: &str, offset: u64, len: usize) -> FsResult<Vec<u8>> {
        let all = self.read(path)?;
        let start = (offset.min(all.len() as u64)) as usize;
        let end = start.saturating_add(len).min(all.len());
        Ok(all[start..end].to_vec())
    }
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

/// Prikaz `tasks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TasksOp {
    List,
    Cancel(u64),
    Pause(u64),
    Resume(u64),
    /// Poslednich N radku vystupu ulohy.
    Log(u64, u32),
}

/// Uloha `async` pro hostitele: kod v zapisu B (formatovac), kopie
/// zachycenych promennych a prava volajiciho.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskSpec {
    pub code: String,
    pub vars: Vec<(String, Value)>,
    pub needs: Option<TaskNeeds>,
    pub cwd: String,
    /// Popis pro blok v terminalu.
    pub label: String,
    /// Zamek nebezpecnych operaci volajiciho (kolik zmen bez potvrzeni).
    pub lock_threshold: usize,
}

/// Prava ulohy (manifest `needs` volajiciho; `None` = bez omezeni).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskNeeds {
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub net: bool,
}

/// Dokoncena uloha: vysledek a zaznamy undo jejich zmen souboru (prevezme
/// je relace, `undo` je vrati).
#[derive(Debug, Clone)]
pub struct TaskDone {
    pub id: u64,
    pub result: Result<Value, ShError>,
    pub undo: Vec<crate::vfs::undo::UndoEntry>,
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

/// Pozadavek prikazu `audio` na zvukovou sluzbu (v systemu `audiod`).
///
/// Prehravani jde po kouscich: `Open`, nekolik `Write` (kazdy ceka na
/// misto v bufferu, nejvys desetiny sekundy), `Finish` (pocka na dohrani)
/// nebo `Abort` (Ctrl-C). Interpret mezi kousky kontroluje preruseni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioReq {
    /// Novy prehravany stream: frekvence, kanaly, typ vzorku
    /// (`audioproto::Sample` jako cislo).
    Open { rate: u32, channels: u8, sample: u8 },
    /// Dalsi data (cele snimky formatu z `Open`).
    Write { data: Vec<u8> },
    /// Dohrat a zavrit; vysledek = zaznam s delkou a podtecenimi.
    Finish,
    /// Zastavit hned a zavrit (preruseni).
    Abort,
    /// Hlavni hlasitost v % (`None` = jen zjistit).
    Volume(Option<u8>),
    Mute(bool),
    /// Stav zarizeni a sluzby.
    Status,
    /// Seznam zvukovych zarizeni.
    Devices,
    /// Vybrat zarizeni (index ze seznamu).
    Select(u8),
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
    /// Prubeh dlouhe operace (`done` z `total`; `total = 0` = neznamy).
    /// Hostitel ho ukaze na miste (jeden pruh na `id` v bloku); stejne `id`
    /// = aktualizace. Rychlost a odhad si spocita okno.
    fn progress(&mut self, _id: u32, _done: u64, _total: u64, _label: &str) {}
    /// Vyber ze seznamu (vyzva s volbami). `None` = zruseno / bez okna.
    fn choose(&mut self, _question: &str, _choices: &[String]) -> Option<usize> {
        None
    }
    /// Spusti ulohu `async` soubezne (vlastni vlakno, interpret, koncept
    /// a blok). `None` = hostitel soubeznost neumi, uloha probehne hned.
    fn task_spawn(&mut self, _spec: TaskSpec) -> Option<Result<u64, ShError>> {
        None
    }
    /// Pocka na ulohy: `any` = staci jedna hotova, jinak vsechny. Vrati
    /// hotove (poradi dokonceni). Termin `deadline` (ns od 1970) uplynul =
    /// chyba `TIMED_OUT` (ulohy bezi dal). Termin v minulosti = jen
    /// vyzvednout hotove bez cekani.
    fn task_wait(&mut self, _ids: &[u64], _any: bool, _deadline: Option<i64>) -> Result<Vec<TaskDone>, ShError> {
        Err(ShError::sys(abi::Error::NotSupported, "ulohy"))
    }
    /// Zrusi ulohu. Kdyz uz dobehla, vrati jeji vysledek (a undo).
    fn task_cancel(&mut self, _id: u64) -> Option<TaskDone> {
        None
    }
    /// Zvuk (prikaz `audio`). `None` = hostitel zvuk nema.
    fn audio(&mut self, _req: AudioReq) -> Option<Result<Value, ShError>> {
        None
    }
    /// Prehled uloh systemu (`taskd`). `None` = hostitel ho nema.
    fn tasks(&mut self, _op: TasksOp) -> Option<Result<Value, ShError>> {
        None
    }
    /// Vypnuti (`reboot = false`) nebo restart systemu (`shutdown`,
    /// `reboot`). Pri uspechu se pocitac vypne a volani se nevrati (nebo
    /// vrati `Ok` tesne pred vypnutim); `NOT_SUPPORTED` = hostitel napajeni
    /// nema (testy, relace bez prava).
    fn power(&mut self, _reboot: bool) -> Result<(), abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Balicek pro vyvojare (`report`): text s cestou a nahledem obsahu.
    /// `NOT_SUPPORTED` = hostitel sluzbu logu nema.
    fn report(&mut self) -> Result<String, abi::Error> {
        Err(abi::Error::NotSupported)
    }
    /// Uroven logu jadra (`loglevel`): `target` prazdny = vypis.
    fn log_level(&mut self, _target: &str, _level: &str) -> Result<String, abi::Error> {
        Err(abi::Error::NotSupported)
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
