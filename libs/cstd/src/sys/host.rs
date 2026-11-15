//! Nahradni systemova vrstva pro testy na hostiteli: tabulka fd nad
//! `std::fs` (stejna pravidla cislovani jako osrt). Na cili se nepouziva.

use std::fs;
use std::io::{self, Read, Seek as _, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::{Entry, Kind, OpenFlags, R, Seek, Stat};
use crate::errno;

const MAX: usize = 4096;

enum Obj {
    File(Mutex<fs::File>),
    Dir(PathBuf),
    Stdin,
    Stdout,
    Stderr,
    PipeR(Mutex<io::PipeReader>),
    PipeW(Mutex<io::PipeWriter>),
    User(Box<dyn core::any::Any + Send + Sync>),
}

struct Desc {
    obj: Obj,
    status: Mutex<u32>,
}

#[derive(Clone)]
struct Slot {
    desc: Arc<Desc>,
    cloexec: bool,
}

static TABLE: Mutex<Vec<Option<Slot>>> = Mutex::new(Vec::new());

fn with<T>(f: impl FnOnce(&mut Vec<Option<Slot>>) -> T) -> T {
    let mut t = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    if t.is_empty() {
        for (o, s) in [(Obj::Stdin, 0), (Obj::Stdout, 1), (Obj::Stderr, 1)] {
            t.push(Some(Slot { desc: Arc::new(Desc { obj: o, status: Mutex::new(s) }), cloexec: false }));
        }
    }
    f(&mut t)
}

fn io(e: io::Error) -> crate::c_int {
    errno::from_io(&e)
}

fn insert(obj: Obj, status: u32, min: usize, cloexec: bool) -> R<i32> {
    let desc = Arc::new(Desc { obj, status: Mutex::new(status) });
    insert_desc(desc, min, cloexec)
}

fn insert_desc(desc: Arc<Desc>, min: usize, cloexec: bool) -> R<i32> {
    with(|v| {
        let i = match (min..v.len()).find(|&i| v[i].is_none()) {
            Some(i) => i,
            None => {
                let i = v.len().max(min);
                if i >= MAX {
                    return Err(errno::EMFILE);
                }
                v.resize(i + 1, None);
                i
            }
        };
        v[i] = Some(Slot { desc, cloexec });
        Ok(i as i32)
    })
}

fn get(fd: i32) -> R<Arc<Desc>> {
    let i = usize::try_from(fd).map_err(|_| errno::EBADF)?;
    with(|v| v.get(i).and_then(|s| s.as_ref()).map(|s| s.desc.clone()).ok_or(errno::EBADF))
}

fn path(p: &[u8]) -> R<PathBuf> {
    core::str::from_utf8(p).map(PathBuf::from).map_err(|_| errno::EINVAL)
}

fn stat_md(m: &fs::Metadata) -> Stat {
    let t = |r: io::Result<std::time::SystemTime>| {
        r.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or((0, 0), |d| (d.as_secs() as i64, d.subsec_nanos()))
    };
    Stat {
        kind: if m.is_dir() { Kind::Dir } else { Kind::File },
        ino: 0,
        size: m.len(),
        mtime: t(m.modified()),
        ctime: t(m.modified()),
        btime: t(m.created()),
        uid: 0,
        readable: true,
        writable: !m.permissions().readonly(),
    }
}

pub fn open(p: &[u8], o: &OpenFlags) -> R<i32> {
    let pb = path(p)?;
    if o.directory {
        if !fs::metadata(&pb).map_err(io)?.is_dir() {
            return Err(errno::ENOTDIR);
        }
        return insert(Obj::Dir(pb), 0, 0, o.cloexec);
    }
    if let Ok(m) = fs::metadata(&pb)
        && m.is_dir()
    {
        if o.write || o.append {
            return Err(errno::EISDIR);
        }
        return insert(Obj::Dir(pb), 0, 0, o.cloexec);
    }
    let f = fs::OpenOptions::new()
        .read(o.read)
        .write(o.write && !o.append)
        .append(o.append)
        .truncate(o.trunc)
        .create(o.create && !o.excl)
        .create_new(o.create && o.excl)
        .open(&pb)
        .map_err(io)?;
    insert(Obj::File(Mutex::new(f)), o.status(), 0, o.cloexec)
}

pub fn close(fd: i32) -> R<()> {
    let i = usize::try_from(fd).map_err(|_| errno::EBADF)?;
    let old = with(|v| v.get_mut(i).and_then(Option::take));
    old.map(drop).ok_or(errno::EBADF)
}

fn can(d: &Desc, write: bool) -> bool {
    let acc = *d.status.lock().unwrap() & super::status::ACCMODE;
    if write { acc != super::status::RDONLY } else { acc != super::status::WRONLY }
}

pub fn read(fd: i32, buf: &mut [u8]) -> R<usize> {
    let d = get(fd)?;
    if !can(&d, false) {
        return Err(errno::EBADF);
    }
    match &d.obj {
        Obj::File(f) => f.lock().unwrap().read(buf).map_err(io),
        Obj::Stdin => io::stdin().read(buf).map_err(io),
        Obj::PipeR(p) => p.lock().unwrap().read(buf).map_err(io),
        Obj::Dir(_) => Err(errno::EISDIR),
        _ => Err(errno::EBADF),
    }
}

pub fn write(fd: i32, data: &[u8]) -> R<usize> {
    let d = get(fd)?;
    if !can(&d, true) {
        return Err(errno::EBADF);
    }
    match &d.obj {
        Obj::File(f) => f.lock().unwrap().write(data).map_err(io),
        Obj::Stdout => io::stdout().write(data).map_err(io),
        Obj::Stderr => io::stderr().write(data).map_err(io),
        Obj::PipeW(p) => p.lock().unwrap().write(data).map_err(io),
        _ => Err(errno::EBADF),
    }
}

pub fn pread(fd: i32, buf: &mut [u8], off: u64) -> R<usize> {
    let d = get(fd)?;
    let Obj::File(f) = &d.obj else { return Err(errno::ESPIPE) };
    let mut f = f.lock().unwrap();
    let keep = f.stream_position().map_err(io)?;
    f.seek(SeekFrom::Start(off)).map_err(io)?;
    let r = f.read(buf).map_err(io);
    f.seek(SeekFrom::Start(keep)).map_err(io)?;
    r
}

pub fn pwrite(fd: i32, data: &[u8], off: u64) -> R<usize> {
    let d = get(fd)?;
    let Obj::File(f) = &d.obj else { return Err(errno::ESPIPE) };
    let mut f = f.lock().unwrap();
    let keep = f.stream_position().map_err(io)?;
    f.seek(SeekFrom::Start(off)).map_err(io)?;
    let r = f.write(data).map_err(io);
    f.seek(SeekFrom::Start(keep)).map_err(io)?;
    r
}

pub fn seek(fd: i32, pos: Seek) -> R<u64> {
    let d = get(fd)?;
    let Obj::File(f) = &d.obj else { return Err(errno::ESPIPE) };
    let p = match pos {
        Seek::Set(p) => SeekFrom::Start(p),
        Seek::Cur(x) => SeekFrom::Current(x),
        Seek::End(x) => SeekFrom::End(x),
    };
    f.lock().unwrap().seek(p).map_err(io)
}

pub fn dup(fd: i32, min: i32, cloexec: bool) -> R<i32> {
    let min = usize::try_from(min).map_err(|_| errno::EINVAL)?;
    insert_desc(get(fd)?, min, cloexec)
}

pub fn dup2(old: i32, new: i32, cloexec: bool) -> R<i32> {
    let desc = get(old)?;
    let n = usize::try_from(new).ok().filter(|&n| n < MAX).ok_or(errno::EBADF)?;
    let prev = with(|v| {
        if v.len() <= n {
            v.resize(n + 1, None);
        }
        v[n].replace(Slot { desc, cloexec })
    });
    drop(prev);
    Ok(new)
}

/// Otevrena cisla s `FD_CLOEXEC` (`posix_spawn`).
pub fn fd_list() -> Vec<(i32, bool)> {
    with(|v| v.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i as i32, s.cloexec))).collect())
}

pub fn cloexec(fd: i32) -> R<bool> {
    let i = usize::try_from(fd).map_err(|_| errno::EBADF)?;
    with(|v| v.get(i).and_then(|s| s.as_ref()).map(|s| s.cloexec).ok_or(errno::EBADF))
}

pub fn set_cloexec(fd: i32, on: bool) -> R<()> {
    let i = usize::try_from(fd).map_err(|_| errno::EBADF)?;
    with(|v| match v.get_mut(i).and_then(|s| s.as_mut()) {
        Some(s) => {
            s.cloexec = on;
            Ok(())
        }
        None => Err(errno::EBADF),
    })
}

pub fn status(fd: i32) -> R<u32> {
    Ok(*get(fd)?.status.lock().unwrap())
}

pub fn set_status(fd: i32, flags: u32) -> R<()> {
    let d = get(fd)?;
    let mut s = d.status.lock().unwrap();
    let settable = super::status::APPEND | super::status::NONBLOCK;
    *s = (*s & !settable) | (flags & settable);
    Ok(())
}

pub fn fstat(fd: i32) -> R<Stat> {
    let d = get(fd)?;
    match &d.obj {
        Obj::File(f) => f.lock().unwrap().metadata().map(|m| stat_md(&m)).map_err(io),
        Obj::Dir(p) => fs::metadata(p).map(|m| stat_md(&m)).map_err(io),
        Obj::PipeR(_) | Obj::PipeW(_) => Ok(Stat::stream(Kind::Pipe)),
        _ => Ok(Stat::stream(Kind::Stream)),
    }
}

/// Na hostiteli fd cestu neznaji (jen testy logiky).
pub fn fd_path(_fd: i32) -> R<Vec<u8>> {
    Err(errno::ENOTSUP)
}

pub fn abs_path(path: &[u8]) -> R<Vec<u8>> {
    let p = std::str::from_utf8(path).map_err(|_| errno::EINVAL)?;
    if p.is_empty() {
        return Err(errno::ENOENT);
    }
    std::path::absolute(p).map(|a| a.to_string_lossy().replace('\\', "/").into_bytes()).map_err(|_| errno::EINVAL)
}

/// Na hostiteli pevna velikost (jen testy logiky statvfs).
pub fn volume_space() -> R<(u32, u64, u64)> {
    Ok((4096, 1 << 20, 1 << 18))
}

pub fn stat(p: &[u8]) -> R<Stat> {
    fs::metadata(path(p)?).map(|m| stat_md(&m)).map_err(io)
}

pub fn ftruncate(fd: i32, len: u64) -> R<()> {
    let d = get(fd)?;
    let Obj::File(f) = &d.obj else { return Err(errno::EINVAL) };
    f.lock().unwrap().set_len(len).map_err(io)
}

pub fn fsync(fd: i32) -> R<()> {
    let d = get(fd)?;
    match &d.obj {
        Obj::File(f) => f.lock().unwrap().sync_all().map_err(io),
        _ => Ok(()),
    }
}

pub fn pipe(cloexec: bool, nonblock: bool) -> R<(i32, i32)> {
    let (r, w) = io::pipe().map_err(io)?;
    let nb = if nonblock { super::status::NONBLOCK } else { 0 };
    let a = insert(Obj::PipeR(Mutex::new(r)), super::status::RDONLY | nb, 0, cloexec)?;
    let b = insert(Obj::PipeW(Mutex::new(w)), super::status::WRONLY | nb, 0, cloexec)?;
    Ok((a, b))
}

/// Hostitel: terminal v pameti (`ttyproto::Tty`, 24x80, okno smi menit
/// velikost) pro testy `termios.rs`; udalost zmeny velikosti = pocitadlo.
static HOST_TTY: Mutex<Option<ttyproto::Tty>> = Mutex::new(None);
/// Pocet zmen velikosti od posledniho `tty_wait_resize`.
static HOST_RESIZES: (Mutex<u32>, std::sync::Condvar) = (Mutex::new(0), std::sync::Condvar::new());

pub fn tty_available() -> bool {
    true
}

pub fn tty_call(req: &[u8]) -> R<(Vec<u8>, Vec<u32>)> {
    use ttyproto::{Request, Size, Tty};
    let r = Request::decode(req).map_err(|_| errno::EINVAL)?;
    let mut g = HOST_TTY.lock().unwrap();
    let t = g.get_or_insert_with(|| {
        let mut t = Tty::new(Size { rows: 24, cols: 80, xpixel: 0, ypixel: 0 });
        t.client_resize = true;
        t
    });
    let before = t.size;
    let out = t.handle(&r);
    if t.size != before {
        *HOST_RESIZES.0.lock().unwrap() += 1;
        HOST_RESIZES.1.notify_all();
    }
    let handles = if matches!(r, Request::WatchSize) && out.is_ok() { vec![1] } else { vec![] };
    Ok((ttyproto::encode_reply(out), handles))
}

pub fn tty_wait_resize(_ev: u32) -> R<()> {
    let mut n = HOST_RESIZES.0.lock().unwrap();
    while *n == 0 {
        n = HOST_RESIZES.1.wait(n).unwrap();
    }
    *n -= 1;
    Ok(())
}

pub fn close_handles(_h: Vec<u32>) {}

pub fn abi_errno(e: abi::Error) -> crate::c_int {
    errno::from_os(e as i32)
}

pub fn isatty(fd: i32) -> R<bool> {
    let d = get(fd)?;
    Ok(matches!(d.obj, Obj::Stdin | Obj::Stdout | Obj::Stderr))
}

/// Hostitel: pevne rozhrani z dokumentacniho rozsahu (testy prevodu).
pub const HOST_IFACE: super::Iface = super::Iface {
    mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
    mtu: 1500,
    up: true,
    addr: Some((core::net::IpAddr::V4(core::net::Ipv4Addr::new(192, 0, 2, 10)), 24)),
};

pub fn net_interface() -> Option<super::Iface> {
    Some(HOST_IFACE)
}

pub fn fchdir(fd: i32) -> R<()> {
    let d = get(fd)?;
    let Obj::Dir(p) = &d.obj else { return Err(errno::ENOTDIR) };
    std::env::set_current_dir(p).map_err(io)
}

pub fn list_dir(fd: i32) -> R<Vec<Entry>> {
    let d = get(fd)?;
    let Obj::Dir(p) = &d.obj else { return Err(errno::ENOTDIR) };
    let mut out = Vec::new();
    for x in fs::read_dir(p).map_err(io)? {
        let x = x.map_err(io)?;
        let kind = if x.file_type().map_err(io)?.is_dir() { Kind::Dir } else { Kind::File };
        out.push(Entry { name: x.file_name().into_encoded_bytes(), kind, ino: 0 });
    }
    Ok(out)
}

pub fn mkdir(p: &[u8]) -> R<()> {
    fs::create_dir(path(p)?).map_err(io)
}

pub fn unlink(p: &[u8]) -> R<()> {
    let pb = path(p)?;
    if fs::metadata(&pb).map_err(io)?.is_dir() {
        return Err(errno::EISDIR);
    }
    fs::remove_file(pb).map_err(io)
}

pub fn rmdir(p: &[u8]) -> R<()> {
    fs::remove_dir(path(p)?).map_err(io)
}

pub fn rename(a: &[u8], b: &[u8]) -> R<()> {
    fs::rename(path(a)?, path(b)?).map_err(io)
}

pub fn getcwd() -> Vec<u8> {
    std::env::current_dir().map(|p| p.into_os_string().into_encoded_bytes()).unwrap_or_default()
}

pub fn chdir(p: &[u8]) -> R<()> {
    std::env::set_current_dir(path(p)?).map_err(io)
}

/// Vlakno na hostiteli (std::thread).
pub struct Thread(std::thread::JoinHandle<()>);

pub fn spawn(stack: usize, f: super::ThreadFn) -> R<Thread> {
    std::thread::Builder::new().stack_size(stack).spawn(f).map(Thread).map_err(|_| errno::EAGAIN)
}

/// Na hostiteli nejde vlakno spustit na cizim zasobniku: vlastni zasobnik
/// vychozi velikosti (testy kontroluji atributy a beh vlakna).
pub fn spawn_on(_top: usize, f: super::ThreadFn) -> R<Thread> {
    spawn(2 * 1024 * 1024, f)
}

impl Thread {
    pub fn join(self) -> R<()> {
        self.0.join().map_err(|_| errno::EINVAL)
    }
}

/// Na hostiteli se vlakno ukoncit bez rozbaleni neda (testy to nevolaji).
pub fn exit_thread() -> ! {
    panic!("pthread_exit na hostiteli neni")
}

/// Nahrada futexu: jeden globalni zamek a podminka (pomale, ale spravne).
static FUTEX: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());

pub fn futex_wait(f: &std::sync::atomic::AtomicU32, expected: u32, timeout: Option<core::time::Duration>) -> bool {
    let g = FUTEX.0.lock().unwrap_or_else(|e| e.into_inner());
    if f.load(std::sync::atomic::Ordering::SeqCst) != expected {
        return true;
    }
    match timeout {
        Some(t) => {
            let (_g, r) = FUTEX.1.wait_timeout(g, t).unwrap_or_else(|e| e.into_inner());
            !r.timed_out()
        }
        None => {
            drop(FUTEX.1.wait(g));
            true
        }
    }
}

pub fn futex_wake(_f: &std::sync::atomic::AtomicU32, _all: bool) {
    let _g = FUTEX.0.lock().unwrap_or_else(|e| e.into_inner());
    FUTEX.1.notify_all();
}

pub fn monotonic_ns() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_nanos() as u64
}

/// Na hostiteli (jen testy logiky) z `RandomState` std.
pub fn entropy(buf: &mut [u8]) -> bool {
    use std::hash::{BuildHasher, Hasher};
    for chunk in buf.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_usize(chunk.as_ptr() as usize);
        chunk.copy_from_slice(&h.finish().to_le_bytes()[..chunk.len()]);
    }
    true
}

pub fn realtime() -> Option<(i64, u32)> {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((d.as_secs() as i64, d.subsec_nanos()))
}

// --- sockety a cekani (na hostiteli jen epoll a poll nad soubory) ----------

use std::net::SocketAddr;

pub fn socket(_k: super::SockKind, _v6: bool, _cloexec: bool, _nb: bool) -> R<i32> {
    Err(errno::ENOSYS)
}
pub fn sock_bind(_fd: i32, _a: SocketAddr) -> R<()> {
    Err(errno::ENOTSOCK)
}
pub fn sock_listen(_fd: i32, _b: u16) -> R<()> {
    Err(errno::ENOTSOCK)
}
pub fn sock_connect(_fd: i32, _a: SocketAddr) -> R<()> {
    Err(errno::ENOTSOCK)
}
pub fn sock_accept(_fd: i32, _c: bool, _n: bool) -> R<(i32, SocketAddr)> {
    Err(errno::ENOTSOCK)
}
pub fn sock_recv(_fd: i32, _b: &mut [u8], _d: bool) -> R<usize> {
    Err(errno::ENOTSOCK)
}
pub fn sock_send(_fd: i32, _b: &[u8], _d: bool) -> R<usize> {
    Err(errno::ENOTSOCK)
}
pub fn sock_recv_from(_fd: i32, _b: &mut [u8], _d: bool) -> R<(usize, SocketAddr)> {
    Err(errno::ENOTSOCK)
}
pub fn sock_send_to(_fd: i32, _b: &[u8], _t: SocketAddr) -> R<usize> {
    Err(errno::ENOTSOCK)
}
pub fn sock_shutdown_write(_fd: i32) -> R<()> {
    Err(errno::ENOTSOCK)
}
pub fn sock_name(_fd: i32, _peer: bool) -> R<SocketAddr> {
    Err(errno::ENOTSOCK)
}
pub fn sock_is_stream(_fd: i32) -> R<(bool, bool)> {
    Err(errno::ENOTSOCK)
}
pub fn sock_set_timeouts(
    _fd: i32,
    _r: Option<Option<core::time::Duration>>,
    _w: Option<Option<core::time::Duration>>,
) -> R<()> {
    Err(errno::ENOTSOCK)
}
pub fn sock_timeouts(_fd: i32) -> R<(Option<core::time::Duration>, Option<core::time::Duration>)> {
    Err(errno::ENOTSOCK)
}
pub fn sock_listening(_fd: i32) -> R<bool> {
    Err(errno::ENOTSOCK)
}
pub fn resolve(_host: &str) -> R<Vec<std::net::IpAddr>> {
    Err(errno::ENOSYS)
}

/// Na hostiteli: soubory a roury vzdy pripravene (testy logiky epoll).
pub fn poll(items: &mut [super::PollItem], _timeout: Option<core::time::Duration>) -> R<usize> {
    let mut n = 0;
    for p in items.iter_mut() {
        p.revents = match get(p.fd) {
            Err(_) => 0x20,
            Ok(d) => match &d.obj {
                Obj::PipeR(_) | Obj::Stdin => p.events & 0x1,
                Obj::PipeW(_) | Obj::Stdout | Obj::Stderr => p.events & 0x4,
                Obj::User(_) => 0,
                _ => p.events & 0x5,
            },
        };
        if p.revents != 0 {
            n += 1;
        }
    }
    Ok(n)
}

pub fn insert_user(obj: Box<dyn core::any::Any + Send + Sync>, cloexec: bool) -> R<i32> {
    insert(Obj::User(obj), super::status::RDWR, 0, cloexec)
}

pub fn with_user<T>(fd: i32, f: impl FnOnce(&(dyn core::any::Any + Send + Sync)) -> R<T>) -> R<T> {
    let d = get(fd)?;
    match &d.obj {
        Obj::User(o) => f(o.as_ref()),
        _ => Err(errno::EINVAL),
    }
}

// --- pamet na hostiteli: alokace std (jen testy logiky mmap) ----------------

const PAGE: u64 = 4096;

static MAPS: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

pub fn page_size() -> u64 {
    PAGE
}

fn host_alloc(len: u64) -> R<u64> {
    let layout = std::alloc::Layout::from_size_align(len as usize, PAGE as usize).map_err(|_| errno::EINVAL)?;
    // SAFETY: nenulova delka (overuje volajici).
    let p = unsafe { std::alloc::alloc_zeroed(layout) };
    if p.is_null() {
        return Err(errno::ENOMEM);
    }
    MAPS.lock().unwrap().push((p as u64, len));
    Ok(p as u64)
}

pub fn vm_alloc(len: u64, _prot: u32) -> R<u64> {
    host_alloc(len)
}

pub fn vm_reserve(len: u64) -> R<u64> {
    host_alloc(len)
}

pub fn vm_reserve_at(_addr: u64, _len: u64) -> R<u64> {
    Err(errno::ENOTSUP)
}

pub fn vm_commit(_addr: u64, _len: u64, _prot: u32) -> R<()> {
    Ok(())
}

pub fn vm_protect(_addr: u64, _len: u64, _prot: u32) -> R<()> {
    Ok(())
}

pub fn vm_release(addr: u64, len: u64) -> R<()> {
    let mut m = MAPS.lock().unwrap();
    let i = m.iter().position(|&(a, l)| a == addr && l == len).ok_or(errno::EINVAL)?;
    m.remove(i);
    let layout = std::alloc::Layout::from_size_align(len as usize, PAGE as usize).map_err(|_| errno::EINVAL)?;
    // SAFETY: alokovano host_alloc se stejnym rozlozenim.
    unsafe { std::alloc::dealloc(addr as *mut u8, layout) };
    Ok(())
}

pub fn vm_decommit(addr: u64, len: u64) -> R<()> {
    // SAFETY: rozsah lezi v alokaci (kontrakt volajiciho).
    unsafe { core::ptr::write_bytes(addr as *mut u8, 0, len as usize) };
    Ok(())
}

pub fn vm_seal(_addr: u64, _len: u64, _prot: u32) -> R<()> {
    Ok(())
}

pub fn vm_query(addr: u64) -> R<super::Region> {
    let m = MAPS.lock().unwrap();
    m.iter()
        .find(|&&(a, l)| addr >= a && addr < a + l)
        .map(|&(a, l)| super::Region { kind: 2, start: a, len: l, prot: 3 })
        .ok_or(errno::ENOMEM)
}

/// Zamky na hostiteli nejsou (logiku testuje `libs/fs`).
pub fn file_lock(_fd: i32, _cmd: u8, _class: u8, _write: bool, _start: u64, _len: u64) -> R<super::LockResult> {
    Err(errno::ENOLCK)
}

// --- procesy (nahrada pro testy tabulky potomku) ---------------------------

/// Nahradni potomek: program `exit:<kod>` skonci hned, `sleep:<ms>:<kod>`
/// po `ms` milisekundach; jine jmeno = `ENOENT`. `kill` = kod
/// `abi::process::EXIT_KILLED`.
pub struct Child {
    /// (konec, kod konce)
    state: Mutex<(std::time::Instant, i64)>,
}

/// Kod konce po `PROCESS_KILL` (`abi::process::EXIT_KILLED`).
pub const HOST_EXIT_KILLED: i64 = -1_000_000;

pub fn spawn_child(r: super::SpawnReq) -> R<Child> {
    let parts: Vec<&str> = r.program.split(':').collect();
    let (ms, code) = match parts.as_slice() {
        ["exit", c] => (0, c.parse().map_err(|_| errno::EINVAL)?),
        ["sleep", ms, c] => (ms.parse().map_err(|_| errno::EINVAL)?, c.parse().map_err(|_| errno::EINVAL)?),
        _ => return Err(errno::ENOENT),
    };
    let end = std::time::Instant::now() + core::time::Duration::from_millis(ms);
    Ok(Child { state: Mutex::new((end, code)) })
}

impl Child {
    pub fn try_exit(&self) -> R<Option<i64>> {
        let (end, code) = *self.state.lock().unwrap();
        Ok((std::time::Instant::now() >= end).then_some(code))
    }

    pub fn wait_exit(&self) -> R<i64> {
        loop {
            if let Some(c) = self.try_exit()? {
                return Ok(c);
            }
            std::thread::sleep(core::time::Duration::from_millis(1));
        }
    }

    pub fn kill(&self) -> R<()> {
        *self.state.lock().unwrap() = (std::time::Instant::now(), HOST_EXIT_KILLED);
        Ok(())
    }
}

pub fn wait_children(children: &[&Child], timeout_ns: Option<u64>) -> R<Option<usize>> {
    let limit = timeout_ns.map(|t| std::time::Instant::now() + core::time::Duration::from_nanos(t));
    loop {
        if let Some(i) = children.iter().position(|c| matches!(c.try_exit(), Ok(Some(_)))) {
            return Ok(Some(i));
        }
        if limit.is_some_and(|l| std::time::Instant::now() >= l) {
            return Ok(None);
        }
        std::thread::sleep(core::time::Duration::from_millis(1));
    }
}

// --- pametovy fd (na hostiteli neni: objekt pameti jadra) --------------------

pub fn is_memfd(_fd: i32) -> bool {
    false
}

pub fn memfd_create(_capacity: u64, _cloexec: bool, _sealing: bool) -> R<i32> {
    Err(errno::ENOSYS)
}

pub fn memfd_size(_fd: i32) -> R<u64> {
    Err(errno::EINVAL)
}

pub fn memfd_truncate(_fd: i32, _len: u64) -> R<()> {
    Err(errno::EINVAL)
}

pub fn memfd_map(_fd: i32, _off: u64, _len: u64, _prot: u32, _addr: u64) -> R<u64> {
    Err(errno::EINVAL)
}

pub fn memfd_add_seals(_fd: i32, _seals: u32) -> R<()> {
    Err(errno::EINVAL)
}

pub fn memfd_seals(_fd: i32) -> R<u32> {
    Err(errno::EINVAL)
}

pub fn memfd_export(_fd: i32, _readonly: bool) -> R<u32> {
    Err(errno::EINVAL)
}

pub fn memfd_import(_handle: u32, _writable: bool, _cloexec: bool) -> R<i32> {
    Err(errno::ENOSYS)
}

pub fn start_handle(_tag: u32) -> Option<u32> {
    None
}

// --- sockety AF_UNIX (jen v systemu; na hostiteli zadne cislo neni AF_UNIX) ---

pub fn is_unix(_fd: i32) -> bool {
    false
}

pub fn unix_socket(_cloexec: bool, _nonblock: bool) -> R<i32> {
    Err(errno::EAFNOSUPPORT)
}

pub fn unix_pair(_cloexec: bool, _nonblock: bool) -> R<(i32, i32)> {
    Err(errno::EAFNOSUPPORT)
}

pub fn unix_bind(_fd: i32, _path: &[u8]) -> R<()> {
    Err(errno::ENOTSOCK)
}

pub fn unix_listen(_fd: i32) -> R<()> {
    Err(errno::ENOTSOCK)
}

pub fn unix_listening(_fd: i32) -> R<bool> {
    Err(errno::ENOTSOCK)
}

pub fn unix_connect(_fd: i32, _path: &[u8]) -> R<()> {
    Err(errno::ENOTSOCK)
}

pub fn unix_accept(_fd: i32, _cloexec: bool, _nonblock: bool) -> R<i32> {
    Err(errno::ENOTSOCK)
}

pub fn unix_send(_fd: i32, _data: &[u8], _fds: &[i32], _dontwait: bool) -> R<usize> {
    Err(errno::ENOTSOCK)
}

pub fn unix_recv(_fd: i32, _buf: &mut [u8], _max_fds: usize, _dontwait: bool, _cloexec: bool) -> R<(usize, Vec<i32>, bool)> {
    Err(errno::ENOTSOCK)
}

pub fn unix_shutdown_write(_fd: i32) -> R<()> {
    Err(errno::ENOTSOCK)
}

pub fn unix_name(_fd: i32, _peer: bool) -> R<Option<Vec<u8>>> {
    Err(errno::ENOTSOCK)
}

pub fn unix_peer(_fd: i32) -> R<(i32, u32, u32)> {
    Err(errno::ENOTSOCK)
}

pub fn user_id() -> u32 {
    0
}

pub fn group_ids() -> Vec<u32> {
    Vec::new()
}
