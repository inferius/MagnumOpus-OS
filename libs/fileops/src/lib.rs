//! Operace se soubory pro spravce souboru (milnik 6c, `user/gui-files`)
//! a pomocny proces `user/files-copy`.
//!
//! - [`Vfs`]: co stroj potrebuje od FS (v OS `FsClient`, v testech
//!   [`mem::MemFs`]).
//! - [`job::Job`]: kopirovani nebo presun jako uloha po malych krocich
//!   (`step`), konflikty (prepsat / preskocit / prejmenovat, i "pro
//!   vsechny"), okamzita kopie FS (`clone_path`) s nahradou ctenim
//!   a zapisem, stav ulohy jde ulozit a po preruseni (i po restartu
//!   systemu) pokracovat.
//! - [`proto`]: zpravy mezi aplikaci a `files-copy`.
//!
//! Prepsani nikdy nemaze: puvodni cil jde do kose (lze vratit). Presun
//! v jednom datasetu je prejmenovani, mezi datasety kopie a zdroj do kose.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod job;
pub mod mem;
pub mod path;
pub mod proto;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};

/// Druh polozky z pohledu kopirovani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// Symbolicky odkaz: kopiruje se jen okamzitou kopii FS (odkaz sam).
    Link,
}

/// Metadata polozky pro kopirovani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Meta {
    pub kind: Kind,
    /// Velikost souboru (u slozky 0).
    pub size: u64,
    /// U slozky soucet velikosti v podstromu, kdyz ho FS zna a relace ho
    /// smi videt (`avail::TREE`); jinak `None` a prubeh je jen po polozkach.
    pub tree_bytes: Option<u64>,
}

impl Meta {
    /// Bajty, ktere polozka predstavuje pro prubeh (`None` = nezname).
    pub fn bytes(&self) -> Option<u64> {
        match self.kind {
            Kind::Dir => self.tree_bytes,
            _ => Some(self.size),
        }
    }
}

/// Co stroj kopirovani potrebuje od FS. Cesty jsou cesty relace.
pub trait Vfs {
    fn stat(&mut self, path: &str) -> Result<Meta>;
    /// Polozky slozky (jmeno, druh).
    fn list(&mut self, path: &str) -> Result<Vec<(String, Kind)>>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    fn create_file(&mut self, path: &str) -> Result<()>;
    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>>;
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()>;
    /// Okamzita kopie (sdileni bloku). `NOT_SUPPORTED` = stroj kopiruje
    /// ctenim a zapisem.
    fn clone_path(&mut self, src: &str, dst: &str) -> Result<()>;
    /// Prejmenovani; cil nesmi existovat.
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;
    /// Do kose (lze vratit).
    fn trash(&mut self, path: &str) -> Result<()>;
    /// Trvale smazani i se slozkami - jen pro docasne polozky stroje.
    fn remove_tree(&mut self, path: &str) -> Result<()>;
    /// Koren datasetu, ve kterem cesta lezi.
    fn dataset_of(&mut self, path: &str) -> Result<String>;
    /// Trvale ulozit zmeny pred zapisem stavu ulohy (jinak by stav po
    /// vypadku tvrdil vic, nez je na disku).
    fn sync(&mut self) -> Result<()>;
}
