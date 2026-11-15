//! Systemova vrstva cstd: cisla fd, soubory, adresare, roury.
//!
//! Na cili magnumopus jde vse pres behove prostredi procesu (`osrt`,
//! v std jako `std::os::magnumopus::runtime`), takze tabulku fd sdili
//! C i Rust. Na hostiteli je nahradni implementace nad `std::fs` jen pro
//! testy logiky (FILE, unistd); symboly C se tam neexportuji.
//!
//! Chyby jsou rovnou cisla `errno`.

use crate::c_int;

#[cfg(target_os = "magnumopus")]
mod os;
#[cfg(target_os = "magnumopus")]
pub use os::*;
#[cfg(target_os = "magnumopus")]
mod os_mem;
#[cfg(target_os = "magnumopus")]
pub use os_mem::*;
#[cfg(target_os = "magnumopus")]
mod os_proc;
#[cfg(target_os = "magnumopus")]
pub use os_proc::*;
#[cfg(target_os = "magnumopus")]
mod os_procfd;
#[cfg(target_os = "magnumopus")]
pub use os_procfd::*;
#[cfg(target_os = "magnumopus")]
mod os_unix;
#[cfg(target_os = "magnumopus")]
pub use os_unix::*;

#[cfg(not(target_os = "magnumopus"))]
mod host;
#[cfg(not(target_os = "magnumopus"))]
pub use host::*;

/// Procesovy fd na hostiteli neni (objekt procesu jadra).
#[cfg(not(target_os = "magnumopus"))]
pub fn self_process_fd() -> R<i32> {
    Err(crate::errno::ENOSYS)
}

#[cfg(not(target_os = "magnumopus"))]
pub fn process_memory(_fd: i32, _write: bool, _addr: u64, _buf: *mut u8, _len: usize) -> R<usize> {
    Err(crate::errno::ENOSYS)
}

/// Vysledek se `errno` jako chybou.
pub type R<T> = Result<T, c_int>;

/// Priznaky popisu otevreni (`F_GETFL`), hodnoty jako Linux.
pub mod status {
    pub const RDONLY: u32 = 0;
    pub const WRONLY: u32 = 1;
    pub const RDWR: u32 = 2;
    pub const ACCMODE: u32 = 3;
    pub const APPEND: u32 = 0o2000;
    pub const NONBLOCK: u32 = 0o4000;
}

/// Jak otevrit (rozlozene `O_*` z `open`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub create: bool,
    pub excl: bool,
    pub trunc: bool,
    pub directory: bool,
    pub cloexec: bool,
    pub nonblock: bool,
    /// Mod noveho souboru po `umask` (`O_CREAT`); `None` = jen zdedit.
    pub mode: Option<u16>,
    /// `O_NOFOLLOW`: odkaz na konci cesty = `ELOOP`.
    pub nofollow: bool,
}

impl OpenFlags {
    /// Stav popisu pro `F_GETFL`.
    pub fn status(&self) -> u32 {
        let mut s = match (self.read, self.write || self.append) {
            (true, true) => status::RDWR,
            (false, true) => status::WRONLY,
            _ => status::RDONLY,
        };
        if self.append {
            s |= status::APPEND;
        }
        if self.nonblock {
            s |= status::NONBLOCK;
        }
        s
    }
}

/// Pozice pro `lseek`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seek {
    Set(u64),
    Cur(i64),
    End(i64),
}

/// Druh objektu pro `st_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
    /// Stdio procesu (znakove zarizeni, jako terminal).
    Stream,
    Pipe,
    Socket,
}

/// Odkud ma potomek standardni proud (`spawn.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildIo {
    /// Popis za cislem fd rodice (stdio, roura).
    Parent(c_int),
    /// Zahodit vystup / prazdny vstup (`/dev/null`).
    Null,
    /// Proud nepredat (`posix_spawn_file_actions_addclose`).
    Closed,
    /// Soubor z `addopen` (index v `spawn::Plan::opens`); `posix_spawn`
    /// ho pred spustenim otevre a nahradi `Parent`.
    Open(usize),
}

/// Pozadavek na spusteni potomka (`posix_spawn`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnReq {
    /// Absolutni cesta, nebo holy nazev programu systemu (launchd).
    pub program: String,
    pub args: Vec<Vec<u8>>,
    pub env: Vec<Vec<u8>>,
    /// Absolutni pracovni adresar potomka.
    pub cwd: Vec<u8>,
    pub io: [ChildIo; 3],
    /// Zdedene fd nad 2: (cislo v potomkovi, `ChildIo::Parent` nebo
    /// `Null`, z akce souboru). Druh, ktery zdedit nejde (socket), se
    /// u zdedeneho fd preskoci, u akce je to `ENOTSUP`.
    pub fds: Vec<(c_int, ChildIo, bool)>,
}

/// Sitove rozhrani mimo smycku (`ifaddrs.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Iface {
    pub mac: [u8; 6],
    pub mtu: u16,
    pub up: bool,
    pub addr: Option<(core::net::IpAddr, u8)>,
}

/// Metadata (`struct stat` je sklada v `stat.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub kind: Kind,
    pub ino: u64,
    pub size: u64,
    /// (sekundy, nanosekundy) od 1970 UTC.
    pub mtime: (i64, u32),
    pub ctime: (i64, u32),
    pub btime: (i64, u32),
    pub uid: u64,
    pub readable: bool,
    pub writable: bool,
    /// Mod POSIX z FS (9 bitu, pohled na opravneni); `None` = neznamy.
    pub mode: Option<u16>,
}

impl Stat {
    pub fn stream(kind: Kind) -> Self {
        Stat {
            kind,
            ino: 0,
            size: 0,
            mtime: (0, 0),
            ctime: (0, 0),
            btime: (0, 0),
            uid: 0,
            readable: true,
            writable: true,
            mode: None,
        }
    }
}

/// Polozka adresare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: Vec<u8>,
    pub kind: Kind,
    pub ino: u64,
}

/// Funkce vlakna pro [`spawn`].
pub type ThreadFn = Box<dyn FnOnce() + Send + 'static>;

/// Druh socketu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SockKind {
    Stream,
    Datagram,
}

/// Polozka `poll` (udalosti jako Linux `POLL*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollItem {
    pub fd: i32,
    pub events: u16,
    pub revents: u16,
}

/// Oblast adresniho prostoru (`vm_query`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// 0 volne, 1 rezervace, 2 mapovani (`abi::vm::region`).
    pub kind: u32,
    pub start: u64,
    pub len: u64,
    /// Prava `PROT_*` (hodnoty jako `abi::vm::MAP_*`).
    pub prot: u32,
}

/// Vysledek zamku souboru (`fcntl`, `flock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockResult {
    Done,
    /// `pid` = cislo procesu drzitele (`getpid` jeho cstd; 0 = nezname).
    Conflict { write: bool, start: u64, len: u64, pid: i32 },
    Deadlock,
}
