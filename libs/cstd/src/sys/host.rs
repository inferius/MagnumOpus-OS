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

pub fn isatty(fd: i32) -> R<bool> {
    let d = get(fd)?;
    Ok(matches!(d.obj, Obj::Stdin | Obj::Stdout | Obj::Stderr))
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
