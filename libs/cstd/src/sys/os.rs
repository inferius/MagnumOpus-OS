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
    }
}

pub fn stat(path: &[u8]) -> R<Stat> {
    rt::fs::stat_path(path).map(|s| stat_of(&s)).map_err(e)
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
