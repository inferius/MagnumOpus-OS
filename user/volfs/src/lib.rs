//! Proces vymenneho media (docs/cs/media.md): cizi souborovy system
//! (FAT, NTFS jen pro cteni; exFAT pozdeji) nad kanalem oddilu media,
//! obsluha protokolu `volproto` pro fsd.
//!
//! Knihovna je nezavisla na systemu (testy na hostiteli nad obrazem
//! v pameti); `main.rs` ji napoji na kanaly. Data z media jsou
//! nedoveryhodna: chyba parseru = chyba operace (nebo pad procesu media),
//! nikdy nic mimo tento proces.

pub mod dev;
pub mod fat;
pub mod ntfs;
pub mod serve;
pub mod time;

use std::io;

use volproto::{Error, Info, Result, Stat};

/// Svazek media (jedna implementace na souborovy system). Cesty jsou
/// normalizovane (`volproto::normalize`), `/` = koren.
pub trait Volume {
    fn info(&mut self) -> Result<Info>;
    fn stat(&mut self, path: &str) -> Result<Stat>;
    fn list(&mut self, path: &str) -> Result<Vec<(String, Stat)>>;
    fn read(&mut self, path: &str, off: u64, len: usize) -> Result<Vec<u8>>;
    fn write(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize>;
    fn create(&mut self, path: &str, kind: u8, exclusive: bool) -> Result<()>;
    fn remove(&mut self, path: &str) -> Result<()>;
    fn rename(&mut self, from: &str, to: &str, replace: bool) -> Result<()>;
    fn set_len(&mut self, path: &str, len: u64) -> Result<()>;
    /// Zmeny na medium.
    fn sync(&mut self) -> Result<()>;
    /// Zapsat a oznacit svazek jako cisty (bezpecne odpojeni).
    fn unmount(&mut self) -> Result<()>;
}

/// Chyba I/O crates FS -> kod systemu.
pub fn io_err(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::NotFound => Error::NotFound,
        io::ErrorKind::AlreadyExists => Error::AlreadyExists,
        io::ErrorKind::PermissionDenied => Error::ReadOnly,
        io::ErrorKind::InvalidInput => Error::InvalidArgs,
        io::ErrorKind::UnexpectedEof | io::ErrorKind::WriteZero => Error::IoError,
        _ => {
            let m = e.to_string();
            if m.contains("not empty") {
                Error::NotEmpty
            } else if m.contains("No space") {
                Error::NoSpace
            } else if m.contains("File name") {
                Error::InvalidArgs
            } else {
                Error::IoError
            }
        }
    }
}

#[cfg(test)]
mod tests;
