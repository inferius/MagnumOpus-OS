//! Obsluha jednotlivych operaci WinFsp (`FSP_FILE_SYSTEM_INTERFACE`).
//!
//! Kazda funkce si vezme zamek (`with_state`), prelozi jmeno z Windows
//! (`names::path_from_windows`) a zavola verejne API `fs::Fs`. Kontext
//! souboru (`FileContext`) je cislo handlu v `State::handles`.

use std::ffi::c_void;

use fs::{Error, Kind, Stat, fold_name};

use super::ffi::*;
use super::{Handle, SD, State, debug, with_state};
use crate::image::now_ns;
use crate::names::{self, to_windows};

/// Rozdil mezi 1. 1. 1601 (FILETIME) a 1. 1. 1970 v 100 ns.
const EPOCH_DIFF: u64 = 116_444_736_000_000_000;

/// ns od 1970 -> FILETIME (100 ns od 1601).
pub fn filetime(ns: u64) -> u64 {
    ns / 100 + EPOCH_DIFF
}

pub fn interface() -> Interface {
    Interface {
        GetVolumeInfo: Some(get_volume_info),
        SetVolumeLabel: None,
        GetSecurityByName: Some(get_security_by_name),
        Create: Some(create),
        Open: Some(open),
        Overwrite: Some(overwrite),
        Cleanup: Some(cleanup),
        Close: Some(close),
        Read: Some(read),
        Write: Some(write),
        Flush: Some(flush),
        GetFileInfo: Some(get_file_info),
        SetBasicInfo: Some(set_basic_info),
        SetFileSize: Some(set_file_size),
        CanDelete: Some(can_delete),
        Rename: Some(rename),
        GetSecurity: Some(get_security),
        SetSecurity: None,
        ReadDirectory: Some(read_directory),
        ResolveReparsePoints: None,
        GetReparsePoint: None,
        SetReparsePoint: None,
        DeleteReparsePoint: None,
        GetStreamInfo: None,
        GetDirInfoByName: None,
        Control: None,
        SetDelete: None,
        CreateEx: None,
        OverwriteEx: None,
        GetEa: None,
        SetEa: None,
        Obsolete0: None,
        DispatcherStopped: None,
        Reserved: [None; 31],
    }
}

// --- prevody ---

/// Chyba knihovny -> NTSTATUS.
pub fn status(e: Error) -> NTSTATUS {
    match e {
        Error::NotFound => STATUS_OBJECT_NAME_NOT_FOUND,
        Error::AlreadyExists => STATUS_OBJECT_NAME_COLLISION,
        Error::NotADirectory => STATUS_NOT_A_DIRECTORY,
        Error::IsADirectory => STATUS_FILE_IS_A_DIRECTORY,
        Error::NotEmpty => STATUS_DIRECTORY_NOT_EMPTY,
        Error::NoSpace => STATUS_DISK_FULL,
        Error::ReadOnly => STATUS_MEDIA_WRITE_PROTECTED,
        Error::InvalidArgs => STATUS_OBJECT_NAME_INVALID,
        Error::AccessDenied => STATUS_ACCESS_DENIED,
        Error::NotSupported => STATUS_NOT_SUPPORTED,
        Error::Corrupted => STATUS_FILE_CORRUPT_ERROR,
        Error::IoError => STATUS_IO_DEVICE_ERROR,
        Error::NoMemory => STATUS_NO_MEMORY,
        Error::NoResources | Error::ShouldWait => STATUS_INSUFFICIENT_RESOURCES,
        Error::OutOfRange => STATUS_INVALID_PARAMETER,
        _ => STATUS_UNSUCCESSFUL,
    }
}

/// Retezec z WinFsp (UTF-16 s nulou). Nesparovane surogaty -> `None`.
unsafe fn win_string(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: WinFsp predava retezce zakoncene nulou.
    let n = unsafe { wcslen(p) };
    // SAFETY: n jednotek pred nulou je platnych.
    names::utf16_to_string(unsafe { std::slice::from_raw_parts(p, n) })
}

/// Jak zachazet se jmenem, ktere nejde prelozit.
#[derive(Clone, Copy)]
enum Want {
    /// Otevreni existujiciho: jmeno z Windows neodpovida zadne zobrazene
    /// polozce -> "nenalezeno".
    Existing,
    /// Vytvoreni nebo cil prejmenovani: "neplatne jmeno".
    New,
}

/// Cesta z WinFsp -> cesta v nasem FS.
unsafe fn fs_path(p: *const u16, want: Want) -> Result<String, NTSTATUS> {
    let bad = match want {
        Want::Existing => STATUS_OBJECT_NAME_NOT_FOUND,
        Want::New => STATUS_OBJECT_NAME_INVALID,
    };
    // SAFETY: predano dal.
    let s = unsafe { win_string(p) }.ok_or(bad)?;
    names::path_from_windows(&s).map_err(|_| bad)
}

fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &path[..i],
    }
}

/// Stat viditelneho objektu: symbolicke odkazy prvni verze nezobrazuje,
/// chovaji se jako neexistujici. Chybejici rodic = `PATH_NOT_FOUND`.
fn stat_visible(st: &mut State, path: &str) -> Result<Stat, NTSTATUS> {
    match st.fs.stat(path) {
        Ok(s) if s.kind == Kind::Symlink => Err(STATUS_OBJECT_NAME_NOT_FOUND),
        Ok(s) => Ok(s),
        Err(Error::NotFound) | Err(Error::NotADirectory) => {
            let parent = parent_of(path);
            if path != "/" && matches!(st.fs.stat(parent), Ok(p) if p.kind == Kind::Dir) {
                Err(STATUS_OBJECT_NAME_NOT_FOUND)
            } else {
                Err(STATUS_OBJECT_PATH_NOT_FOUND)
            }
        }
        Err(e) => Err(status(e)),
    }
}

fn file_info(s: &Stat) -> FileInfo {
    let dir = s.kind == Kind::Dir;
    let size = if dir { 0 } else { s.size };
    let bs = fs::BLOCK_SIZE as u64;
    FileInfo {
        FileAttributes: if dir {
            FILE_ATTRIBUTE_DIRECTORY
        } else {
            FILE_ATTRIBUTE_ARCHIVE
        },
        ReparseTag: 0,
        AllocationSize: size.div_ceil(bs).saturating_mul(bs),
        FileSize: size,
        CreationTime: filetime(s.btime),
        LastAccessTime: filetime(s.mtime),
        LastWriteTime: filetime(s.mtime),
        ChangeTime: filetime(s.ctime),
        IndexNumber: s.oid,
        HardLinks: 0,
        EaSize: 0,
    }
}

fn handle_path(st: &State, ctx: *mut c_void) -> Result<String, NTSTATUS> {
    st.handles
        .get(&(ctx as usize))
        .map(|h| h.path.clone())
        .ok_or(STATUS_INVALID_PARAMETER)
}

/// Vyplni `FileInfo` podle cesty.
fn put_info(st: &mut State, path: &str, out: *mut FileInfo) -> NTSTATUS {
    match stat_visible(st, path) {
        Ok(s) => {
            if !out.is_null() {
                // SAFETY: WinFsp predava platny vystupni ukazatel.
                unsafe { *out = file_info(&s) };
            }
            STATUS_SUCCESS
        }
        Err(e) => e,
    }
}

fn new_handle(st: &mut State, path: String, ctx: *mut *mut c_void) {
    let id = st.next_handle;
    st.next_handle += 1;
    st.handles.insert(id, Handle { path });
    // SAFETY: WinFsp predava platny vystupni ukazatel.
    unsafe { *ctx = id as *mut c_void };
}

/// Cesta s jmeny tak, jak jsou ulozena (skutecna velikost pismen), ve
/// tvaru pro Windows (`\A\b.txt`). WinFsp ji potrebuje jako
/// "normalizovane jmeno": bez ni bere jako jmeno otevreneho souboru
/// zadanou cestu prevedenou na velka pismena a prejmenovani, ktere meni
/// jen velikost pismen (`a.txt` -> `A.TXT`), by povazoval za "beze zmeny".
///
/// API `fs` zatim nema dotaz "jmeno polozky podle cesty", proto se jmeno
/// hleda ve vypisu rodice (s mezipameti `State::dir_cache`).
fn true_path(st: &mut State, path: &str) -> Option<String> {
    let mut dir = String::from("/");
    let mut win = String::new();
    for comp in comps(path) {
        let key = fold_name(&dir);
        if !st.dir_cache.contains_key(&key) {
            let list = st.fs.list(&dir).ok()?;
            st.dir_cache.insert(key.clone(), list);
        }
        let want = fold_name(comp);
        let e = st.dir_cache[&key]
            .iter()
            .find(|e| fold_name(&e.name) == want)?;
        let name = e.name.clone();
        win.push('\\');
        win.push_str(&to_windows(&name)?);
        if dir != "/" {
            dir.push('/');
        }
        dir.push_str(&name);
    }
    Some(if win.is_empty() { "\\".into() } else { win })
}

/// Zapise normalizovane jmeno do odpovedi na Create/Open.
fn put_normalized(st: &mut State, path: &str, out: *mut FileInfo) {
    let Some(name) = true_path(st, path) else {
        return;
    };
    let w: Vec<u16> = name.encode_utf16().collect();
    // SAFETY: u Create/Open ukazuje `out` na FSP_FSCTL_OPEN_FILE_INFO.
    unsafe {
        let ofi = out as *mut OpenFileInfo;
        let cap = (*ofi).NormalizedNameSize as usize / 2;
        if (*ofi).NormalizedName.is_null() || w.len() > cap {
            return;
        }
        std::ptr::copy_nonoverlapping(w.as_ptr(), (*ofi).NormalizedName, w.len());
        (*ofi).NormalizedNameSize = (w.len() * 2) as u16;
    }
}

/// Po zmene jmen v adresarich: zahodit mezipamet vypisu.
fn names_changed(st: &mut State) {
    st.dir_cache.clear();
}

/// Pred zmenou: cas a priznak zmeny.
fn mutating(st: &mut State) -> Result<(), NTSTATUS> {
    if st.read_only {
        return Err(STATUS_MEDIA_WRITE_PROTECTED);
    }
    st.fs.set_time(now_ns());
    st.dirty = true;
    Ok(())
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(s) => return s,
        }
    };
}

// --- operace ---

unsafe extern "system" fn get_volume_info(_f: *mut FileSystem, out: *mut VolumeInfo) -> NTSTATUS {
    with_state(|st| {
        let info = st.fs.info();
        let bs = info.block_size as u64;
        // SAFETY: platny vystupni ukazatel.
        let vi = unsafe { &mut *out };
        vi.TotalSize = info.block_count.saturating_mul(bs);
        vi.FreeSize = info.free_blocks.saturating_mul(bs);
        let label: Vec<u16> = info.label.encode_utf16().take(32).collect();
        vi.VolumeLabel[..label.len()].copy_from_slice(&label);
        vi.VolumeLabelLength = (label.len() * 2) as u16;
        STATUS_SUCCESS
    })
}

/// Zkopiruje popisovac zabezpeceni do bufferu WinFsp.
unsafe fn put_sd(buf: *mut c_void, size: *mut usize) -> NTSTATUS {
    if size.is_null() {
        return STATUS_SUCCESS;
    }
    let sd = SD.get().map(Vec::as_slice).unwrap_or(&[]);
    // SAFETY: platne ukazatele od WinFsp, buffer ma `*size` bajtu.
    unsafe {
        if *size < sd.len() {
            *size = sd.len();
            return STATUS_BUFFER_OVERFLOW;
        }
        if !buf.is_null() {
            std::ptr::copy_nonoverlapping(sd.as_ptr(), buf.cast(), sd.len());
        }
        *size = sd.len();
    }
    STATUS_SUCCESS
}

unsafe extern "system" fn get_security_by_name(
    _f: *mut FileSystem,
    name: PWSTR,
    attrs: *mut u32,
    sd: *mut c_void,
    sd_size: *mut usize,
) -> NTSTATUS {
    // SAFETY: retezec od WinFsp.
    let path = tri!(unsafe { fs_path(name, Want::Existing) });
    with_state(|st| {
        let s = tri!(stat_visible(st, &path));
        if !attrs.is_null() {
            // SAFETY: platny vystupni ukazatel.
            unsafe { *attrs = file_info(&s).FileAttributes };
        }
        // SAFETY: ukazatele od WinFsp.
        unsafe { put_sd(sd, sd_size) }
    })
}

unsafe extern "system" fn create(
    _f: *mut FileSystem,
    name: PWSTR,
    options: u32,
    _access: u32,
    _attrs: u32,
    _sd: *mut c_void,
    _alloc: u64,
    ctx: *mut *mut c_void,
    out: *mut FileInfo,
) -> NTSTATUS {
    // SAFETY: retezec od WinFsp.
    let path = tri!(unsafe { fs_path(name, Want::New) });
    with_state(|st| {
        tri!(mutating(st));
        debug!("create {path} (volby 0x{options:x})");
        let kind = if options & FILE_DIRECTORY_FILE != 0 {
            Kind::Dir
        } else {
            Kind::File
        };
        if let Err(e) = st.fs.create(&path, kind) {
            return match e {
                Error::NotFound | Error::NotADirectory => STATUS_OBJECT_PATH_NOT_FOUND,
                e => status(e),
            };
        }
        names_changed(st);
        let r = put_info(st, &path, out);
        if r == STATUS_SUCCESS {
            put_normalized(st, &path, out);
            new_handle(st, path, ctx);
        }
        r
    })
}

unsafe extern "system" fn open(
    _f: *mut FileSystem,
    name: PWSTR,
    options: u32,
    _access: u32,
    ctx: *mut *mut c_void,
    out: *mut FileInfo,
) -> NTSTATUS {
    // SAFETY: retezec od WinFsp.
    let path = tri!(unsafe { fs_path(name, Want::Existing) });
    with_state(|st| {
        let s = tri!(stat_visible(st, &path));
        if options & FILE_DIRECTORY_FILE != 0 && s.kind != Kind::Dir {
            return STATUS_NOT_A_DIRECTORY;
        }
        if options & FILE_NON_DIRECTORY_FILE != 0 && s.kind == Kind::Dir {
            return STATUS_FILE_IS_A_DIRECTORY;
        }
        // SAFETY: platny vystupni ukazatel.
        unsafe { *out = file_info(&s) };
        put_normalized(st, &path, out);
        new_handle(st, path, ctx);
        STATUS_SUCCESS
    })
}

unsafe extern "system" fn overwrite(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    _attrs: u32,
    _replace: BOOLEAN,
    _alloc: u64,
    out: *mut FileInfo,
) -> NTSTATUS {
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        tri!(mutating(st));
        if let Err(e) = st.fs.truncate(&path, 0) {
            return status(e);
        }
        put_info(st, &path, out)
    })
}

unsafe extern "system" fn cleanup(_f: *mut FileSystem, ctx: *mut c_void, _name: PWSTR, flags: u32) {
    if flags & FSP_CLEANUP_DELETE == 0 {
        return;
    }
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        tri!(mutating(st));
        debug!("delete {path}");
        // Smazani uz WinFsp odsouhlasil pres CanDelete; chyba tu nejde
        // vratit (Cleanup je void), jen vypsat.
        names_changed(st);
        if let Err(e) = st.fs.remove(&path) {
            eprintln!("osfs: smazani {path} selhalo: {e}");
        }
        STATUS_SUCCESS
    });
}

unsafe extern "system" fn close(_f: *mut FileSystem, ctx: *mut c_void) {
    with_state(|st| {
        st.handles.remove(&(ctx as usize));
        STATUS_SUCCESS
    });
}

unsafe extern "system" fn read(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    buf: *mut c_void,
    offset: u64,
    len: u32,
    done: *mut u32,
) -> NTSTATUS {
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        let s = tri!(stat_visible(st, &path));
        if offset >= s.size {
            return STATUS_END_OF_FILE;
        }
        let n = (s.size - offset).min(len as u64) as usize;
        match st.fs.read(&path, offset, n) {
            Ok(data) => {
                // SAFETY: buffer od WinFsp ma `len` >= data.len() bajtu.
                unsafe {
                    std::ptr::copy_nonoverlapping(data.as_ptr(), buf.cast(), data.len());
                    *done = data.len() as u32;
                }
                STATUS_SUCCESS
            }
            Err(e) => status(e),
        }
    })
}

unsafe extern "system" fn write(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    buf: *mut c_void,
    offset: u64,
    len: u32,
    to_end: BOOLEAN,
    constrained: BOOLEAN,
    done: *mut u32,
    out: *mut FileInfo,
) -> NTSTATUS {
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        let s = tri!(stat_visible(st, &path));
        tri!(mutating(st));
        let mut offset = offset;
        let mut len = len as u64;
        if constrained != 0 {
            // Strankovy vstup/vystup: soubor se nezvetsuje.
            if offset >= s.size {
                // SAFETY: platny vystupni ukazatel.
                unsafe { *done = 0 };
                return put_info(st, &path, out);
            }
            len = len.min(s.size - offset);
        } else if to_end != 0 {
            offset = s.size;
        }
        // SAFETY: buffer od WinFsp ma `len` bajtu.
        let data = unsafe { std::slice::from_raw_parts(buf as *const u8, len as usize) };
        if let Err(e) = st.fs.write(&path, offset, data) {
            return status(e);
        }
        // SAFETY: platny vystupni ukazatel.
        unsafe { *done = len as u32 };
        put_info(st, &path, out)
    })
}

unsafe extern "system" fn flush(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    out: *mut FileInfo,
) -> NTSTATUS {
    with_state(|st| {
        if st.read_only {
            return STATUS_SUCCESS;
        }
        if ctx.is_null() {
            // Flush cele jednotky: commit.
            st.fs.set_time(now_ns());
            return match st.fs.commit() {
                Ok(()) => {
                    st.dirty = false;
                    STATUS_SUCCESS
                }
                Err(e) => status(e),
            };
        }
        let path = tri!(handle_path(st, ctx));
        if let Err(e) = st.fs.fsync() {
            return status(e);
        }
        put_info(st, &path, out)
    })
}

unsafe extern "system" fn get_file_info(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    out: *mut FileInfo,
) -> NTSTATUS {
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        put_info(st, &path, out)
    })
}

/// Atributy a casy nastavit nejde (API `fs` na to nema funkci) - prijmeme
/// a vratime skutecny stav.
unsafe extern "system" fn set_basic_info(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    _attrs: u32,
    _ctime: u64,
    _atime: u64,
    _mtime: u64,
    _chtime: u64,
    out: *mut FileInfo,
) -> NTSTATUS {
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        put_info(st, &path, out)
    })
}

unsafe extern "system" fn set_file_size(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    size: u64,
    alloc_only: BOOLEAN,
    out: *mut FileInfo,
) -> NTSTATUS {
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        let s = tri!(stat_visible(st, &path));
        // Velikost alokace nas FS nedrzi: zmensi soubor jen kdyz je mensi
        // nez soucasna velikost.
        let target = if alloc_only != 0 {
            if size >= s.size {
                return put_info(st, &path, out);
            }
            size
        } else {
            size
        };
        if target != s.size {
            tri!(mutating(st));
            if let Err(e) = st.fs.truncate(&path, target) {
                return status(e);
            }
        }
        put_info(st, &path, out)
    })
}

unsafe extern "system" fn can_delete(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    _name: PWSTR,
) -> NTSTATUS {
    with_state(|st| {
        if st.read_only {
            return STATUS_MEDIA_WRITE_PROTECTED;
        }
        let path = tri!(handle_path(st, ctx));
        let s = tri!(stat_visible(st, &path));
        if s.kind == Kind::Dir {
            match st.fs.list(&path) {
                Ok(v) if !v.is_empty() => return STATUS_DIRECTORY_NOT_EMPTY,
                Ok(_) => {}
                Err(e) => return status(e),
            }
        }
        STATUS_SUCCESS
    })
}

/// Rozdeli cestu na slozky.
fn comps(p: &str) -> Vec<&str> {
    p.split('/').filter(|c| !c.is_empty()).collect()
}

/// Lezi `path` v `prefix` (nebo je to on)? Porovnani po slozkach bez ohledu
/// na velikost pismen. Vraci zbytek slozek za prefixem.
pub fn strip_prefix_folded<'a>(path: &'a str, prefix: &str) -> Option<Vec<&'a str>> {
    let p = comps(path);
    let q = comps(prefix);
    if p.len() < q.len() {
        return None;
    }
    if p.iter().zip(&q).all(|(a, b)| fold_name(a) == fold_name(b)) {
        Some(p[q.len()..].to_vec())
    } else {
        None
    }
}

unsafe extern "system" fn rename(
    _f: *mut FileSystem,
    _ctx: *mut c_void,
    name: PWSTR,
    new_name: PWSTR,
    replace: BOOLEAN,
) -> NTSTATUS {
    // SAFETY: retezce od WinFsp.
    let from = tri!(unsafe { fs_path(name, Want::Existing) });
    // SAFETY: jako vyse.
    let to = tri!(unsafe { fs_path(new_name, Want::New) });
    debug!("rename {from} -> {to} (nahradit: {replace})");
    with_state(|st| {
        tri!(stat_visible(st, &from));
        tri!(mutating(st));
        let same = strip_prefix_folded(&from, &to).is_some_and(|r| r.is_empty())
            && comps(&from).len() == comps(&to).len();
        if !same {
            match st.fs.stat(&to) {
                Ok(t) => {
                    if replace == 0 || t.kind != Kind::File {
                        return if replace == 0 {
                            STATUS_OBJECT_NAME_COLLISION
                        } else {
                            STATUS_ACCESS_DENIED
                        };
                    }
                    // Nahrazeni: oba kroky pod jednim zamkem, commit
                    // i fsync zachyti bud oba, nebo zadny.
                    names_changed(st);
                    if let Err(e) = st.fs.remove(&to) {
                        return status(e);
                    }
                }
                Err(Error::NotFound) => {}
                Err(e) => return status(e),
            }
        }
        names_changed(st);
        if let Err(e) = st.fs.rename(&from, &to) {
            return match e {
                Error::NotFound | Error::NotADirectory => STATUS_OBJECT_PATH_NOT_FOUND,
                e => status(e),
            };
        }
        // Otevrene handly pod starym jmenem.
        for h in st.handles.values_mut() {
            if let Some(rest) = strip_prefix_folded(&h.path, &from) {
                let mut p = to.clone();
                for c in rest {
                    p.push('/');
                    p.push_str(c);
                }
                h.path = p;
            }
        }
        STATUS_SUCCESS
    })
}

unsafe extern "system" fn get_security(
    _f: *mut FileSystem,
    _ctx: *mut c_void,
    sd: *mut c_void,
    sd_size: *mut usize,
) -> NTSTATUS {
    // SAFETY: ukazatele od WinFsp.
    unsafe { put_sd(sd, sd_size) }
}

/// Prida jednu polozku do bufferu vypisu. `false` = buffer je plny.
unsafe fn add_entry(
    name: &str,
    info: FileInfo,
    buf: *mut c_void,
    len: u32,
    done: *mut u32,
) -> bool {
    let wname: Vec<u16> = name.encode_utf16().collect();
    let size = size_of::<DirInfo>() + wname.len() * 2;
    let mut mem = vec![0u64; size.div_ceil(8)];
    let di = mem.as_mut_ptr() as *mut DirInfo;
    // SAFETY: `mem` ma alespon `size` bajtu a je zarovnane na 8.
    unsafe {
        (*di).Size = size as u16;
        (*di).FileInfo = info;
        let name_ptr = (di as *mut u8).add(size_of::<DirInfo>()) as *mut u16;
        std::ptr::copy_nonoverlapping(wname.as_ptr(), name_ptr, wname.len());
        let api = super::API.get().expect("API nactene pred pripojenim");
        (api.add_dir_info)(di, buf, len, done) != 0
    }
}

unsafe extern "system" fn read_directory(
    _f: *mut FileSystem,
    ctx: *mut c_void,
    _pattern: PWSTR,
    marker: PWSTR,
    buf: *mut c_void,
    len: u32,
    done: *mut u32,
) -> NTSTATUS {
    // SAFETY: retezec od WinFsp (nebo null).
    let marker = unsafe { win_string(marker) };
    with_state(|st| {
        let path = tri!(handle_path(st, ctx));
        let entries = match st.fs.list(&path) {
            Ok(v) => v,
            Err(e) => return status(e),
        };
        // Polozky v poradi podle slozeneho jmena (tak je vraci `list`).
        let mut items: Vec<(String, FileInfo)> = Vec::new();
        if path != "/" {
            let me = tri!(stat_visible(st, &path));
            let up = tri!(stat_visible(st, parent_of(&path)));
            items.push((".".into(), file_info(&me)));
            items.push(("..".into(), file_info(&up)));
        }
        let dots = items.len();
        // Pokracovani za markerem: "." a ".." jsou prvni, ostatni podle
        // slozeneho jmena (marker nemusi uz existovat).
        let after: Option<String> = match marker.as_deref() {
            None => None,
            Some(".") | Some("..") => None,
            Some(m) => Some(fold_name(
                &names::from_windows(m).unwrap_or_else(|_| m.to_string()),
            )),
        };
        let skip_dots = match marker.as_deref() {
            None => 0,
            Some(".") => 1.min(dots),
            Some(_) => dots,
        };
        for e in entries {
            if e.kind == Kind::Symlink {
                continue;
            }
            let Some(win) = to_windows(&e.name) else {
                continue;
            };
            if let Some(a) = &after
                && fold_name(&e.name) <= *a
            {
                continue;
            }
            let full = if path == "/" {
                format!("/{}", e.name)
            } else {
                format!("{path}/{}", e.name)
            };
            match st.fs.stat(&full) {
                Ok(s) => items.push((win, file_info(&s))),
                Err(e) => return status(e),
            }
        }
        for (name, info) in items.into_iter().skip(skip_dots) {
            // SAFETY: buffer od WinFsp.
            if !unsafe { add_entry(&name, info, buf, len, done) } {
                // Buffer je plny: WinFsp si rekne o dalsi cast s markerem.
                return STATUS_SUCCESS;
            }
        }
        // Konec vypisu.
        // SAFETY: null = znacka konce, viz FspFileSystemAddDirInfo.
        unsafe {
            let api = super::API.get().expect("API nactene pred pripojenim");
            (api.add_dir_info)(std::ptr::null_mut(), buf, len, done);
        }
        STATUS_SUCCESS
    })
}
