//! Vymenna media ve jmennem prostoru relaci (docs/cs/media.md): fsd je
//! jen proxy. Cizi souborovy system (FAT, NTFS) rozebira proces media
//! (`user/volfs`, protokol `volproto`); fsd nikdy.
//!
//! - Relace s pripojenim [`MEDIA_ROOT`] vidi `/media` (seznam pripojenych
//!   medii) a `/media/<jmeno>/...` (obsah media). Pripojeni jen pro cteni
//!   zakaze zapis i na media.
//! - Operace nad cestou media (STAT, LIST*, READ, WRITE, CREATE, REMOVE,
//!   RENAME, TRUNCATE, OPEN, LSTAT, REALPATH) se prelozi na pozadavek
//!   procesu media. Odpoved prijde pozdeji: relace se do te doby
//!   neobsluhuje ([`Media::deferred`], hlavni smycka `media_pump`), ostatni
//!   relace bezi dal - pomale USB nezdrzi fsd.
//! - Handly souboru na mediu drzi fsd ([`MediaHandle`]: medium a cesta,
//!   cisla od [`HANDLE_BASE`]); proces media je bez stavu. `READ_H`,
//!   `WRITE_H`, `SET_LEN`, `STAT_H`, `LIST_H`, `CLOSE` jdou pres ne.
//!   Ostatni operace s handlem media (zamky, mapovani, `READ_VMO`,
//!   identita pro dedeni) = `NOT_SUPPORTED`; klient pak cte po `MAX_IO`
//!   a `mmap` kopiruje.
//! - Vlastnik vseho na mediu = uzivatel relace; prava = vse (jen pro
//!   cteni bez zapisu). ACL NTFS se ignoruji.
//! - Proces media, ktery neodpovi do [`TIMEOUT`] nebo skonci, zmizi:
//!   cekajici relace dostanou chybu, handly `NOT_FOUND`.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use fsproto::v2::{MEDIA_ROOT, MediaEntry, Revision, op as v2op, open_flags};
use fsproto::{DirEntry, Reader, Stat, StatEntry, Writer, avail, kind, op};
use libos::{Channel, Error, Handle, Result, println, time};
use volproto::{Info, open_flags as vflags, op as vop};

use crate::serve::Service;
use crate::serve_v2::{Reply, Session};

/// Prvni cislo handlu souboru na mediu (handly FS jsou mnohem mensi).
pub const HANDLE_BASE: u64 = 1 << 62;
/// Dataset v `Revision` souboru na mediu (zadny skutecny dataset).
pub const MEDIA_DATASET: u64 = u64::MAX - 1;
/// Nejdele, kolik fsd ceka na odpoved procesu media.
pub const TIMEOUT: core::time::Duration = core::time::Duration::from_secs(30);

/// Pripojene medium.
pub struct Medium {
    pub name: String,
    pub ch: Channel,
    pub key: u64,
    pub armed: bool,
    /// Po odpovedi na `INFO` (do te doby medium neni videt).
    pub info: Option<Info>,
    read_only: bool,
    next_id: u64,
    /// Odpojuje se (`MEDIA_REMOVE`): nove operace `NOT_FOUND`.
    removing: bool,
}

/// Otevreny soubor nebo slozka na mediu (nebo slozka `/media`: medium "").
#[derive(Clone)]
pub struct MediaHandle {
    session: u64,
    medium: String,
    path: String,
    dir: bool,
    write: bool,
}

/// Co udelat s odpovedi procesu media.
enum Cont {
    /// Registrace media (`INFO`).
    Info,
    Stat { read_only: bool },
    Empty,
    Bytes,
    WriteH,
    List,
    ListPage { after: String, max: usize },
    ListStatPage { after: String, max: usize, read_only: bool },
    Open { handle: MediaHandle },
    StatH { read_only: bool, handle: u64 },
    /// `MEDIA_REMOVE` dokoncen.
    Removed,
}

struct Pending {
    /// Klic relace (port smycky), ktera ceka; 0 = nikdo (registrace).
    session: u64,
    /// Uzivatel relace (vlastnik ve `Stat`).
    owner: u64,
    cont: Cont,
    deadline: u64,
}

/// Stav medii ve fsd.
#[derive(Default)]
pub struct Media {
    pub list: Vec<Medium>,
    handles: BTreeMap<u64, MediaHandle>,
    next_handle: u64,
    /// (medium, cislo pozadavku) -> cekajici relace.
    pending: BTreeMap<(String, u64), Pending>,
    /// Posledni pozadavek relace byl odeslan medium: odpoved az pozdeji.
    pub deferred: bool,
    /// Identita relace pro vlastnika (nastavuje `handle_media`).
    owner: u64,
    /// Poradi pridanych medii (unikatni docasne jmeno pred jmenovkou:
    /// dva disky stejneho modelu).
    seq: u64,
}

/// Cesta v `/media`: (jmeno media, cesta v mediu); `None` = slozka `/media`.
type Target = (Option<String>, String);

/// Lexikalne normalizovana cesta relace (`.`, `..`, `//`).
fn lexical(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    let mut s = String::new();
    for p in parts {
        s.push('/');
        s.push_str(p);
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

/// Je cesta relace v `/media`? (Relace bez pripojeni `/media` = ne.)
/// Vraci cil a jestli je pripojeni jen pro cteni.
fn target(sess: &Session, raw: &str) -> Option<(Target, bool)> {
    let m = sess.mounts.iter().find(|m| m.name == MEDIA_ROOT)?;
    let p = lexical(raw);
    let rest = p.strip_prefix(MEDIA_ROOT)?;
    if !rest.is_empty() && !rest.starts_with('/') {
        return None;
    }
    let ro = m.read_only();
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return Some(((None, String::from("/")), ro));
    }
    let (name, inner) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    Some(((Some(name.to_string()), inner.to_string()), ro))
}

fn wall(secs: i64) -> libos::abi::time::WallTime {
    libos::abi::time::WallTime { secs, nanos: 0 }
}

/// Stav souboru media pro klienta (vlastnik relace, prava podle zapisu).
fn to_stat(s: &volproto::Stat, owner: u64, read_only: bool) -> Stat {
    use fsproto::perm::rights;
    let ro = read_only || s.read_only;
    Stat {
        oid: 0,
        kind: if s.kind == volproto::kind::DIR { kind::DIR } else { kind::FILE },
        size: s.size,
        mtime: wall(s.mtime),
        ctime: wall(s.mtime),
        btime: wall(s.btime),
        owner,
        rights: if ro { rights::VIEW } else { rights::EDIT },
        tree_bytes: 0,
        tree_files: 0,
        tree_dirs: 0,
        mode: 0,
        known: avail::SIZE | avail::MTIME | avail::BTIME | avail::OWNER | avail::RIGHTS,
        hidden: 0,
    }
}

fn dir_stat(owner: u64) -> Stat {
    to_stat(&volproto::Stat { kind: volproto::kind::DIR, ..Default::default() }, owner, true)
}

/// Seznam media (`LIST` od procesu media), serazeny podle slozeneho jmena.
fn parse_list(body: &[u8]) -> Result<Vec<(String, volproto::Stat)>> {
    let mut r = Reader::new(body);
    let n = r.u32()?;
    let mut v = (0..n).map(|_| Ok((r.str()?, volproto::Stat::get(&mut r)?))).collect::<Result<Vec<_>>>()?;
    v.sort_by_key(|(n, _)| fs::fold_name(n));
    Ok(v)
}

fn page_start<T>(list: &[(String, T)], after: &str) -> usize {
    if after.is_empty() {
        0
    } else {
        let a = fs::fold_name(after);
        list.partition_point(|(n, _)| fs::fold_name(n) <= a)
    }
}

/// Revize souboru na mediu (jen pro tvar odpovedi `OPEN`/`STAT_H`).
fn revision(h: u64) -> Revision {
    Revision { dataset: MEDIA_DATASET, oid: h, generation: 0, version: 0 }
}

impl Media {
    fn medium(&mut self, name: &str) -> Result<&mut Medium> {
        self.list.iter_mut().find(|m| m.name == name && m.info.is_some() && !m.removing).ok_or(Error::NotFound)
    }

    /// Odesle pozadavek medium a zaznamena, kdo ceka.
    fn send(&mut self, name: &str, session: u64, cont: Cont, build: impl FnOnce(&mut Writer), opcode: u32) -> Result<()> {
        let m = self.list.iter_mut().find(|m| m.name == name).ok_or(Error::NotFound)?;
        m.next_id += 1;
        let id = m.next_id;
        let mut w = volproto::request(id, opcode);
        build(&mut w);
        m.ch.write(&w.finish(), Vec::new()).map_err(|_| Error::NotFound)?;
        let owner = self.owner;
        self.pending.insert((name.to_string(), id), Pending { session, owner, cont, deadline: time::deadline_after(TIMEOUT) });
        Ok(())
    }

    /// Pozadavek za relaci `session`: odpoved odlozit.
    fn forward(&mut self, name: &str, session: u64, cont: Cont, opcode: u32, build: impl FnOnce(&mut Writer)) -> Result<Reply> {
        self.medium(name)?;
        self.send(name, session, cont, build, opcode)?;
        self.deferred = true;
        Ok(Reply::body(Writer::new()))
    }

    /// Jmeno pro nove medium: jmenovka (bez znaku, ktere ve jmene byt
    /// nemohou), jinak "USB disk"; obsazene = " (2)", " (3)", ...
    fn unique_name(&self, label: &str) -> String {
        let base: String = label.chars().filter(|c| *c != '/' && !c.is_control()).collect::<String>().trim().to_string();
        let base = if base.is_empty() { String::from("USB disk") } else { base };
        let taken = |n: &str| self.list.iter().any(|m| m.info.is_some() && m.name == n);
        if !taken(&base) {
            return base;
        }
        (2..).map(|i| format!("{base} ({i})")).find(|n| !taken(n)).unwrap_or(base)
    }

    /// Nejblizsi termin cekajiciho pozadavku (pro cekani smycky).
    pub fn next_deadline(&self) -> Option<u64> {
        self.pending.values().map(|p| p.deadline).min()
    }

    /// Relace skoncila: jeji handly pryc.
    pub fn close_session(&mut self, key: u64) {
        self.handles.retain(|_, h| h.session != key);
        for p in self.pending.values_mut() {
            if p.session == key {
                p.session = 0;
            }
        }
    }
}

/// Odpoved pro relaci (`media_pump` ji zapise na kanal relace).
pub struct Delivery {
    pub session: u64,
    pub reply: Vec<u8>,
}

impl Service {
    /// Operace nad `/media` nebo handlem media; `None` = neni jeji.
    pub fn handle_media(&mut self, sess: &mut Session, opcode: u32, msg: &[u8], handles: &mut Vec<Handle>) -> Option<Result<Reply>> {
        self.media.deferred = false;
        self.media.owner = sess.identity.as_ref().map_or(0, |p| p.user);
        let mut r = Reader::new(msg);
        r.u32().ok()?;
        match opcode {
            v2op::MEDIA_ADD => Some(self.media_add(sess, &mut r, handles)),
            v2op::MEDIA_REMOVE => Some((|| {
                // Odpojit smi system i uzivatel, ktery media vidi (Soubory).
                let user_rw = sess.mounts.iter().any(|m| m.name == MEDIA_ROOT && !m.read_only());
                if !admin(sess) && !user_rw {
                    return Err(Error::AccessDenied);
                }
                let name = r.str()?;
                let m = self.media.medium(&name)?;
                m.removing = true;
                self.media.send(&name, sess.key, Cont::Removed, |_| {}, vop::UNMOUNT)?;
                self.media.deferred = true;
                Ok(Reply::body(Writer::new()))
            })()),
            v2op::MEDIA_LIST => Some((|| {
                if !admin(sess) && !sess.mounts.iter().any(|m| m.name == MEDIA_ROOT) {
                    return Err(Error::AccessDenied);
                }
                let mut w = Writer::new();
                let ready: Vec<&Medium> = self.media.list.iter().filter(|m| m.info.is_some() && !m.removing).collect();
                w.u32(ready.len() as u32);
                for m in ready {
                    let i = m.info.as_ref().unwrap_or_else(|| unreachable!());
                    MediaEntry {
                        name: m.name.clone(),
                        label: i.label.clone(),
                        fs: i.fs.clone(),
                        read_only: m.read_only || i.read_only,
                        total_bytes: i.total_bytes,
                        free_bytes: i.free_bytes,
                    }
                    .put(&mut w);
                }
                Ok(Reply::body(w))
            })()),
            // Okamzita kopie s mediem (medium ji nema) a prejmenovani do
            // media z vlastniho FS: `NOT_SUPPORTED` (Soubory kopiruji ctenim
            // a zapisem). Bez toho by cil na mediu sel do vlastniho FS.
            fsproto::data::op::CLONE | op::RENAME | v2op::RENAME_REPLACE
                if {
                    let mut r2 = Reader::new(msg);
                    let _ = r2.u32();
                    let a = r2.str().ok().and_then(|p| target(sess, &p));
                    let b = r2.str().ok().and_then(|p| target(sess, &p));
                    if opcode == fsproto::data::op::CLONE { a.is_some() || b.is_some() } else { b.is_some() && a.is_none() }
                } =>
            {
                Some(Err(Error::NotSupported))
            }
            // Kos na mediu neni (FAT, exFAT, NTFS): klient smaze trvale.
            fsproto::hist::op::TRASH => {
                let raw = r.str().ok()?;
                target(sess, &raw)?;
                Some(Err(Error::NotSupported))
            }
            // Kazde medium je pro klienty vlastni "dataset" (presun mezi nim
            // a domovem = kopie a smazani, Soubory, files-copy).
            fsproto::hist::op::DATASET_OF => {
                let raw = r.str().ok()?;
                let ((name, _), _) = target(sess, &raw)?;
                let mut w = Writer::new();
                w.str(&match name {
                    Some(n) => format!("{MEDIA_ROOT}/{n}"),
                    None => String::from(MEDIA_ROOT),
                });
                Some(Ok(Reply::body(w)))
            }
            // Operace nad cestou.
            op::STAT | op::LIST | op::LIST_PAGE | op::LIST_STAT_PAGE | op::READ | op::WRITE | op::CREATE | op::REMOVE | op::RENAME | op::TRUNCATE
            | v2op::OPEN | v2op::LSTAT | v2op::REALPATH | v2op::RENAME_REPLACE => {
                let raw = r.str().ok()?;
                let (t, ro) = target(sess, &raw)?;
                Some(self.media_path(sess, opcode, &raw, t, ro, &mut r))
            }
            // Operace s handlem media.
            v2op::READ_H | v2op::WRITE_H | v2op::SET_LEN | v2op::STAT_H | v2op::LIST_H | v2op::CLOSE | v2op::READ_VMO | v2op::MAP_VMO | v2op::LOCK
            | v2op::UNLOCK | v2op::POSIX_LOCK | v2op::OPEN_STREAM | v2op::DELETE_ON_CLOSE => {
                let h = r.u64().ok()?;
                if h < HANDLE_BASE {
                    return None;
                }
                Some(self.media_handle_op(sess, opcode, h, &mut r))
            }
            _ => None,
        }
    }

    fn media_add(&mut self, sess: &Session, r: &mut Reader, handles: &mut Vec<Handle>) -> Result<Reply> {
        if !admin(sess) {
            return Err(Error::AccessDenied);
        }
        let name = r.str()?;
        let read_only = r.u8()? != 0;
        let ch = Channel::from_handle(handles.pop().ok_or(Error::InvalidArgs)?);
        // Docasne jmeno (unikatni klic); skutecne az podle jmenovky.
        self.media.seq += 1;
        let tmp = format!("\u{1}{}\u{1}{name}", self.media.seq);
        self.media.list.push(Medium { name: tmp.clone(), ch, key: 0, armed: false, info: None, read_only, next_id: 0, removing: false });
        self.media.send(&tmp, 0, Cont::Info, |_| {}, vop::INFO)?;
        Ok(Reply::body(Writer::new()))
    }

    fn media_path(&mut self, sess: &Session, opcode: u32, raw: &str, t: Target, ro: bool, r: &mut Reader) -> Result<Reply> {
        let owner = self.media.owner;
        let key = sess.key;
        let (name, path) = t;
        let path = volproto::normalize(&path)?;
        let writes = matches!(opcode, op::WRITE | op::CREATE | op::REMOVE | op::RENAME | op::TRUNCATE | v2op::RENAME_REPLACE);
        let Some(name) = name else {
            // Samotna slozka /media.
            if writes {
                return Err(Error::AccessDenied);
            }
            return self.media_root(opcode, raw, r, sess);
        };
        let medium_ro = {
            let m = self.media.medium(&name)?;
            m.read_only || m.info.as_ref().is_some_and(|i| i.read_only)
        };
        let read_only = ro || medium_ro;
        if writes && read_only {
            return Err(Error::ReadOnly);
        }
        let _ = owner;
        match opcode {
            op::STAT | v2op::LSTAT => self.media.forward(&name, key, Cont::Stat { read_only }, vop::STAT, |w| {
                w.str(&path);
            }),
            v2op::REALPATH => {
                let mut w = Writer::new();
                w.str(&lexical(raw));
                Ok(Reply::body(w))
            }
            op::LIST => self.media.forward(&name, key, Cont::List, vop::LIST, |w| {
                w.str(&path);
            }),
            op::LIST_PAGE | op::LIST_STAT_PAGE => {
                let after = r.str()?;
                let max = r.u32()? as usize;
                let cont = if opcode == op::LIST_PAGE { Cont::ListPage { after, max } } else { Cont::ListStatPage { after, max, read_only } };
                self.media.forward(&name, key, cont, vop::LIST, |w| {
                    w.str(&path);
                })
            }
            op::READ => {
                let off = r.u64()?;
                let n = (r.u32()? as usize).min(volproto::MAX_IO);
                self.media.forward(&name, key, Cont::Bytes, vop::READ, |w| {
                    w.str(&path).u64(off).u32(n as u32);
                })
            }
            op::WRITE => {
                let off = r.u64()?;
                let data = r.rest();
                if data.len() > volproto::MAX_IO {
                    return Err(Error::InvalidArgs);
                }
                self.media.forward(&name, key, Cont::Empty, vop::WRITE, |w| {
                    w.str(&path).u64(off).bytes(data);
                })
            }
            op::CREATE => {
                let k = r.u8()?;
                if k != kind::FILE && k != kind::DIR {
                    return Err(Error::InvalidArgs);
                }
                self.media.forward(&name, key, Cont::Empty, vop::CREATE, |w| {
                    w.str(&path).u8(k).u8(1);
                })
            }
            op::REMOVE => self.media.forward(&name, key, Cont::Empty, vop::REMOVE, |w| {
                w.str(&path);
            }),
            op::RENAME | v2op::RENAME_REPLACE => {
                let to_raw = r.str()?;
                let ((to_name, to_path), _) = target(sess, &to_raw).ok_or(Error::NotSupported)?;
                // Jen v ramci jednoho media (jinak kopie: files-copy).
                if to_name.as_deref() != Some(name.as_str()) {
                    return Err(Error::NotSupported);
                }
                let to_path = volproto::normalize(&to_path)?;
                let replace = opcode == v2op::RENAME_REPLACE;
                self.media.forward(&name, key, Cont::Empty, vop::RENAME, |w| {
                    w.str(&path).str(&to_path).u8(replace as u8);
                })
            }
            op::TRUNCATE => {
                let len = r.u64()?;
                self.media.forward(&name, key, Cont::Empty, vop::SET_LEN, |w| {
                    w.str(&path).u64(len);
                })
            }
            v2op::OPEN => {
                let (access, _share, flags) = (r.u8()?, r.u8()?, r.u8()?);
                let write = access & (fsproto::v2::access::WRITE | fsproto::v2::access::DELETE) != 0
                    || flags & (open_flags::CREATE | open_flags::TRUNCATE) != 0;
                if write && read_only {
                    return Err(Error::ReadOnly);
                }
                let mut vf = 0;
                if flags & open_flags::CREATE != 0 {
                    vf |= vflags::CREATE;
                }
                if flags & open_flags::EXCL != 0 {
                    vf |= vflags::EXCL;
                }
                if flags & open_flags::TRUNCATE != 0 {
                    vf |= vflags::TRUNCATE;
                }
                if flags & open_flags::DIR != 0 {
                    vf |= vflags::DIR;
                }
                let handle = MediaHandle { session: key, medium: name.clone(), path: path.clone(), dir: false, write: write && !read_only };
                self.media.forward(&name, key, Cont::Open { handle }, vop::OPEN, |w| {
                    w.str(&path).u8(vf);
                })
            }
            _ => Err(Error::NotSupported),
        }
    }

    /// Slozka `/media`: seznam pripojenych medii.
    fn media_root(&mut self, opcode: u32, raw: &str, r: &mut Reader, sess: &Session) -> Result<Reply> {
        let owner = self.media.owner;
        let names: Vec<String> = {
            let mut v: Vec<String> = self.media.list.iter().filter(|m| m.info.is_some() && !m.removing).map(|m| m.name.clone()).collect();
            v.sort_by_key(|n| fs::fold_name(n));
            v
        };
        let mut w = Writer::new();
        match opcode {
            op::STAT | v2op::LSTAT => dir_stat(owner).put(&mut w),
            v2op::REALPATH => {
                w.str(&lexical(raw));
            }
            op::LIST => {
                let list: Vec<DirEntry> = names.iter().map(|n| DirEntry { name: n.clone(), kind: kind::DIR, oid: 0 }).collect();
                fsproto::encode_list(&mut w, &list);
            }
            op::LIST_PAGE | op::LIST_STAT_PAGE => {
                let after = r.str()?;
                let max = r.u32()? as usize;
                let items: Vec<(String, ())> = names.into_iter().map(|n| (n, ())).collect();
                let rest = &items[page_start(&items, &after)..];
                if opcode == op::LIST_PAGE {
                    let page: Vec<DirEntry> = rest.iter().map(|(n, _)| DirEntry { name: n.clone(), kind: kind::DIR, oid: 0 }).collect();
                    fsproto::encode_list_page(&mut w, &page, max, fsproto::MAX_REPLY);
                } else {
                    let n = fsproto::list_stat_fit(rest.iter().map(|(n, _)| n.as_str()), max, fsproto::MAX_REPLY);
                    let st = dir_stat(owner);
                    let page: Vec<StatEntry> = rest[..n].iter().map(|(name, _)| stat_entry(name, &st)).collect();
                    fsproto::encode_list_stat_page(&mut w, &page, n < rest.len());
                }
            }
            v2op::OPEN => {
                let (access, _share, flags) = (r.u8()?, r.u8()?, r.u8()?);
                if access & fsproto::v2::access::WRITE != 0 || flags & (open_flags::CREATE | open_flags::TRUNCATE) != 0 {
                    return Err(Error::AccessDenied);
                }
                let h = self.media.new_handle(MediaHandle { session: sess.key, medium: String::new(), path: String::from("/"), dir: true, write: false });
                w.u64(h);
                dir_stat(owner).put(&mut w);
                revision(h).put(&mut w);
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(Reply::body(w))
    }

    fn media_handle_op(&mut self, sess: &Session, opcode: u32, h: u64, r: &mut Reader) -> Result<Reply> {
        let mh = self.media.handles.get(&h).filter(|m| m.session == sess.key).cloned().ok_or(Error::BadHandle)?;
        let key = sess.key;
        let ro = !mh.write;
        match opcode {
            v2op::CLOSE => {
                self.media.handles.remove(&h);
                Ok(Reply::body(Writer::new()))
            }
            v2op::STAT_H if mh.medium.is_empty() => {
                let mut w = Writer::new();
                dir_stat(self.media.owner).put(&mut w);
                revision(h).put(&mut w);
                Ok(Reply::body(w))
            }
            v2op::LIST_H if mh.medium.is_empty() => {
                let after = r.str()?;
                let max = r.u32()? as usize;
                let mut names: Vec<(String, ())> = self.media.list.iter().filter(|m| m.info.is_some() && !m.removing).map(|m| (m.name.clone(), ())).collect();
                names.sort_by_key(|(n, _)| fs::fold_name(n));
                let page: Vec<DirEntry> = names[page_start(&names, &after)..].iter().map(|(n, _)| DirEntry { name: n.clone(), kind: kind::DIR, oid: 0 }).collect();
                let mut w = Writer::new();
                fsproto::encode_list_page(&mut w, &page, max, fsproto::MAX_REPLY);
                Ok(Reply::body(w))
            }
            _ if mh.medium.is_empty() => Err(Error::NotSupported),
            v2op::READ_H => {
                let off = r.u64()?;
                let n = (r.u32()? as usize).min(fsproto::MAX_IO).min(volproto::MAX_IO);
                self.media.forward(&mh.medium, key, Cont::Bytes, vop::READ, |w| {
                    w.str(&mh.path).u64(off).u32(n as u32);
                })
            }
            v2op::WRITE_H => {
                if ro {
                    return Err(Error::AccessDenied);
                }
                let off = r.u64()?;
                let data = r.rest();
                if data.len() > volproto::MAX_IO {
                    return Err(Error::InvalidArgs);
                }
                self.media.forward(&mh.medium, key, Cont::WriteH, vop::WRITE, |w| {
                    w.str(&mh.path).u64(off).bytes(data);
                })
            }
            v2op::SET_LEN => {
                if ro {
                    return Err(Error::AccessDenied);
                }
                let len = r.u64()?;
                self.media.forward(&mh.medium, key, Cont::Empty, vop::SET_LEN, |w| {
                    w.str(&mh.path).u64(len);
                })
            }
            v2op::STAT_H => self.media.forward(&mh.medium, key, Cont::StatH { read_only: ro, handle: h }, vop::STAT, |w| {
                w.str(&mh.path);
            }),
            v2op::LIST_H => {
                let after = r.str()?;
                let max = r.u32()? as usize;
                self.media.forward(&mh.medium, key, Cont::ListPage { after, max }, vop::LIST, |w| {
                    w.str(&mh.path);
                })
            }
            // Zamky, mapovani, velke cteni, proudy: na mediu ne (klient ma
            // nahradni cestu nebo hlasi chybu).
            _ => Err(Error::NotSupported),
        }
    }

    /// Odpovedi procesu media `i` (a uplynule terminy): vrati odpovedi pro
    /// relace. Medium s chybou nebo zavrenym kanalem zmizi.
    pub fn media_replies(&mut self, i: usize) -> Vec<Delivery> {
        let mut out = Vec::new();
        let mut broken = false;
        loop {
            let m = match self.media.list[i].ch.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    broken = true;
                    break;
                }
            };
            let Ok((id, res)) = volproto::parse_reply(&m.data) else {
                broken = true;
                break;
            };
            let name = self.media.list[i].name.clone();
            let Some(p) = self.media.pending.remove(&(name.clone(), id)) else { continue };
            let res = res.map(|b| b.to_vec());
            let (d, drop_after) = self.finish(i, p, res);
            out.extend(d);
            if drop_after {
                broken = true;
                break;
            }
        }
        if broken {
            out.extend(self.drop_medium(i));
        }
        out
    }

    /// Terminy: cekajici pozadavky po [`TIMEOUT`] = chyba a medium pryc.
    pub fn media_timeouts(&mut self) -> Vec<Delivery> {
        let now = time::now();
        let late: Vec<String> = self.media.pending.iter().filter(|(_, p)| p.deadline <= now).map(|((n, _), _)| n.clone()).collect();
        let mut out = Vec::new();
        for name in late {
            if let Some(i) = self.media.list.iter().position(|m| m.name == name) {
                println!("fsd: medium {name} neodpovida, odpojuji");
                out.extend(self.drop_medium(i));
            }
        }
        out
    }

    /// Medium zmizelo: cekajici relace dostanou chybu, handly zaniknou.
    fn drop_medium(&mut self, i: usize) -> Vec<Delivery> {
        let m = self.media.list.remove(i);
        let mut out = Vec::new();
        let keys: Vec<(String, u64)> = self.media.pending.keys().filter(|(n, _)| *n == m.name).cloned().collect();
        for k in keys {
            if let Some(p) = self.media.pending.remove(&k) {
                // Odpojeni dokoncene (proces media skoncil po UNMOUNT).
                let res = if matches!(p.cont, Cont::Removed) { Ok(Writer::new()) } else { Err(Error::NotFound) };
                if p.session != 0 {
                    out.push(Delivery { session: p.session, reply: fsproto::reply(res) });
                }
            }
        }
        self.media.handles.retain(|_, h| h.medium != m.name);
        out
    }

    /// Prelozi odpoved procesu media na odpoved relaci.
    fn finish(&mut self, i: usize, p: Pending, res: Result<Vec<u8>>) -> (Option<Delivery>, bool) {
        let owner = p.owner;
        let drop_after = matches!(p.cont, Cont::Removed) || (matches!(p.cont, Cont::Info) && res.is_err());
        let reply: Result<Writer> = (|| {
            let body = res?;
            let mut w = Writer::new();
            match p.cont {
                Cont::Info => {
                    let info = Info::get(&mut Reader::new(&body))?;
                    let name = self.media.unique_name(&info.label);
                    let m = &mut self.media.list[i];
                    println!("fsd: medium {name} pripojeno ({}, {} MiB{})", info.fs, info.total_bytes >> 20, if m.read_only || info.read_only { ", jen pro cteni" } else { "" });
                    m.name = name;
                    m.info = Some(info);
                }
                Cont::Stat { read_only } => to_stat(&volproto::Stat::get(&mut Reader::new(&body))?, owner, read_only).put(&mut w),
                Cont::StatH { read_only, handle } => {
                    to_stat(&volproto::Stat::get(&mut Reader::new(&body))?, owner, read_only).put(&mut w);
                    revision(handle).put(&mut w);
                }
                Cont::Empty | Cont::Removed => {}
                Cont::Bytes => {
                    w.bytes(&body);
                }
                Cont::WriteH => {
                    w.u32(Reader::new(&body).u32()?);
                }
                Cont::List => {
                    let list: Vec<DirEntry> = parse_list(&body)?.into_iter().map(|(name, s)| DirEntry { name, kind: kind_of(&s), oid: 0 }).collect();
                    fsproto::encode_list(&mut w, &list);
                    if w.len() > fsproto::MAX_REPLY {
                        return Err(Error::BufferTooSmall);
                    }
                }
                Cont::ListPage { after, max } => {
                    let list = parse_list(&body)?;
                    let page: Vec<DirEntry> = list[page_start(&list, &after)..].iter().map(|(n, s)| DirEntry { name: n.clone(), kind: kind_of(s), oid: 0 }).collect();
                    fsproto::encode_list_page(&mut w, &page, max, fsproto::MAX_REPLY);
                }
                Cont::ListStatPage { after, max, read_only } => {
                    let list = parse_list(&body)?;
                    let rest = &list[page_start(&list, &after)..];
                    let n = fsproto::list_stat_fit(rest.iter().map(|(n, _)| n.as_str()), max, fsproto::MAX_REPLY);
                    let page: Vec<StatEntry> = rest[..n].iter().map(|(name, s)| stat_entry(name, &to_stat(s, owner, read_only))).collect();
                    fsproto::encode_list_stat_page(&mut w, &page, n < rest.len());
                }
                Cont::Open { mut handle } => {
                    let s = volproto::Stat::get(&mut Reader::new(&body))?;
                    handle.dir = s.kind == volproto::kind::DIR;
                    let ro = !handle.write;
                    let h = self.media.new_handle(handle);
                    w.u64(h);
                    to_stat(&s, owner, ro).put(&mut w);
                    revision(h).put(&mut w);
                }
            }
            Ok(w)
        })();
        if p.session == 0 {
            if let Err(e) = &reply {
                println!("fsd: medium nejde pripojit ({e})");
            }
            return (None, drop_after);
        }
        (Some(Delivery { session: p.session, reply: fsproto::reply(reply) }), drop_after)
    }
}

/// Ridici relace nebo relace systemu (init) s celym FS.
fn admin(sess: &Session) -> bool {
    sess.control || (sess.identity.is_none() && sess.full_root().is_ok())
}

fn kind_of(s: &volproto::Stat) -> u8 {
    if s.kind == volproto::kind::DIR { kind::DIR } else { kind::FILE }
}

fn stat_entry(name: &str, st: &Stat) -> StatEntry {
    const ENTRY_AVAIL: u16 = avail::SIZE | avail::MTIME | avail::RIGHTS | avail::TREE;
    StatEntry {
        name: name.to_string(),
        kind: st.kind,
        oid: 0,
        size: st.size,
        mtime: st.mtime,
        rights: st.rights,
        tree_bytes: 0,
        tree_files: 0,
        tree_dirs: 0,
        known: st.known & ENTRY_AVAIL,
        hidden: 0,
    }
}

impl Media {
    fn new_handle(&mut self, h: MediaHandle) -> u64 {
        self.next_handle += 1;
        let id = HANDLE_BASE + self.next_handle;
        self.handles.insert(id, h);
        id
    }
}
