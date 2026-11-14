//! Tabulka cisel fd procesu (POSIX `int fd`) pro knihovnu C `cstd`.
//!
//! Cislo ukazuje na popis otevreni ([`Description`]): soubor relace FS
//! s pozici, stdio procesu nebo konec roury. `dup` sdili popis i pozici
//! (jako POSIX). Cisla 0, 1, 2 jsou od startu stdin, stdout a stderr.
//! Bezny `std::fs::File` v Rustu cislo nezabira; tabulka je jedna na
//! proces, takze C i Rust (pres `std::os::magnumopus::runtime`) vidi totez.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use libos::sync::Mutex;

use crate::fs::{File, SeekFrom, Stat};
use crate::pipe::Pipe;
use crate::stdio::{self, Stream};
use crate::{Error, Result};

/// Nejvic otevrenych cisel v procesu (`RLIMIT_NOFILE`, `OPEN_MAX`).
pub const MAX: usize = 4096;

/// Priznaky popisu otevreni (stav `fcntl(F_GETFL)`), hodnoty jako
/// POSIX/Linux, aby je cstd predal beze zmeny.
pub mod status {
    pub const RDONLY: u32 = 0;
    pub const WRONLY: u32 = 1;
    pub const RDWR: u32 = 2;
    pub const ACCMODE: u32 = 3;
    pub const APPEND: u32 = 0o2000;
    pub const NONBLOCK: u32 = 0o4000;
    /// Priznaky, ktere smi menit `F_SETFL`.
    pub const SETTABLE: u32 = APPEND | NONBLOCK;
}

/// Na co cislo ukazuje.
pub enum Object {
    /// Soubor nebo adresar relace FS (pozice je v [`File`]).
    File(File),
    /// Standardni proud procesu.
    Stdio(Stream),
    /// Konec roury.
    Pipe(Pipe),
    /// Socket ([`crate::sock`]).
    Socket(crate::sock::Socket),
    /// Objekt knihovny nad std (napr. mnozina `epoll` v cstd); cteni
    /// a zapis nejdou.
    User(alloc::boxed::Box<dyn core::any::Any + Send + Sync>),
}

/// Popis otevreni (sdileny vsemi duplikaty cisla).
pub struct Description {
    pub object: Object,
    status: AtomicU32,
}

impl Description {
    pub fn new(object: Object, status: u32) -> Arc<Self> {
        Arc::new(Self { object, status: AtomicU32::new(status) })
    }

    /// Priznaky `status::*`.
    pub fn status(&self) -> u32 {
        self.status.load(Ordering::Acquire)
    }

    /// Zmeni menitelne priznaky (`F_SETFL`); rezim pristupu zustane.
    pub fn set_status(&self, flags: u32) {
        let keep = self.status() & !status::SETTABLE;
        self.status.store(keep | (flags & status::SETTABLE), Ordering::Release);
    }

    fn can_read(&self) -> bool {
        self.status() & status::ACCMODE != status::WRONLY
    }

    fn can_write(&self) -> bool {
        self.status() & status::ACCMODE != status::RDONLY
    }

    fn nonblock(&self) -> bool {
        self.status() & status::NONBLOCK != 0
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        if !self.can_read() {
            return Err(Error::BadHandle);
        }
        match &self.object {
            Object::File(f) => f.read(buf),
            Object::Stdio(Stream::In) => stdio::read(buf),
            Object::Stdio(_) | Object::User(_) => Err(Error::BadHandle),
            Object::Pipe(p) => p.read(buf),
            Object::Socket(s) => s.recv(buf, self.nonblock()),
        }
    }

    pub fn write(&self, data: &[u8]) -> Result<usize> {
        if !self.can_write() {
            return Err(Error::BadHandle);
        }
        match &self.object {
            Object::File(f) => f.write(data),
            Object::Stdio(Stream::In) | Object::User(_) => Err(Error::BadHandle),
            Object::Stdio(s) => stdio::write(*s, data),
            Object::Pipe(p) => p.write(data),
            Object::Socket(s) => s.send(data, self.nonblock()),
        }
    }

    /// Cteni na pozici bez jeji zmeny (`pread`); jen soubory.
    pub fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        match &self.object {
            Object::File(f) if self.can_read() => f.read_at(offset, buf),
            Object::File(_) => Err(Error::BadHandle),
            _ => Err(Error::NotSupported),
        }
    }

    /// Zapis na pozici bez jeji zmeny (`pwrite`); jen soubory.
    pub fn write_at(&self, offset: u64, data: &[u8]) -> Result<usize> {
        match &self.object {
            Object::File(f) if self.can_write() => f.write_at(offset, data),
            Object::File(_) => Err(Error::BadHandle),
            _ => Err(Error::NotSupported),
        }
    }

    /// Posun pozice; proudy a roury `NotSupported` (C: `ESPIPE`).
    pub fn seek(&self, pos: SeekFrom) -> Result<u64> {
        match &self.object {
            Object::File(f) => f.seek(pos),
            _ => Err(Error::NotSupported),
        }
    }

    /// Soubor relace FS (pro `fstat`, `ftruncate`, `fsync`, `fdopendir`).
    pub fn file(&self) -> Option<&File> {
        match &self.object {
            Object::File(f) => Some(f),
            _ => None,
        }
    }

    /// Socket pod cislem (pro funkce socketu v cstd).
    pub fn socket(&self) -> Option<&crate::sock::Socket> {
        match &self.object {
            Object::Socket(s) => Some(s),
            _ => None,
        }
    }

    /// Je neblokujici (`O_NONBLOCK`)?
    pub fn is_nonblocking(&self) -> bool {
        self.nonblock()
    }

    /// Metadata souboru; `None` = proud nebo roura.
    pub fn stat(&self) -> Option<Result<Stat>> {
        self.file().map(File::stat)
    }
}

#[derive(Clone)]
struct Slot {
    desc: Arc<Description>,
    cloexec: bool,
}

static TABLE: Mutex<Option<Vec<Option<Slot>>>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Vec<Option<Slot>>) -> R) -> R {
    let mut t = TABLE.lock();
    let v = t.get_or_insert_with(|| {
        let std = |s, st| Some(Slot { desc: Description::new(Object::Stdio(s), st), cloexec: false });
        let mut v = Vec::new();
        v.push(std(Stream::In, status::RDONLY));
        v.push(std(Stream::Out, status::WRONLY));
        v.push(std(Stream::Err, status::WRONLY));
        v
    });
    f(v)
}

fn free_from(v: &mut Vec<Option<Slot>>, min: usize) -> Result<usize> {
    if let Some(i) = (min..v.len()).find(|&i| v[i].is_none()) {
        return Ok(i);
    }
    let i = v.len().max(min);
    if i >= MAX {
        return Err(Error::NoResources);
    }
    v.resize(i + 1, None);
    Ok(i)
}

/// Prida popis pod nejnizsi volne cislo `>= min`; `NoResources` = plna
/// tabulka (C: `EMFILE`).
pub fn insert(desc: Arc<Description>, min: usize, cloexec: bool) -> Result<i32> {
    with(|v| {
        let i = free_from(v, min)?;
        v[i] = Some(Slot { desc, cloexec });
        Ok(i as i32)
    })
}

fn index(fd: i32) -> Result<usize> {
    usize::try_from(fd).ok().filter(|&i| i < MAX).ok_or(Error::BadHandle)
}

/// Popis pod cislem; `BadHandle` = neplatne cislo (C: `EBADF`).
pub fn get(fd: i32) -> Result<Arc<Description>> {
    let i = index(fd)?;
    with(|v| v.get(i).and_then(|s| s.as_ref()).map(|s| s.desc.clone()).ok_or(Error::BadHandle))
}

/// Uvolni cislo. Popis (a soubor) zanikne s poslednim duplikatem.
pub fn close(fd: i32) -> Result<()> {
    let i = index(fd)?;
    // Popis se zahodi az mimo zamek (zavreni souboru mluvi s fsd).
    let old = with(|v| v.get_mut(i).and_then(Option::take));
    old.map(drop).ok_or(Error::BadHandle)
}

/// Duplikat na nejnizsi volne cislo `>= min` (`dup`, `F_DUPFD`).
pub fn dup(fd: i32, min: i32, cloexec: bool) -> Result<i32> {
    let min = usize::try_from(min).map_err(|_| Error::InvalidArgs)?;
    if min >= MAX {
        return Err(Error::InvalidArgs);
    }
    let desc = get(fd)?;
    insert(desc, min, cloexec)
}

/// Duplikat presne na cislo `new` (`dup2`, `dup3`); puvodni obsah `new`
/// se zavre.
pub fn dup2(old: i32, new: i32, cloexec: bool) -> Result<i32> {
    let desc = get(old)?;
    let n = index(new)?;
    let prev = with(|v| {
        if v.len() <= n {
            v.resize(n + 1, None);
        }
        v[n].replace(Slot { desc, cloexec })
    });
    drop(prev);
    Ok(new)
}

/// Priznak `FD_CLOEXEC` cisla.
pub fn cloexec(fd: i32) -> Result<bool> {
    let i = index(fd)?;
    with(|v| v.get(i).and_then(|s| s.as_ref()).map(|s| s.cloexec).ok_or(Error::BadHandle))
}

pub fn set_cloexec(fd: i32, on: bool) -> Result<()> {
    let i = index(fd)?;
    with(|v| match v.get_mut(i).and_then(|s| s.as_mut()) {
        Some(s) => {
            s.cloexec = on;
            Ok(())
        }
        None => Err(Error::BadHandle),
    })
}

/// Je cislo terminal? (stdio procesu, dokud termd neda vlastni typ.)
pub fn is_terminal(fd: i32) -> Result<bool> {
    Ok(matches!(get(fd)?.object, Object::Stdio(s) if stdio::has_channel(s)))
}
