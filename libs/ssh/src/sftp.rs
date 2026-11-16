//! Server SFTP verze 3 (draft-ietf-secsh-filexfer-02) nad souborovym
//! systemem uctu ([`SftpFs`]). Slouzi `sftp` i `scp` (OpenSSH 9+ kopiruje
//! pres SFTP).
//!
//! - Cesty: absolutni, nebo vuci domovu uctu; `.` a `..` se rozlozi
//!   (nad koren nejde). Prava vynucuje FS (identita relace), ne server.
//! - Handle = cislo (soubor nebo vypis slozky); nejvys [`MAX_HANDLES`].
//! - Odkazy, `setstat` (prava, casy) a rozsireni: odpoved OK bez zmeny
//!   (scp nastavuje casy; FS je spravuje sam), `symlink` nepodporovan.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Error;

pub const VERSION: u32 = 3;
/// Nejvic otevrenych handlu na relaci.
pub const MAX_HANDLES: usize = 64;
/// Nejvetsi paket od klienta (OpenSSH posila do 256 KiB).
pub const MAX_PACKET: usize = 256 * 1024 + 64;
/// Nejvic dat v jedne odpovedi READ.
const MAX_READ: usize = 64 * 1024;

mod ty {
    pub const INIT: u8 = 1;
    pub const VERSION: u8 = 2;
    pub const OPEN: u8 = 3;
    pub const CLOSE: u8 = 4;
    pub const READ: u8 = 5;
    pub const WRITE: u8 = 6;
    pub const LSTAT: u8 = 7;
    pub const FSTAT: u8 = 8;
    pub const SETSTAT: u8 = 9;
    pub const FSETSTAT: u8 = 10;
    pub const OPENDIR: u8 = 11;
    pub const READDIR: u8 = 12;
    pub const REMOVE: u8 = 13;
    pub const MKDIR: u8 = 14;
    pub const RMDIR: u8 = 15;
    pub const REALPATH: u8 = 16;
    pub const STAT: u8 = 17;
    pub const RENAME: u8 = 18;
    pub const STATUS: u8 = 101;
    pub const HANDLE: u8 = 102;
    pub const DATA: u8 = 103;
    pub const NAME: u8 = 104;
    pub const ATTRS: u8 = 105;
}

/// Kody `SSH_FXP_STATUS`.
pub mod status {
    pub const OK: u32 = 0;
    pub const EOF: u32 = 1;
    pub const NO_SUCH_FILE: u32 = 2;
    pub const PERMISSION_DENIED: u32 = 3;
    pub const FAILURE: u32 = 4;
    pub const BAD_MESSAGE: u32 = 5;
    pub const OP_UNSUPPORTED: u32 = 8;
}

mod pflags {
    pub const WRITE: u32 = 2;
    pub const APPEND: u32 = 4;
    pub const CREAT: u32 = 8;
    pub const TRUNC: u32 = 0x10;
    pub const EXCL: u32 = 0x20;
}

/// Vlastnosti souboru (podmnozina SFTP v3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs {
    pub dir: bool,
    pub size: u64,
    /// Bity prav POSIX (`rwxrwxrwx`); `None` = vychozi 644 / 755.
    pub mode: Option<u16>,
    /// Cas zmeny (s od 1970).
    pub mtime: u32,
}

/// Souborovy system uctu (v systemu relace FS z initu).
pub trait SftpFs {
    fn stat(&mut self, path: &str) -> Result<Attrs, Error>;
    fn list(&mut self, path: &str) -> Result<Vec<(String, Attrs)>, Error>;
    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>, Error>;
    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<(), Error>;
    /// Novy prazdny soubor (`AlreadyExists`, kdyz je).
    fn create(&mut self, path: &str) -> Result<(), Error>;
    fn truncate(&mut self, path: &str, size: u64) -> Result<(), Error>;
    fn mkdir(&mut self, path: &str) -> Result<(), Error>;
    /// Smaze soubor nebo prazdnou slozku.
    fn remove(&mut self, path: &str) -> Result<(), Error>;
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Error>;
}

enum Handle {
    File { path: String, append: bool },
    Dir { entries: Vec<(String, Attrs)>, done: bool },
}

pub struct Sftp {
    home: String,
    buf: Vec<u8>,
    handles: BTreeMap<u32, Handle>,
    next: u32,
    initialized: bool,
}

/// Normalizace cesty vuci `home`: absolutni bez `.` a `..`.
pub fn resolve(home: &str, p: &str) -> String {
    let base = if p.starts_with('/') { "" } else { home };
    let mut parts: Vec<&str> = Vec::new();
    for c in base.split('/').chain(p.split('/')) {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let mut out = String::new();
    for c in &parts {
        out.push('/');
        out.push_str(c);
    }
    if out.is_empty() {
        out.push('/');
    }
    out
}

fn code_of(e: Error) -> u32 {
    match e {
        Error::NotFound => status::NO_SUCH_FILE,
        Error::AccessDenied | Error::ReadOnly => status::PERMISSION_DENIED,
        Error::NotSupported => status::OP_UNSUPPORTED,
        Error::InvalidArgs => status::BAD_MESSAGE,
        _ => status::FAILURE,
    }
}

/// Cteni paketu.
struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn u8(&mut self) -> Result<u8, Error> {
        let (&b, r) = self.0.split_first().ok_or(Error::InvalidArgs)?;
        self.0 = r;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let (v, r) = self.0.split_first_chunk::<4>().ok_or(Error::InvalidArgs)?;
        self.0 = r;
        Ok(u32::from_be_bytes(*v))
    }
    fn u64(&mut self) -> Result<u64, Error> {
        let (v, r) = self.0.split_first_chunk::<8>().ok_or(Error::InvalidArgs)?;
        self.0 = r;
        Ok(u64::from_be_bytes(*v))
    }
    fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let n = self.u32()? as usize;
        if n > self.0.len() {
            return Err(Error::InvalidArgs);
        }
        let (v, r) = self.0.split_at(n);
        self.0 = r;
        Ok(v)
    }
    fn str(&mut self) -> Result<&'a str, Error> {
        core::str::from_utf8(self.bytes()?).map_err(|_| Error::InvalidArgs)
    }
    /// Atributy od klienta (jen preskocit; FS je spravuje sam).
    fn attrs(&mut self) -> Result<(), Error> {
        let f = self.u32()?;
        if f & 1 != 0 {
            self.u64()?;
        }
        if f & 2 != 0 {
            self.u32()?;
            self.u32()?;
        }
        if f & 4 != 0 {
            self.u32()?;
        }
        if f & 8 != 0 {
            self.u32()?;
            self.u32()?;
        }
        if f & 0x8000_0000 != 0 {
            let n = self.u32()?;
            for _ in 0..n.min(64) {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(())
    }
}

/// Zapis odpovedi.
struct W(Vec<u8>);

impl W {
    fn new(t: u8, id: u32) -> Self {
        let mut v = alloc::vec![0, 0, 0, 0, t];
        v.extend_from_slice(&id.to_be_bytes());
        W(v)
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
        self
    }
    fn attrs(&mut self, a: &Attrs) -> &mut Self {
        let perm = a.mode.unwrap_or(if a.dir { 0o755 } else { 0o644 }) as u32;
        let ty = if a.dir { 0o040000 } else { 0o100000 };
        self.u32(1 | 4 | 8).u64(a.size).u32(ty | perm).u32(a.mtime).u32(a.mtime)
    }
    fn done(mut self) -> Vec<u8> {
        let n = (self.0.len() - 4) as u32;
        self.0[..4].copy_from_slice(&n.to_be_bytes());
        self.0
    }
}

fn st(id: u32, code: u32, msg: &str) -> Vec<u8> {
    let mut w = W::new(ty::STATUS, id);
    w.u32(code).bytes(msg.as_bytes()).bytes(b"");
    w.done()
}

/// Radek jako `ls -l` (sftp `ls -l` ho ukazuje).
fn longname(name: &str, a: &Attrs) -> String {
    let perm = a.mode.unwrap_or(if a.dir { 0o755 } else { 0o644 });
    let mut s = String::from(if a.dir { "d" } else { "-" });
    for i in (0..9).rev() {
        let bit = perm & (1 << i) != 0;
        s.push(if !bit { '-' } else { ['x', 'w', 'r'][i % 3] });
    }
    alloc::format!("{s} 1 user user {:>10} {name}", a.size)
}

impl Sftp {
    pub fn new(home: &str) -> Self {
        Self { home: String::from(home), buf: Vec::new(), handles: BTreeMap::new(), next: 1, initialized: false }
    }

    /// Data od klienta; vraci odpovedi (bajty pro kanal). `Err` = porusene
    /// spojeni (prilis velky paket): kanal zavrit.
    pub fn input(&mut self, data: &[u8], fs: &mut dyn SftpFs) -> Result<Vec<u8>, Error> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            let Some(len) = self.buf.first_chunk::<4>().map(|b| u32::from_be_bytes(*b) as usize) else { break };
            if len > MAX_PACKET || len == 0 {
                return Err(Error::InvalidArgs);
            }
            if self.buf.len() < 4 + len {
                break;
            }
            let pkt: Vec<u8> = self.buf.drain(..4 + len).skip(4).collect();
            out.extend(self.packet(&pkt, fs));
        }
        Ok(out)
    }

    fn handle_id(&mut self, h: Handle) -> Option<u32> {
        if self.handles.len() >= MAX_HANDLES {
            return None;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        self.handles.insert(id, h);
        Some(id)
    }

    fn handle_reply(&mut self, id: u32, h: Handle) -> Vec<u8> {
        match self.handle_id(h) {
            Some(hid) => {
                let mut w = W::new(ty::HANDLE, id);
                w.bytes(&hid.to_be_bytes());
                w.done()
            }
            None => st(id, status::FAILURE, "prilis mnoho otevrenych souboru"),
        }
    }

    fn packet(&mut self, p: &[u8], fs: &mut dyn SftpFs) -> Vec<u8> {
        let mut r = R(p);
        let Ok(t) = r.u8() else { return Vec::new() };
        if t == ty::INIT {
            self.initialized = true;
            let mut w = W(alloc::vec![0, 0, 0, 0, ty::VERSION]);
            w.u32(VERSION);
            return w.done();
        }
        let Ok(id) = r.u32() else { return Vec::new() };
        if !self.initialized {
            return st(id, status::BAD_MESSAGE, "chybi init");
        }
        match self.op(t, id, &mut r, fs) {
            Ok(v) => v,
            Err(e) => st(id, code_of(e), e.name()),
        }
    }

    fn path(&self, r: &mut R) -> Result<String, Error> {
        Ok(resolve(&self.home, r.str()?))
    }

    fn hid(r: &mut R) -> Result<u32, Error> {
        let b = r.bytes()?;
        Ok(u32::from_be_bytes(b.try_into().map_err(|_| Error::InvalidArgs)?))
    }

    fn op(&mut self, t: u8, id: u32, r: &mut R, fs: &mut dyn SftpFs) -> Result<Vec<u8>, Error> {
        match t {
            ty::REALPATH => {
                let p = self.path(r)?;
                let mut w = W::new(ty::NAME, id);
                w.u32(1).bytes(p.as_bytes()).bytes(p.as_bytes()).attrs(&Attrs { dir: true, ..Default::default() });
                Ok(w.done())
            }
            ty::STAT | ty::LSTAT => {
                let a = fs.stat(&self.path(r)?)?;
                let mut w = W::new(ty::ATTRS, id);
                w.attrs(&a);
                Ok(w.done())
            }
            ty::FSTAT => {
                let h = Self::hid(r)?;
                let a = match self.handles.get(&h) {
                    Some(Handle::File { path, .. }) => fs.stat(path)?,
                    Some(Handle::Dir { .. }) => Attrs { dir: true, ..Default::default() },
                    None => return Err(Error::InvalidArgs),
                };
                let mut w = W::new(ty::ATTRS, id);
                w.attrs(&a);
                Ok(w.done())
            }
            ty::OPEN => {
                let path = self.path(r)?;
                let flags = r.u32()?;
                r.attrs()?;
                let exists = fs.stat(&path).map(|a| !a.dir);
                if flags & pflags::CREAT != 0 {
                    match exists {
                        Ok(true) if flags & pflags::EXCL != 0 => return Err(Error::AlreadyExists),
                        Ok(true) => {}
                        _ => fs.create(&path)?,
                    }
                } else if !exists? {
                    return Err(Error::InvalidArgs);
                }
                if flags & pflags::TRUNC != 0 && flags & pflags::WRITE != 0 {
                    fs.truncate(&path, 0)?;
                }
                Ok(self.handle_reply(id, Handle::File { path, append: flags & pflags::APPEND != 0 }))
            }
            ty::CLOSE => {
                let h = Self::hid(r)?;
                self.handles.remove(&h).ok_or(Error::InvalidArgs)?;
                Ok(st(id, status::OK, ""))
            }
            ty::READ => {
                let h = Self::hid(r)?;
                let (off, len) = (r.u64()?, r.u32()? as usize);
                let Some(Handle::File { path, .. }) = self.handles.get(&h) else { return Err(Error::InvalidArgs) };
                let data = fs.read(path, off, len.min(MAX_READ))?;
                if data.is_empty() {
                    return Ok(st(id, status::EOF, "konec souboru"));
                }
                let mut w = W::new(ty::DATA, id);
                w.bytes(&data);
                Ok(w.done())
            }
            ty::WRITE => {
                let h = Self::hid(r)?;
                let off = r.u64()?;
                let data = r.bytes()?;
                let Some(Handle::File { path, append }) = self.handles.get(&h) else { return Err(Error::InvalidArgs) };
                let off = if *append { fs.stat(path)?.size } else { off };
                fs.write(path, off, data)?;
                Ok(st(id, status::OK, ""))
            }
            ty::OPENDIR => {
                let path = self.path(r)?;
                let mut entries = fs.list(&path)?;
                entries.insert(0, (String::from(".."), Attrs { dir: true, ..Default::default() }));
                entries.insert(0, (String::from("."), Attrs { dir: true, ..Default::default() }));
                Ok(self.handle_reply(id, Handle::Dir { entries, done: false }))
            }
            ty::READDIR => {
                let h = Self::hid(r)?;
                let Some(Handle::Dir { entries, done }) = self.handles.get_mut(&h) else { return Err(Error::InvalidArgs) };
                if *done || entries.is_empty() {
                    return Ok(st(id, status::EOF, "konec vypisu"));
                }
                // Po davkach (paket ma mez); 100 polozek je bezpecne.
                let n = entries.len().min(100);
                let chunk: Vec<(String, Attrs)> = entries.drain(..n).collect();
                if entries.is_empty() {
                    *done = true;
                }
                let mut w = W::new(ty::NAME, id);
                w.u32(chunk.len() as u32);
                for (name, a) in &chunk {
                    w.bytes(name.as_bytes()).bytes(longname(name, a).as_bytes()).attrs(a);
                }
                Ok(w.done())
            }
            ty::REMOVE | ty::RMDIR => {
                fs.remove(&self.path(r)?)?;
                Ok(st(id, status::OK, ""))
            }
            ty::MKDIR => {
                fs.mkdir(&self.path(r)?)?;
                Ok(st(id, status::OK, ""))
            }
            ty::RENAME => {
                let from = self.path(r)?;
                let to = self.path(r)?;
                fs.rename(&from, &to)?;
                Ok(st(id, status::OK, ""))
            }
            ty::SETSTAT | ty::FSETSTAT => Ok(st(id, status::OK, "")),
            _ => Ok(st(id, status::OP_UNSUPPORTED, "nepodporovano")),
        }
    }
}
