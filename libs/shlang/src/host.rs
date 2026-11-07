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

    // Snapshoty (pro undo). Bez nich si undo pamatuje puvodni obsah.
    fn snapshot(&mut self, _name: &str) -> FsResult<()> {
        Err(abi::Error::NotSupported)
    }
    fn delete_snapshot(&mut self, _name: &str) -> FsResult<()> {
        Err(abi::Error::NotSupported)
    }
    fn snapshot_stat(&mut self, _snap: &str, _path: &str) -> FsResult<Meta> {
        Err(abi::Error::NotSupported)
    }
    fn snapshot_list(&mut self, _snap: &str, _path: &str) -> FsResult<Vec<DirEntry>> {
        Err(abi::Error::NotSupported)
    }
    fn snapshot_read(&mut self, _snap: &str, _path: &str) -> FsResult<Vec<u8>> {
        Err(abi::Error::NotSupported)
    }
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
}
