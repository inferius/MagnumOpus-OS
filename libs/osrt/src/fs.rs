//! Soubory pro `std::fs` nad relaci FS (fsproto v2).
//!
//! Relaci preda rodic ve startovni zprave (stitek `abi::params::tag::FS`);
//! jeji obsah (pripojene svazky, prava) urcil rodic, tj. manifest
//! aplikace. Bez relace vraci operace `NotSupported`. Cesty skladane
//! s pracovnim adresarem: [`crate::env::resolve`].

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use fsproto::FsClient;
use fsproto::v2::{access, open_flags, share};
use libos::Channel;

pub use fsproto::perm::{Acl, EVERYONE, Grant, Permissions, rights};
pub use fsproto::v2::PosixLock;
pub use fsproto::{DirEntry, Stat, kind};

use crate::start::Init;
use crate::{Error, Result};

static SESSION: Init<FsClient> = Init::new();

/// # Safety
///
/// Jen pri startu (viz `start::Init`).
pub(crate) unsafe fn init(channel: Option<Channel>) {
    if let Some(c) = channel {
        // SAFETY: predano volajicim.
        unsafe { SESSION.set(FsClient::new(c)) };
    }
}

fn session() -> Result<&'static FsClient> {
    SESSION.get().ok_or(Error::NotSupported)
}

/// Velikost svazku relace: (velikost bloku, bloku celkem, volnych bloku);
/// bloky cekajici na uvolneni se za volne nepocitaji (C `statvfs`).
pub fn volume_space() -> Result<(u32, u64, u64)> {
    let i = session()?.info()?;
    Ok((i.block_size, i.block_count, i.free_blocks))
}

/// Nova relace FS se stejnou identitou a pripojenymi cestami (pro
/// potomka); `None` = proces relaci nema.
pub fn new_session() -> Option<Channel> {
    SESSION.get()?.connect().ok().map(FsClient::into_channel)
}

/// Ma proces relaci FS?
pub fn available() -> bool {
    SESSION.get().is_some()
}



/// Jak otevrit soubor (odpovida `std::fs::OpenOptions`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub truncate: bool,
    pub create: bool,
    pub create_new: bool,
}

/// Otevreny soubor nebo adresar (handle relace; soubor prezije
/// prejmenovani i smazani). Pozice pro `read`/`write` je v souboru,
/// takze `&File` muze cist a psat (jako v std).
pub struct File {
    handle: u64,
    pos: AtomicU64,
    append: bool,
    /// Absolutni cesta pri otevreni (`openat` relativne k adresari v cstd;
    /// po prejmenovani muze byt zastarala).
    path: String,
}

impl File {
    pub fn open(path: &[u8], o: &OpenOptions) -> Result<File> {
        let path = crate::env::resolve(path)?;
        let mut acc = 0;
        if o.read {
            acc |= access::READ;
        }
        if o.write || o.append {
            acc |= access::WRITE;
        }
        let mut flags = 0;
        if o.create || o.create_new {
            flags |= open_flags::CREATE;
        }
        if o.create_new {
            flags |= open_flags::EXCL;
        }
        if o.truncate {
            flags |= open_flags::TRUNCATE;
        }
        // Zapis do souboru potrebuje pravo zapisu (std: truncate/create
        // bez write = chyba InvalidInput, overuje std samo).
        let opened = session()?.open(&path, acc, share::ALL, flags)?;
        Ok(File { handle: opened.handle, pos: AtomicU64::new(0), append: o.append, path })
    }

    /// Otevre adresar pro vypis.
    pub fn open_dir(path: &[u8]) -> Result<File> {
        let path = crate::env::resolve(path)?;
        let opened = session()?.open(&path, access::READ, share::ALL, 0)?;
        if opened.stat.kind == kind::FILE {
            let _ = session()?.close(opened.handle);
            return Err(Error::NotADirectory);
        }
        Ok(File { handle: opened.handle, pos: AtomicU64::new(0), append: false, path })
    }


    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let data = session()?.read_h(self.handle, offset, buf.len())?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<usize> {
        match session()?.write_h(self.handle, offset, data) {
            Ok(n) => Ok(n),
            // Cast prijata: vratit, kolik (std zavola znovu se zbytkem).
            Err((n, _)) if n > 0 => Ok(n),
            Err((_, e)) => Err(e),
        }
    }

    /// Cteni od aktualni pozice.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let at = self.pos.load(Ordering::Acquire);
        let n = self.read_at(at, buf)?;
        self.pos.fetch_add(n as u64, Ordering::AcqRel);
        Ok(n)
    }

    /// Zapis na aktualni pozici (pri `append` vzdy na konec).
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        let at = if self.append { self.stat()?.size } else { self.pos.load(Ordering::Acquire) };
        let n = self.write_at(at, data)?;
        self.pos.store(at + n as u64, Ordering::Release);
        Ok(n)
    }

    /// Nastavi pozici; vraci novou.
    pub fn seek(&self, pos: SeekFrom) -> Result<u64> {
        let new = match pos {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::Current(d) => self.pos.load(Ordering::Acquire).checked_add_signed(d),
            SeekFrom::End(d) => self.stat()?.size.checked_add_signed(d),
        };
        let new = new.ok_or(Error::InvalidArgs)?;
        self.pos.store(new, Ordering::Release);
        Ok(new)
    }

    pub fn tell(&self) -> u64 {
        self.pos.load(Ordering::Acquire)
    }

    pub fn stat(&self) -> Result<Stat> {
        session()?.stat_h(self.handle).map(|(s, _)| s)
    }

    pub fn set_len(&self, size: u64) -> Result<()> {
        session()?.set_len(self.handle, size)
    }

    /// Trvale ulozeni (`sync_all`/`sync_data`): fsync relace.
    pub fn sync(&self) -> Result<()> {
        session()?.fsync()
    }

    /// Proud bajtu nad souborem od aktualni pozice (stdin/stdout potomka,
    /// fsproto `OPEN_STREAM`): `write` = zapis (soubor v rezimu append
    /// pripojuje), jinak cteni do konce souboru. Pozice tohoto `File` se
    /// nemeni (potomek ma vlastni).
    pub fn open_stream(&self, write: bool) -> Result<Channel> {
        use fsproto::v2::stream_mode;
        let mode = match (write, self.append) {
            (false, _) => stream_mode::READ,
            (true, true) => stream_mode::APPEND,
            (true, false) => stream_mode::WRITE,
        };
        session()?.open_stream(self.handle, mode, self.tell())
    }

    /// Zamek POSIX (`fcntl`/`flock`, viz `fsproto::v2::posix_lock`).
    pub fn posix_lock(&self, cmd: u8, class: u8, write: bool, start: u64, len: u64) -> Result<fsproto::v2::PosixLock> {
        session()?.posix_lock(self.handle, cmd, class, write, start, len)
    }

    /// Absolutni cesta pri otevreni.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Polozky otevreneho adresare.
    pub fn list(&self) -> Result<Vec<DirEntry>> {
        session()?.list_h(self.handle)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        if let Ok(s) = session() {
            let _ = s.close(self.handle);
        }
    }
}

/// Pozice pro [`File::seek`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekFrom {
    Start(u64),
    End(i64),
    Current(i64),
}

pub fn stat(path: &str) -> Result<Stat> {
    session()?.stat(path)
}

/// Metadata podle cesty (relativni vuci pracovnimu adresari).
pub fn stat_path(path: &[u8]) -> Result<Stat> {
    stat(&crate::env::resolve(path)?)
}

/// Polozky adresare podle cesty.
pub fn read_dir(path: &[u8]) -> Result<(String, Vec<DirEntry>)> {
    let abs = crate::env::resolve(path)?;
    let entries = File::open_dir(abs.as_bytes())?.list()?;
    Ok((abs, entries))
}

pub fn create_dir(path: &[u8]) -> Result<()> {
    session()?.create(&crate::env::resolve(path)?, kind::DIR)
}

/// Smaze soubor (adresar odmitne: `IsADirectory`).
pub fn remove_file(path: &[u8]) -> Result<()> {
    let abs = crate::env::resolve(path)?;
    if stat(&abs)?.kind == kind::DIR {
        return Err(Error::IsADirectory);
    }
    session()?.remove(&abs)
}

/// Smaze prazdny adresar (soubor odmitne: `NotADirectory`).
pub fn remove_dir(path: &[u8]) -> Result<()> {
    let abs = crate::env::resolve(path)?;
    if stat(&abs)?.kind != kind::DIR {
        return Err(Error::NotADirectory);
    }
    session()?.remove(&abs)
}

pub fn rename(from: &[u8], to: &[u8]) -> Result<()> {
    session()?.rename(&crate::env::resolve(from)?, &crate::env::resolve(to)?)
}

/// Opravneni objektu: vlastnik, ucinna prava volajiciho a popis
/// opravneni (ACL), kdyz ho objekt ma vlastni (`None` = dedi od rodice).
pub fn permissions(path: &[u8]) -> Result<Permissions> {
    session()?.permissions(&crate::env::resolve(path)?)
}

/// Nastavi popis opravneni (`None` = dedit od rodice). Vyzaduje pravo
/// `MANAGE` na objekt.
pub fn set_acl(path: &[u8], acl: Option<&Acl>) -> Result<()> {
    session()?.set_acl(&crate::env::resolve(path)?, acl)
}
