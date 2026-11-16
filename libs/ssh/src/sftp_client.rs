//! Klient SFTP v3 (bez I/O): pakety pozadavku a rozbor odpovedi. Program
//! `scp` a `sftp` je posila kanalem subsystemu `sftp` (`client.rs`) a ceka
//! na odpoved se stejnym cislem. `scp` jde v systemu jen pres SFTP (jako
//! OpenSSH 9), stary protokol scp ne.

use alloc::string::String;
use alloc::vec::Vec;

use crate::sftp::{MAX_PACKET, VERSION, pflags as open, status, ty as pkt};

/// Kolik bajtu cist / psat jednim paketem.
pub const CHUNK: usize = 32 * 1024;

/// Atributy z odpovedi (jen co klient pouziva).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileAttrs {
    pub size: Option<u64>,
    pub perms: Option<u32>,
    pub mtime: Option<u32>,
}

impl FileAttrs {
    /// Slozka podle bitu typu v pravech POSIX.
    pub fn is_dir(&self) -> bool {
        self.perms.is_some_and(|p| p & 0o170000 == 0o040000)
    }
}

/// Odpoved serveru.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Version(u32),
    Status { id: u32, code: u32, message: String },
    Handle { id: u32, handle: Vec<u8> },
    Data { id: u32, data: Vec<u8> },
    Name { id: u32, entries: Vec<(String, FileAttrs)> },
    Attrs { id: u32, attrs: FileAttrs },
}

impl Reply {
    pub fn id(&self) -> Option<u32> {
        match self {
            Reply::Version(_) => None,
            Reply::Status { id, .. } | Reply::Handle { id, .. } | Reply::Data { id, .. } | Reply::Name { id, .. } | Reply::Attrs { id, .. } => Some(*id),
        }
    }
}

/// Chyba protokolu (spatny paket od serveru).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bad;

struct R<'a>(&'a [u8]);

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Bad> {
        if self.0.len() < n {
            return Err(Bad);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, Bad> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, Bad> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| Bad)?))
    }
    fn u64(&mut self) -> Result<u64, Bad> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| Bad)?))
    }
    fn bytes(&mut self) -> Result<&'a [u8], Bad> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn str(&mut self) -> Result<String, Bad> {
        Ok(String::from_utf8_lossy(self.bytes()?).into_owned())
    }
    fn attrs(&mut self) -> Result<FileAttrs, Bad> {
        let flags = self.u32()?;
        let mut a = FileAttrs::default();
        if flags & 1 != 0 {
            a.size = Some(self.u64()?);
        }
        if flags & 2 != 0 {
            self.u32()?;
            self.u32()?;
        }
        if flags & 4 != 0 {
            a.perms = Some(self.u32()?);
        }
        if flags & 8 != 0 {
            self.u32()?;
            a.mtime = Some(self.u32()?);
        }
        if flags & 0x8000_0000 != 0 {
            let n = self.u32()?;
            for _ in 0..n {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(a)
    }
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}

/// Stav klienta: cislovani pozadavku a prijem paketu z proudu kanalu.
#[derive(Default)]
pub struct SftpClient {
    next: u32,
    buf: Vec<u8>,
}

impl SftpClient {
    pub fn new() -> Self {
        Self { next: 1, buf: Vec::new() }
    }

    /// Paket s delkou: typ, (cislo), telo. Vraci cislo pozadavku.
    fn packet(&mut self, ty: u8, body: &[u8]) -> (u32, Vec<u8>) {
        let id = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        let mut p = Vec::with_capacity(body.len() + 9);
        p.extend_from_slice(&((body.len() + 5) as u32).to_be_bytes());
        p.push(ty);
        p.extend_from_slice(&id.to_be_bytes());
        p.extend_from_slice(body);
        (id, p)
    }

    /// `SSH_FXP_INIT` (verze 3).
    pub fn init(&self) -> Vec<u8> {
        let mut p = Vec::new();
        p.extend_from_slice(&5u32.to_be_bytes());
        p.push(pkt::INIT);
        p.extend_from_slice(&VERSION.to_be_bytes());
        p
    }

    fn path_req(&mut self, ty: u8, path: &str) -> (u32, Vec<u8>) {
        let mut b = Vec::new();
        put_str(&mut b, path.as_bytes());
        self.packet(ty, &b)
    }

    /// Otevreni souboru: `write` = zapis (vytvorit, zkratit), jinak cteni.
    pub fn open(&mut self, path: &str, write: bool) -> (u32, Vec<u8>) {
        let mut b = Vec::new();
        put_str(&mut b, path.as_bytes());
        let flags = if write { open::WRITE | open::CREAT | open::TRUNC } else { open::READ };
        b.extend_from_slice(&flags.to_be_bytes());
        // Prava 644 pro novy soubor.
        b.extend_from_slice(&4u32.to_be_bytes());
        b.extend_from_slice(&0o100644u32.to_be_bytes());
        self.packet(pkt::OPEN, &b)
    }

    pub fn close(&mut self, handle: &[u8]) -> (u32, Vec<u8>) {
        let mut b = Vec::new();
        put_str(&mut b, handle);
        self.packet(pkt::CLOSE, &b)
    }

    pub fn read(&mut self, handle: &[u8], offset: u64, len: u32) -> (u32, Vec<u8>) {
        let mut b = Vec::new();
        put_str(&mut b, handle);
        b.extend_from_slice(&offset.to_be_bytes());
        b.extend_from_slice(&len.to_be_bytes());
        self.packet(pkt::READ, &b)
    }

    pub fn write(&mut self, handle: &[u8], offset: u64, data: &[u8]) -> (u32, Vec<u8>) {
        let mut b = Vec::with_capacity(data.len() + 32);
        put_str(&mut b, handle);
        b.extend_from_slice(&offset.to_be_bytes());
        put_str(&mut b, data);
        self.packet(pkt::WRITE, &b)
    }

    pub fn stat(&mut self, path: &str) -> (u32, Vec<u8>) {
        self.path_req(pkt::STAT, path)
    }

    pub fn opendir(&mut self, path: &str) -> (u32, Vec<u8>) {
        self.path_req(pkt::OPENDIR, path)
    }

    pub fn readdir(&mut self, handle: &[u8]) -> (u32, Vec<u8>) {
        let mut b = Vec::new();
        put_str(&mut b, handle);
        self.packet(pkt::READDIR, &b)
    }

    pub fn remove(&mut self, path: &str) -> (u32, Vec<u8>) {
        self.path_req(pkt::REMOVE, path)
    }

    pub fn mkdir(&mut self, path: &str) -> (u32, Vec<u8>) {
        let mut b = Vec::new();
        put_str(&mut b, path.as_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        self.packet(pkt::MKDIR, &b)
    }

    pub fn realpath(&mut self, path: &str) -> (u32, Vec<u8>) {
        self.path_req(pkt::REALPATH, path)
    }

    /// Data z kanalu subsystemu.
    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Dalsi cela odpoved (`Ok(None)` = jeste neprisla cela).
    pub fn next_reply(&mut self) -> Result<Option<Reply>, Bad> {
        if self.buf.len() < 4 {
            return Ok(None);
        }
        let n = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
        if n == 0 || n > MAX_PACKET {
            return Err(Bad);
        }
        if self.buf.len() < 4 + n {
            return Ok(None);
        }
        let p: Vec<u8> = self.buf.drain(..4 + n).skip(4).collect();
        let mut r = R(&p);
        let ty = r.u8()?;
        let reply = if ty == pkt::VERSION {
            Reply::Version(r.u32()?)
        } else {
            let id = r.u32()?;
            match ty {
                pkt::STATUS => {
                    let code = r.u32()?;
                    // Zprava a jazyk jsou ve v3 nepovinne.
                    let message = if r.0.is_empty() { String::new() } else { r.str()? };
                    Reply::Status { id, code, message }
                }
                pkt::HANDLE => Reply::Handle { id, handle: r.bytes()?.to_vec() },
                pkt::DATA => Reply::Data { id, data: r.bytes()?.to_vec() },
                pkt::NAME => {
                    let n = r.u32()? as usize;
                    if n > MAX_PACKET / 16 {
                        return Err(Bad);
                    }
                    let mut entries = Vec::with_capacity(n);
                    for _ in 0..n {
                        let name = r.str()?;
                        let _long = r.bytes()?;
                        entries.push((name, r.attrs()?));
                    }
                    Reply::Name { id, entries }
                }
                pkt::ATTRS => Reply::Attrs { id, attrs: r.attrs()? },
                _ => return Err(Bad),
            }
        };
        Ok(Some(reply))
    }
}

/// Status OK?
pub fn ok(r: &Reply) -> bool {
    matches!(r, Reply::Status { code: status::OK, .. })
}

/// Status EOF (konec souboru nebo vypisu)?
pub fn eof(r: &Reply) -> bool {
    matches!(r, Reply::Status { code: status::EOF, .. })
}

/// Text chyby ze statusu pro cloveka.
pub fn describe(r: &Reply) -> String {
    match r {
        Reply::Status { code, message, .. } => {
            let what = match *code {
                status::NO_SUCH_FILE => "soubor neexistuje",
                status::PERMISSION_DENIED => "nemáte oprávnění",
                status::OP_UNSUPPORTED => "server operaci neumí",
                _ => "chyba serveru",
            };
            if message.is_empty() { String::from(what) } else { alloc::format!("{what} ({message})") }
        }
        _ => String::from("neočekávaná odpověď serveru"),
    }
}
