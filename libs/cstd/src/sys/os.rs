//! Systemova vrstva nad osrt (cil magnumopus).

use std::os::magnumopus::runtime as rt;

use rt::fd::{self, Description, Object};
use rt::fs::{File, OpenOptions, SeekFrom};

use super::{Entry, Kind, OpenFlags, R, Seek, Stat};
use crate::errno;

fn e(err: rt::Error) -> crate::c_int {
    errno::from_os(err as i32)
}

fn kind_of(k: u8) -> Kind {
    match k {
        rt::fs::kind::DIR | rt::fs::kind::DATASET => Kind::Dir,
        rt::fs::kind::SYMLINK => Kind::Symlink,
        _ => Kind::File,
    }
}

fn stat_of(s: &rt::fs::Stat) -> Stat {
    let r = rt::fs::rights::READ;
    let w = rt::fs::rights::WRITE;
    Stat {
        kind: kind_of(s.kind),
        ino: s.oid,
        size: s.size,
        mtime: (s.mtime.secs, s.mtime.nanos),
        ctime: (s.ctime.secs, s.ctime.nanos),
        btime: (s.btime.secs, s.btime.nanos),
        uid: s.owner,
        readable: s.rights & r != 0,
        writable: s.rights & w != 0,
    }
}

fn desc(fd: i32) -> R<std::sync::Arc<Description>> {
    fd::get(fd).map_err(e)
}

pub fn open(path: &[u8], o: &OpenFlags) -> R<i32> {
    let file = if o.directory {
        File::open_dir(path).map_err(e)?
    } else {
        let opts = OpenOptions {
            read: o.read,
            write: o.write && !o.append,
            append: o.append,
            truncate: o.trunc,
            create: o.create && !o.excl,
            create_new: o.create && o.excl,
        };
        let f = File::open(path, &opts).map_err(e)?;
        // Adresar otevreny jen pro cteni je v POSIX povoleny (fdopendir).
        if (o.write || o.append) && f.stat().map_err(e)?.kind == rt::fs::kind::DIR {
            return Err(errno::EISDIR);
        }
        f
    };
    fd::insert(Description::new(Object::File(file), o.status()), 0, o.cloexec).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        other => e(other),
    })
}

pub fn close(fd: i32) -> R<()> {
    fd::close(fd).map_err(e)
}

pub fn read(fd: i32, buf: &mut [u8]) -> R<usize> {
    desc(fd)?.read(buf).map_err(e)
}

pub fn write(fd: i32, data: &[u8]) -> R<usize> {
    desc(fd)?.write(data).map_err(|err| match err {
        rt::Error::PeerClosed => errno::EPIPE,
        other => e(other),
    })
}

pub fn pread(fd: i32, buf: &mut [u8], off: u64) -> R<usize> {
    desc(fd)?.read_at(off, buf).map_err(|err| match err {
        rt::Error::NotSupported => errno::ESPIPE,
        other => e(other),
    })
}

pub fn pwrite(fd: i32, data: &[u8], off: u64) -> R<usize> {
    desc(fd)?.write_at(off, data).map_err(|err| match err {
        rt::Error::NotSupported => errno::ESPIPE,
        other => e(other),
    })
}

pub fn seek(fd: i32, pos: Seek) -> R<u64> {
    let p = match pos {
        Seek::Set(p) => SeekFrom::Start(p),
        Seek::Cur(d) => SeekFrom::Current(d),
        Seek::End(d) => SeekFrom::End(d),
    };
    desc(fd)?.seek(p).map_err(|err| match err {
        rt::Error::NotSupported => errno::ESPIPE,
        other => e(other),
    })
}

pub fn dup(fd: i32, min: i32, cloexec: bool) -> R<i32> {
    fd::dup(fd, min, cloexec).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        other => e(other),
    })
}

pub fn dup2(old: i32, new: i32, cloexec: bool) -> R<i32> {
    fd::dup2(old, new, cloexec).map_err(e)
}

pub fn cloexec(fd: i32) -> R<bool> {
    fd::cloexec(fd).map_err(e)
}

pub fn set_cloexec(fd: i32, on: bool) -> R<()> {
    fd::set_cloexec(fd, on).map_err(e)
}

pub fn status(fd: i32) -> R<u32> {
    Ok(desc(fd)?.status())
}

pub fn set_status(fd: i32, flags: u32) -> R<()> {
    desc(fd)?.set_status(flags);
    Ok(())
}

pub fn fstat(fd: i32) -> R<Stat> {
    let d = desc(fd)?;
    match &d.object {
        Object::File(f) => f.stat().map(|s| stat_of(&s)).map_err(e),
        Object::Stdio(_) => Ok(Stat::stream(Kind::Stream)),
        Object::Pipe(_) => Ok(Stat::stream(Kind::Pipe)),
        Object::Socket(_) => Ok(Stat::stream(Kind::Socket)),
        Object::User(_) => Ok(Stat::stream(Kind::Stream)),
    }
}

pub fn stat(path: &[u8]) -> R<Stat> {
    rt::fs::stat_path(path).map(|s| stat_of(&s)).map_err(e)
}

/// Velikost svazku: (velikost bloku, bloku, volnych bloku).
pub fn volume_space() -> R<(u32, u64, u64)> {
    rt::fs::volume_space().map_err(e)
}

pub fn ftruncate(fd: i32, len: u64) -> R<()> {
    let d = desc(fd)?;
    match d.file() {
        Some(f) => f.set_len(len).map_err(e),
        None => Err(errno::EINVAL),
    }
}

pub fn fsync(fd: i32) -> R<()> {
    let d = desc(fd)?;
    match d.file() {
        Some(f) => f.sync().map_err(e),
        // Proudy a roury nemaji co ulozit.
        None => Ok(()),
    }
}

pub fn pipe(cloexec: bool, nonblock: bool) -> R<(i32, i32)> {
    let (r, w) = rt::pipe::pipe().map_err(e)?;
    let nb = if nonblock { super::status::NONBLOCK } else { 0 };
    let a = fd::insert(Description::new(Object::Pipe(r), super::status::RDONLY | nb), 0, cloexec).map_err(e)?;
    match fd::insert(Description::new(Object::Pipe(w), super::status::WRONLY | nb), 0, cloexec) {
        Ok(b) => Ok((a, b)),
        Err(err) => {
            let _ = fd::close(a);
            Err(if err == rt::Error::NoResources { errno::EMFILE } else { e(err) })
        }
    }
}

pub fn isatty(fd: i32) -> R<bool> {
    fd::is_terminal(fd).map_err(e)
}

/// Absolutni cesta otevreneho souboru nebo adresare (`openat`).
pub fn fd_path(fd: i32) -> R<Vec<u8>> {
    let d = desc(fd)?;
    let f = d.file().ok_or(errno::ENOTDIR)?;
    Ok(f.path().as_bytes().to_vec())
}

/// Absolutni normalizovana cesta (`.`, `..`, pracovni adresar).
pub fn abs_path(path: &[u8]) -> R<Vec<u8>> {
    rt::env::resolve(path).map(String::into_bytes).map_err(e)
}

pub fn list_dir(fd: i32) -> R<Vec<Entry>> {
    let d = desc(fd)?;
    let f = d.file().ok_or(errno::ENOTDIR)?;
    let list = f.list().map_err(e)?;
    Ok(list.into_iter().map(|x| Entry { name: x.name.into_bytes(), kind: kind_of(x.kind), ino: x.oid }).collect())
}

pub fn mkdir(path: &[u8]) -> R<()> {
    rt::fs::create_dir(path).map_err(e)
}

pub fn unlink(path: &[u8]) -> R<()> {
    rt::fs::remove_file(path).map_err(e)
}

pub fn rmdir(path: &[u8]) -> R<()> {
    rt::fs::remove_dir(path).map_err(e)
}

pub fn rename(from: &[u8], to: &[u8]) -> R<()> {
    rt::fs::rename(from, to).map_err(e)
}

pub fn getcwd() -> Vec<u8> {
    rt::env::current_dir().into_bytes()
}

pub fn chdir(path: &[u8]) -> R<()> {
    // Cil musi byt existujici adresar (POSIX: ENOENT / ENOTDIR).
    let s = stat(path)?;
    if s.kind != Kind::Dir {
        return Err(errno::ENOTDIR);
    }
    rt::env::set_current_dir(path).map_err(e)
}

/// Bezici vlakno (C `pthread_t` drzi popis s timto handlem).
pub struct Thread(rt::thread::Thread);

/// Nove vlakno nad osrt (TLS jako std, i pod zavadecem).
pub fn spawn(stack: usize, f: super::ThreadFn) -> R<Thread> {
    rt::thread::spawn(stack, f).map(Thread).map_err(|_| errno::EAGAIN)
}

impl Thread {
    pub fn join(self) -> R<()> {
        self.0.join().map_err(e)
    }
}

/// Ukonci aktualni vlakno bez rozbaleni zasobniku (`pthread_exit`).
pub fn exit_thread() -> ! {
    rt::thread::exit_current()
}

/// Ceka, dokud `*f == expected` (nejdele `timeout`); `false` = vyprselo.
pub fn futex_wait(f: &std::sync::atomic::AtomicU32, expected: u32, timeout: Option<core::time::Duration>) -> bool {
    rt::futex::futex_wait(f, expected, timeout)
}

pub fn futex_wake(f: &std::sync::atomic::AtomicU32, all: bool) {
    if all {
        rt::futex::futex_wake_all(f);
    } else {
        rt::futex::futex_wake(f);
    }
}

/// Monotonni cas v ns od startu.
pub fn monotonic_ns() -> u64 {
    rt::time::monotonic_ns()
}

/// Realny cas (sekundy, ns od 1970 UTC); `None` = jadro ho nezna.
pub fn realtime() -> Option<(i64, u32)> {
    rt::time::realtime()
}

/// Nahodne bajty z nasazeneho generatoru jadra; `false` = jeste neni
/// nasazeny (volajici ceka a zkusi znovu).
pub fn entropy(buf: &mut [u8]) -> bool {
    rt::fill_random_seeded(buf).is_ok()
}

// --- sockety a cekani ------------------------------------------------------

use std::net::SocketAddr;

fn net_err(err: rt::Error) -> crate::c_int {
    match err {
        rt::Error::ShouldWait => errno::EAGAIN,
        rt::Error::NotSupported => errno::ENETDOWN,
        rt::Error::BadState => errno::ENOTCONN,
        rt::Error::InvalidArgs => errno::EINVAL,
        rt::Error::AlreadyExists => errno::EISCONN,
        rt::Error::PeerClosed => errno::EPIPE,
        other => e(other),
    }
}

fn sock<T>(fd: i32, f: impl FnOnce(&rt::sock::Socket, bool) -> rt::Result<T>) -> R<T> {
    let d = desc(fd)?;
    let s = d.socket().ok_or(errno::ENOTSOCK)?;
    f(s, d.is_nonblocking()).map_err(net_err)
}

pub fn socket(kind: super::SockKind, v6: bool, cloexec: bool, nonblock: bool) -> R<i32> {
    let k = match kind {
        super::SockKind::Stream => rt::sock::Kind::Stream,
        super::SockKind::Datagram => rt::sock::Kind::Datagram,
    };
    let st = super::status::RDWR | if nonblock { super::status::NONBLOCK } else { 0 };
    fd::insert(Description::new(Object::Socket(rt::sock::Socket::new(k, v6)), st), 0, cloexec).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        other => e(other),
    })
}

pub fn sock_bind(fd: i32, a: SocketAddr) -> R<()> {
    sock(fd, |s, _| s.bind(a))
}

pub fn sock_listen(fd: i32, backlog: u16) -> R<()> {
    sock(fd, |s, _| s.listen(backlog))
}

pub fn sock_connect(fd: i32, a: SocketAddr) -> R<()> {
    sock(fd, |s, nb| s.connect(a, nb))
}

pub fn sock_accept(fd: i32, cloexec: bool, nonblock: bool) -> R<(i32, SocketAddr)> {
    let (new, from) = sock(fd, |s, nb| s.accept(nb))?;
    let st = super::status::RDWR | if nonblock { super::status::NONBLOCK } else { 0 };
    let n = fd::insert(Description::new(Object::Socket(new), st), 0, cloexec).map_err(e)?;
    Ok((n, from))
}

pub fn sock_recv(fd: i32, buf: &mut [u8], dontwait: bool) -> R<usize> {
    sock(fd, |s, nb| s.recv(buf, nb || dontwait))
}

pub fn sock_send(fd: i32, data: &[u8], dontwait: bool) -> R<usize> {
    sock(fd, |s, nb| s.send(data, nb || dontwait))
}

pub fn sock_recv_from(fd: i32, buf: &mut [u8], dontwait: bool) -> R<(usize, SocketAddr)> {
    sock(fd, |s, nb| s.recv_from(buf, nb || dontwait))
}

pub fn sock_send_to(fd: i32, data: &[u8], to: SocketAddr) -> R<usize> {
    sock(fd, |s, _| s.send_to(data, to))
}

pub fn sock_shutdown_write(fd: i32) -> R<()> {
    sock(fd, |s, _| s.shutdown_write())
}

pub fn sock_name(fd: i32, peer: bool) -> R<SocketAddr> {
    sock(fd, |s, _| if peer { s.peer_addr() } else { s.local_addr() })
}

pub fn sock_is_stream(fd: i32) -> R<(bool, bool)> {
    sock(fd, |s, _| Ok((s.kind == rt::sock::Kind::Stream, s.v6)))
}

pub fn sock_set_timeouts(fd: i32, read: Option<Option<core::time::Duration>>, write: Option<Option<core::time::Duration>>) -> R<()> {
    sock(fd, |s, _| {
        s.set_timeouts(read, write);
        Ok(())
    })
}

pub fn sock_timeouts(fd: i32) -> R<(Option<core::time::Duration>, Option<core::time::Duration>)> {
    sock(fd, |s, _| Ok(s.timeouts()))
}

pub fn sock_listening(fd: i32) -> R<bool> {
    sock(fd, |s, _| Ok(s.is_listening()))
}

pub fn resolve(host: &str) -> R<Vec<std::net::IpAddr>> {
    rt::sock::resolve(host).map_err(net_err)
}

pub fn poll(items: &mut [super::PollItem], timeout: Option<core::time::Duration>) -> R<usize> {
    let mut v: Vec<rt::poll::PollFd> =
        items.iter().map(|p| rt::poll::PollFd { fd: p.fd, events: p.events, revents: 0 }).collect();
    let n = rt::poll::poll(&mut v, timeout).map_err(e)?;
    for (a, b) in items.iter_mut().zip(&v) {
        a.revents = b.revents;
    }
    Ok(n)
}

/// Objekt knihovny v tabulce fd (mnozina `epoll`).
pub fn insert_user(obj: Box<dyn core::any::Any + Send + Sync>, cloexec: bool) -> R<i32> {
    fd::insert(Description::new(Object::User(obj), super::status::RDWR), 0, cloexec).map_err(|err| match err {
        rt::Error::NoResources => errno::EMFILE,
        other => e(other),
    })
}

/// Objekt knihovny pod cislem.
pub fn with_user<T>(fd: i32, f: impl FnOnce(&(dyn core::any::Any + Send + Sync)) -> R<T>) -> R<T> {
    let d = desc(fd)?;
    match &d.object {
        Object::User(o) => f(o.as_ref()),
        _ => Err(errno::EINVAL),
    }
}

// --- pamet (mmap) ------------------------------------------------------------

fn vm_err(err: rt::Error) -> crate::c_int {
    match err {
        rt::Error::NoMemory | rt::Error::NoResources => errno::ENOMEM,
        rt::Error::AccessDenied => errno::EACCES,
        rt::Error::OutOfRange | rt::Error::NotFound => errno::ENOMEM,
        rt::Error::AlreadyExists => errno::EEXIST,
        other => e(other),
    }
}

pub fn page_size() -> u64 {
    rt::vm::page_size()
}

pub fn vm_alloc(len: u64, prot: u32) -> R<u64> {
    rt::vm::alloc(len, prot).map_err(vm_err)
}

pub fn vm_reserve(len: u64) -> R<u64> {
    rt::vm::reserve(len, 0).map_err(vm_err)
}

pub fn vm_reserve_at(addr: u64, len: u64) -> R<u64> {
    rt::vm::reserve_at(addr, len, 0).map_err(vm_err)
}

pub fn vm_commit(addr: u64, len: u64, prot: u32) -> R<()> {
    rt::vm::commit(addr, len, prot).map_err(vm_err)
}

pub fn vm_protect(addr: u64, len: u64, prot: u32) -> R<()> {
    rt::vm::protect(addr, len, prot).map_err(vm_err)
}

pub fn vm_release(addr: u64, len: u64) -> R<()> {
    rt::vm::release(addr, len).map_err(vm_err)
}

pub fn vm_decommit(addr: u64, len: u64) -> R<()> {
    rt::vm::decommit(addr, len).map_err(vm_err)
}

pub fn vm_seal(addr: u64, len: u64, prot: u32) -> R<()> {
    rt::vm::seal(addr, len, prot).map_err(vm_err)
}

pub fn vm_query(addr: u64) -> R<super::Region> {
    let r = rt::vm::query(addr).map_err(vm_err)?;
    Ok(super::Region { kind: r.kind, start: r.start, len: r.len, prot: r.flags & 7 })
}

// --- zamky souboru -----------------------------------------------------------

use super::LockResult;

pub fn file_lock(fd: i32, cmd: u8, class: u8, write: bool, start: u64, len: u64) -> R<LockResult> {
    let d = desc(fd)?;
    let f = d.file().ok_or(errno::EINVAL)?;
    match f.posix_lock(cmd, class, write, start, len) {
        Ok(fsproto_lock) => Ok(match fsproto_lock {
            rt::fs::PosixLock::Done => LockResult::Done,
            rt::fs::PosixLock::Conflict { write, start, len } => LockResult::Conflict { write, start, len },
            rt::fs::PosixLock::Deadlock => LockResult::Deadlock,
        }),
        Err(rt::Error::ServiceRestarted) => Err(errno::ENOLCK),
        Err(err) => Err(e(err)),
    }
}
