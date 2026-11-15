//! Dedeni fd potomkem (`posix_spawn`, docs/cs/cstd.md, kapitola Procesy).
//!
//! Rodic popis otevreni prevede na polozku bloku parametru
//! (`abi::params::Fd`) a pripadny handle ([`export`]); potomek je pri
//! startu vlozi do sve tabulky fd na stejna cisla ([`import`]). Kazdy
//! druh ma vlastni podobu:
//!
//! - roura a stdio rodice pod cislem nad 2: duplikat kanalu proudu
//!   (potomek i rodic ctou/pisou tentyz kanal),
//! - pametovy fd: duplikat objektu pameti s pecetmi a kopii pozice,
//! - soubor a adresar: trvala identita objektu (dataset, oid, generace);
//!   potomek ho otevre ve sve relaci FS (`REOPEN`, prava overi fsd znovu)
//!   na pozici rodice. Pozice se **nesdili** (POSIX ji sdili): kazdy proces
//!   ma od spusteni vlastni.
//!
//! - prazdny fd (`/dev/null`, stdio rodice bez kanalu): bez handlu,
//!   potomek ma zase `/dev/null`,
//! - socket `AF_UNIX`: duplikat kanalu spojeni nebo posluchace (stejny
//!   tvar jde i v `SCM_RIGHTS`, [`crate::unix`]).
//!
//! Sockety TCP/UDP a objekty knihoven (`epoll`) zdedit nejde
//! (`NotSupported`): spojeni s netd patri relaci rodice.

use alloc::string::String;
use alloc::vec::Vec;

use abi::params::fd_kind;
use libos::params::Fd;
use libos::{Channel, Handle};

use crate::fd::{Description, Object};
use crate::fs::File;
use crate::memfd::Memory;
use crate::pipe::Pipe;
use crate::stdio::Inherit;
use crate::{Error, Result};

/// Data polozky: cisla little-endian za sebou (format `abi::params::fd_kind`).
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let (a, b) = self.0.split_at_checked(n).ok_or(Error::Corrupted)?;
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| Error::Corrupted)?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| Error::Corrupted)?))
    }
}

/// Zdedeny fd pripraveny pro [`crate::process::spawn_with_fds`].
pub struct Export {
    pub entry: Fd,
    /// Handly polozky v poradi (`abi::params::fd_kind::has_handle`: jeden;
    /// socket `AF_UNIX` navic handly zprav z fronty, pocet je v datech).
    pub handles: Vec<Handle>,
}

/// Prevede popis rodice na fd `child_fd` potomka. `NotSupported` = druh
/// zdedit nejde (socket, `epoll`, stdio bez kanalu).
pub fn export(child_fd: i32, desc: &Description) -> Result<Export> {
    let fd = u32::try_from(child_fd).map_err(|_| Error::InvalidArgs)?;
    let status = desc.status();
    let entry = |kind, data| Fd { fd, kind, status, data };
    Ok(match &desc.object {
        Object::Pipe(p) => Export { entry: entry(fd_kind::PIPE, Vec::new()), handles: alloc::vec![p.try_clone()?.into_channel().into_handle()] },
        Object::Stdio(s) => match crate::stdio::inherit(*s)? {
            Inherit::Channel(c) => Export { entry: entry(fd_kind::PIPE, Vec::new()), handles: alloc::vec![c.into_handle()] },
            // Proud bez kanalu (zahozeny nebo zadny): jako /dev/null.
            Inherit::Null | Inherit::None => Export { entry: entry(fd_kind::NULL, Vec::new()), handles: Vec::new() },
        },
        Object::Null => Export { entry: entry(fd_kind::NULL, Vec::new()), handles: Vec::new() },
        Object::Memory(m) => {
            let mut w = Vec::with_capacity(13);
            w.push(u8::from(m.writable()));
            w.extend_from_slice(&m.seals().to_le_bytes());
            w.extend_from_slice(&m.tell().to_le_bytes());
            Export { entry: entry(fd_kind::MEMORY, w), handles: alloc::vec![m.export(false)?] }
        }
        Object::File(f) => {
            let id = f.identity()?;
            let mut w = Vec::with_capacity(35 + f.path().len());
            for v in [id.dataset, id.oid, id.generation] {
                w.extend_from_slice(&v.to_le_bytes());
            }
            w.extend_from_slice(&[id.access, id.share]);
            w.extend_from_slice(&f.tell().to_le_bytes());
            w.push(u8::from(f.is_append()));
            w.extend_from_slice(f.path().as_bytes());
            Export { entry: entry(fd_kind::FILE, w), handles: Vec::new() }
        }
        Object::Unix(u) => {
            let (data, handles) = u.export()?;
            Export { entry: entry(fd_kind::UNIX, data), handles }
        }
        Object::Socket(_) | Object::User(_) => return Err(Error::NotSupported),
    })
}

/// Popis z polozky; handly bere z `handles` v poradi (kolik druh potrebuje).
pub(crate) fn object(e: &Fd, handles: &mut dyn Iterator<Item = Handle>) -> Result<Object> {
    let mut r = Reader(&e.data);
    let handle = if fd_kind::has_handle(e.kind) && e.kind != fd_kind::UNIX { handles.next() } else { None };
    match e.kind {
        fd_kind::PIPE => Ok(Object::Pipe(Pipe::from_channel(Channel::from_handle(handle.ok_or(Error::Corrupted)?)))),
        fd_kind::MEMORY => {
            let (writable, seals, pos) = (r.u8()? != 0, r.u32()?, r.u64()?);
            Ok(Object::Memory(Memory::inherited(handle.ok_or(Error::Corrupted)?, writable, seals, pos)))
        }
        fd_kind::FILE => {
            let id = fsproto::v2::Identity {
                dataset: r.u64()?,
                oid: r.u64()?,
                generation: r.u64()?,
                access: r.u8()?,
                share: r.u8()?,
            };
            let (pos, append) = (r.u64()?, r.u8()? != 0);
            let path = String::from_utf8(r.0.to_vec()).map_err(|_| Error::Corrupted)?;
            Ok(Object::File(File::from_identity(&id, pos, append, path)?))
        }
        fd_kind::NULL => Ok(Object::Null),
        fd_kind::UNIX => Ok(Object::Unix(crate::unix::UnixSocket::import(&e.data, handles)?)),
        _ => Err(Error::NotSupported),
    }
}

/// Vlozi zdedene fd do tabulky (pri startu, po relaci FS). Polozka, ktera
/// nejde obnovit (soubor mezitim smazany, neznamy druh), se preskoci:
/// cislo zustane volne (C `EBADF`). Handly jsou v poradi polozek s handlem.
pub(crate) fn import(fds: &[Fd], handles: Vec<Handle>) {
    let mut handles = handles.into_iter();
    for e in fds {
        // Handly polozky se vezmou vzdy (i kdyz ji nejde obnovit), at dalsi
        // polozky dostanou sve.
        let obj = object(e, &mut handles);
        // Stdio potomka (0-2) preda launchd jako proudy, ne polozky.
        if e.fd <= 2 {
            continue;
        }
        if let Ok(obj) = obj {
            let _ = crate::fd::install(e.fd as i32, Description::new(obj, e.status));
        }
    }
}
