//! Klient pro Windows: posila pozadavky pres `DeviceIoControl` na handle
//! korene pripojene jednotky.
//!
//! ```no_run
//! use osfs_proto::{PathArg, client::Client};
//!
//! let root = std::path::Path::new("Q:\\");
//! if Client::is_osfs(root) {
//!     let mut c = Client::open(root)?;
//!     for e in c.list_all("", &PathArg::from_windows_full("Q:\\Fotky"))? {
//!         println!("{} {} B (podstrom {} B)", e.name, e.size, e.tree_bytes);
//!     }
//! }
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! Handle je jeden na klienta; `Client` neni `Sync` - pro vic vlaken si
//! kazde otevre vlastni (otevreni je levne).

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use crate::{
    error, EntryInfo, HelloInfo, PathArg, Request, Response, SnapshotInfo, VolumeInfo, MAX_MESSAGE,
    MAX_READ,
};

type Handle = *mut c_void;

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn DeviceIoControl(
        handle: Handle,
        code: u32,
        input: *const c_void,
        input_len: u32,
        output: *mut c_void,
        output_len: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetVolumeInformationW(
        root: *const u16,
        label: *mut u16,
        label_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        fs_name: *mut u16,
        fs_name_len: u32,
    ) -> i32;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const INVALID_HANDLE: Handle = -1isize as Handle;

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Chyba serveru -> `io::Error` (kod z `error`, citelne jmeno ve zprave).
pub fn server_error(code: i32) -> io::Error {
    let kind = match code {
        error::NOT_FOUND => io::ErrorKind::NotFound,
        error::ALREADY_EXISTS => io::ErrorKind::AlreadyExists,
        error::ACCESS_DENIED | error::READ_ONLY => io::ErrorKind::PermissionDenied,
        error::INVALID_ARGS => io::ErrorKind::InvalidInput,
        error::NOT_SUPPORTED => io::ErrorKind::Unsupported,
        error::CORRUPTED => io::ErrorKind::InvalidData,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("osfs: {} ({code})", error::name(code)))
}

/// Spojeni na jednu jednotku.
pub struct Client {
    handle: Handle,
    writable: bool,
}

// Handle souboru jde pouzivat z jineho vlakna (ne soucasne - proto ne Sync).
unsafe impl Send for Client {}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: platny handle z CreateFileW, zaviram jednou.
        unsafe { CloseHandle(self.handle) };
    }
}

impl Client {
    /// Hlasi jednotka nas souborovy system? (`root` = `Q:\`.)
    pub fn is_osfs(root: &Path) -> bool {
        let w = wide(root);
        let mut name = [0u16; 64];
        // SAFETY: buffery a delky odpovidaji, nepotrebne vystupy null.
        let ok = unsafe {
            GetVolumeInformationW(
                w.as_ptr(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                name.as_mut_ptr(),
                name.len() as u32,
            )
        };
        if ok == 0 {
            return false;
        }
        let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        String::from_utf16_lossy(&name[..n]) == crate::FS_NAME
    }

    /// Otevre koren jednotky (`Q:\`). Zkusi pristup i pro zapis (operace
    /// jako `rename` nebo `snapshot_create`); kdyz nejde, otevre jen pro cteni.
    pub fn open(root: &Path) -> io::Result<Self> {
        let w = wide(root);
        for (access, writable) in [(GENERIC_READ | GENERIC_WRITE, true), (GENERIC_READ, false)] {
            // SAFETY: platna cesta zakoncena nulou.
            let h = unsafe {
                CreateFileW(
                    w.as_ptr(),
                    access,
                    FILE_SHARE_ALL,
                    std::ptr::null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    std::ptr::null_mut(),
                )
            };
            if h != INVALID_HANDLE && !h.is_null() {
                return Ok(Self {
                    handle: h,
                    writable,
                });
            }
        }
        Err(io::Error::last_os_error())
    }

    /// Posle pozadavek a vrati odpoved.
    pub fn call(&mut self, req: &Request) -> io::Result<Response> {
        if req.is_write() && !self.writable {
            return Err(server_error(error::ACCESS_DENIED));
        }
        let input = req.encode();
        if input.len() > MAX_MESSAGE {
            return Err(server_error(error::INVALID_ARGS));
        }
        let mut out = vec![0u8; MAX_MESSAGE];
        let mut got: u32 = 0;
        // SAFETY: buffery a delky odpovidaji; synchronni handle.
        let ok = unsafe {
            DeviceIoControl(
                self.handle,
                req.ioctl(),
                input.as_ptr().cast(),
                input.len() as u32,
                out.as_mut_ptr().cast(),
                out.len() as u32,
                &mut got,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        out.truncate(got as usize);
        Response::decode(req.op(), &out).map_err(server_error)
    }

    fn unexpected() -> io::Error {
        server_error(error::CORRUPTED)
    }

    pub fn hello(&mut self) -> io::Result<HelloInfo> {
        match self.call(&Request::Hello)? {
            Response::Hello(h) => Ok(h),
            _ => Err(Self::unexpected()),
        }
    }

    pub fn volume_info(&mut self) -> io::Result<VolumeInfo> {
        match self.call(&Request::VolumeInfo)? {
            Response::VolumeInfo(v) => Ok(v),
            _ => Err(Self::unexpected()),
        }
    }

    /// Metadata objektu; `at` = jmeno snapshotu nebo "" pro zivy stav.
    pub fn stat(&mut self, at: &str, path: &PathArg) -> io::Result<EntryInfo> {
        match self.call(&Request::Stat {
            at: at.into(),
            path: path.clone(),
        })? {
            Response::Entry(e) => Ok(e),
            _ => Err(Self::unexpected()),
        }
    }

    /// Cely vypis adresare (vsechny stranky).
    pub fn list_all(&mut self, at: &str, path: &PathArg) -> io::Result<Vec<EntryInfo>> {
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            match self.call(&Request::List {
                at: at.into(),
                path: path.clone(),
                cursor: cursor.clone(),
            })? {
                Response::List(p) => {
                    out.extend(p.entries);
                    if p.next.is_empty() || p.next == cursor {
                        return Ok(out);
                    }
                    cursor = p.next;
                }
                _ => return Err(Self::unexpected()),
            }
        }
    }

    /// Cast souboru od `offset` (vrati mene, kdyz soubor konci).
    pub fn read(
        &mut self,
        at: &str,
        path: &PathArg,
        offset: u64,
        len: usize,
    ) -> io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let want = (len - out.len()).min(MAX_READ as usize) as u32;
            match self.call(&Request::Read {
                at: at.into(),
                path: path.clone(),
                offset: offset + out.len() as u64,
                len: want,
            })? {
                Response::Data(d) => {
                    let n = d.len();
                    out.extend(d);
                    if n < want as usize {
                        break;
                    }
                }
                _ => return Err(Self::unexpected()),
            }
        }
        Ok(out)
    }

    pub fn rename(&mut self, from: &PathArg, to: &PathArg) -> io::Result<()> {
        match self.call(&Request::Rename {
            from: from.clone(),
            to: to.clone(),
        })? {
            Response::Done(_) => Ok(()),
            _ => Err(Self::unexpected()),
        }
    }

    /// Vsechny snapshoty datasetu, ve kterem lezi `path` (vsechny stranky).
    pub fn snapshots(&mut self, path: &PathArg) -> io::Result<Vec<SnapshotInfo>> {
        let mut out = Vec::new();
        let mut after = 0;
        loop {
            match self.call(&Request::Snapshots {
                path: path.clone(),
                after,
            })? {
                Response::Snapshots { list, next } => {
                    out.extend(list);
                    if next == 0 || next <= after {
                        return Ok(out);
                    }
                    after = next;
                }
                _ => return Err(Self::unexpected()),
            }
        }
    }

    /// Vytvori snapshot datasetu, ve kterem lezi `path`; vrati jeho id.
    pub fn snapshot_create(&mut self, path: &PathArg, name: &str) -> io::Result<u64> {
        match self.call(&Request::SnapshotCreate {
            path: path.clone(),
            name: name.into(),
        })? {
            Response::Created(id) => Ok(id),
            _ => Err(Self::unexpected()),
        }
    }

    /// Ulozi zmeny hned; vrati generaci ulozeneho stavu.
    pub fn commit(&mut self) -> io::Result<u64> {
        match self.call(&Request::Commit)? {
            Response::Done(g) => Ok(g),
            _ => Err(Self::unexpected()),
        }
    }
}
