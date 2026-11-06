//! Zpravy: pozadavky, odpovedi a jejich kodovani.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec::{Reader, Writer};
use crate::{error, MAGIC, MAX_CURSOR, MAX_MESSAGE, MAX_PATH, VERSION};

/// Kody operaci.
pub mod op {
    pub const HELLO: u16 = 1;
    pub const VOLUME_INFO: u16 = 2;
    pub const STAT: u16 = 3;
    pub const LIST: u16 = 4;
    pub const READ: u16 = 5;
    pub const RENAME: u16 = 6;
    pub const SNAPSHOTS: u16 = 7;
    pub const SNAPSHOT_CREATE: u16 = 8;
    pub const COMMIT: u16 = 9;
}

/// Cesta v pozadavku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathArg {
    /// Cesta v nasem FS: `/a/b`, jmena tak, jak jsou ulozena (i skryta ve
    /// Windows, napr. `/CON.txt` nebo `/a:b`).
    Fs(String),
    /// Cesta tak, jak ji vidi Windows, relativne ke koreni jednotky:
    /// `\a\b` (zakazane znaky jako U+F0xx). Server ji prelozi stejne jako
    /// pri otevreni souboru.
    Windows(String),
}

impl PathArg {
    /// Z plne cesty ve Windows (`Q:\a\b`, `Q:`, `\\?\Q:\a`) udela
    /// `PathArg::Windows("\a\b")`.
    pub fn from_windows_full(path: &str) -> Self {
        let p = path.strip_prefix("\\\\?\\").unwrap_or(path);
        let b = p.as_bytes();
        let rest = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
            &p[2..]
        } else {
            p
        };
        let rest = if rest.is_empty() { "\\" } else { rest };
        PathArg::Windows(rest.into())
    }

    fn put(&self, w: &mut Writer) {
        match self {
            PathArg::Fs(p) => w.u8(0).str(p),
            PathArg::Windows(p) => w.u8(1).str(p),
        };
    }

    fn get(r: &mut Reader) -> Option<Self> {
        let kind = r.u8()?;
        let p = r.str()?;
        if p.len() > MAX_PATH {
            return None;
        }
        match kind {
            0 => Some(PathArg::Fs(p)),
            1 => Some(PathArg::Windows(p)),
            _ => None,
        }
    }
}

/// Pozadavek. `at` = jmeno snapshotu, prazdne = zivy stav.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Verze a schopnosti serveru.
    Hello,
    /// Informace o svazku.
    VolumeInfo,
    /// Metadata objektu.
    Stat { at: String, path: PathArg },
    /// Stranka vypisu adresare (vcetne polozek skrytych ve Windows).
    /// `cursor` = prazdny pro prvni stranku, jinak `ListPage::next`.
    List {
        at: String,
        path: PathArg,
        cursor: String,
    },
    /// Cast obsahu souboru (nejvys `MAX_READ` bajtu).
    Read {
        at: String,
        path: PathArg,
        offset: u64,
        len: u32,
    },
    /// Prejmenovani nebo presun (cil nesmi existovat).
    Rename { from: PathArg, to: PathArg },
    /// Stranka seznamu snapshotu s id vetsim nez `after` (0 = od zacatku).
    Snapshots { after: u64 },
    /// Vytvori snapshot aktualniho stavu (uklada = commit).
    SnapshotCreate { name: String },
    /// Ulozi vsechny zmeny hned (commit).
    Commit,
}

impl Request {
    pub fn op(&self) -> u16 {
        match self {
            Request::Hello => op::HELLO,
            Request::VolumeInfo => op::VOLUME_INFO,
            Request::Stat { .. } => op::STAT,
            Request::List { .. } => op::LIST,
            Request::Read { .. } => op::READ,
            Request::Rename { .. } => op::RENAME,
            Request::Snapshots { .. } => op::SNAPSHOTS,
            Request::SnapshotCreate { .. } => op::SNAPSHOT_CREATE,
            Request::Commit => op::COMMIT,
        }
    }

    /// Meni operace svazek? Takove se posilaji kodem `IOCTL_WRITE`.
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Request::Rename { .. } | Request::SnapshotCreate { .. } | Request::Commit
        )
    }

    /// IOCTL kod, kterym se pozadavek posila.
    pub fn ioctl(&self) -> u32 {
        if self.is_write() {
            crate::IOCTL_WRITE
        } else {
            crate::IOCTL_READ
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(MAGIC).u16(VERSION).u16(self.op());
        match self {
            Request::Hello | Request::VolumeInfo | Request::Commit => {}
            Request::Snapshots { after } => {
                w.u64(*after);
            }
            Request::Stat { at, path } => {
                w.str(at);
                path.put(&mut w);
            }
            Request::List { at, path, cursor } => {
                w.str(at);
                path.put(&mut w);
                w.str(cursor);
            }
            Request::Read {
                at,
                path,
                offset,
                len,
            } => {
                w.str(at);
                path.put(&mut w);
                w.u64(*offset).u32(*len);
            }
            Request::Rename { from, to } => {
                from.put(&mut w);
                to.put(&mut w);
            }
            Request::SnapshotCreate { name } => {
                w.str(name);
            }
        }
        w.buf
    }

    /// Dekoduje pozadavek. `Err(kod)` = neplatna zprava (spatna hlavicka,
    /// neznama operace, zbytek navic) - server ji odmitne timto kodem.
    pub fn decode(b: &[u8]) -> Result<Self, i32> {
        if b.len() > MAX_MESSAGE {
            return Err(error::INVALID_ARGS);
        }
        let mut r = Reader::new(b);
        let bad = error::INVALID_ARGS;
        if r.u32() != Some(MAGIC) {
            return Err(bad);
        }
        if r.u16() != Some(VERSION) {
            return Err(error::NOT_SUPPORTED);
        }
        let op = r.u16().ok_or(bad)?;
        let req = match op {
            op::HELLO => Request::Hello,
            op::VOLUME_INFO => Request::VolumeInfo,
            op::SNAPSHOTS => Request::Snapshots {
                after: r.u64().ok_or(bad)?,
            },
            op::COMMIT => Request::Commit,
            op::STAT => Request::Stat {
                at: r.str().ok_or(bad)?,
                path: PathArg::get(&mut r).ok_or(bad)?,
            },
            op::LIST => Request::List {
                at: r.str().ok_or(bad)?,
                path: PathArg::get(&mut r).ok_or(bad)?,
                cursor: r.str().filter(|c| c.len() <= MAX_CURSOR).ok_or(bad)?,
            },
            op::READ => Request::Read {
                at: r.str().ok_or(bad)?,
                path: PathArg::get(&mut r).ok_or(bad)?,
                offset: r.u64().ok_or(bad)?,
                len: r.u32().ok_or(bad)?,
            },
            op::RENAME => Request::Rename {
                from: PathArg::get(&mut r).ok_or(bad)?,
                to: PathArg::get(&mut r).ok_or(bad)?,
            },
            op::SNAPSHOT_CREATE => Request::SnapshotCreate {
                name: r.str().ok_or(bad)?,
            },
            _ => return Err(error::NOT_SUPPORTED),
        };
        if !r.done() {
            return Err(bad);
        }
        Ok(req)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelloInfo {
    /// Nejvyssi verze protokolu, kterou server umi.
    pub version: u16,
    /// Bity `feature::*`.
    pub features: u64,
    /// Kdo odpovida, napr. `osfs 0.1.0 (WinFsp)`.
    pub server: String,
    /// Nejvetsi zprava, kterou server prijme a posle (bajty).
    pub max_message: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub label: String,
    pub block_size: u32,
    pub block_count: u64,
    /// Volne bloky (vcetne cekajicich na odlozene uvolneni).
    pub free_blocks: u64,
    pub deferred_blocks: u64,
    /// Generace posledniho ulozeneho stavu.
    pub generation: u64,
    /// 1 = XXH3-128, 2 = BLAKE3.
    pub algo: u8,
    pub read_only: bool,
    /// O kolik generaci se pripojeni vratilo kvuli poskozenemu stavu.
    pub fallback: u64,
    /// Kolik operaci se pri pripojeni prehralo z intent logu.
    pub replayed: u64,
}

/// Druh objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Symlink,
}

impl Kind {
    fn raw(self) -> u8 {
        match self {
            Kind::File => 1,
            Kind::Dir => 2,
            Kind::Symlink => 3,
        }
    }

    fn from_raw(v: u8) -> Option<Self> {
        match v {
            1 => Some(Kind::File),
            2 => Some(Kind::Dir),
            3 => Some(Kind::Symlink),
            _ => None,
        }
    }
}

/// Proc Windows polozku nezobrazi (pravidla viz `docs/cs/osfs.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hidden {
    Visible,
    /// Obsahuje znak z U+F000-F0FF.
    PrivateUse,
    /// Rezervovane jmeno (CON, NUL, COM1, ...).
    Reserved,
    /// Konci teckou nebo mezerou.
    TrailingDotOrSpace,
    /// Delsi nez 255 znaku UTF-16.
    TooLong,
    /// Symbolicky odkaz (prvni verze ovladace je nezobrazuje).
    Symlink,
    /// Neznamy duvod (novejsi server).
    Other,
}

impl Hidden {
    fn raw(self) -> u8 {
        match self {
            Hidden::Visible => 0,
            Hidden::PrivateUse => 1,
            Hidden::Reserved => 2,
            Hidden::TrailingDotOrSpace => 3,
            Hidden::TooLong => 4,
            Hidden::Symlink => 5,
            Hidden::Other => 255,
        }
    }

    fn from_raw(v: u8) -> Self {
        match v {
            0 => Hidden::Visible,
            1 => Hidden::PrivateUse,
            2 => Hidden::Reserved,
            3 => Hidden::TrailingDotOrSpace,
            4 => Hidden::TooLong,
            5 => Hidden::Symlink,
            _ => Hidden::Other,
        }
    }
}

/// Polozka adresare nebo vysledek `Stat`. Casy v ns od 1. 1. 1970 (UTC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    /// Jmeno v nasem FS (u korene prazdne).
    pub name: String,
    /// Jmeno, pod kterym polozku vidi Windows (prazdne = skryta).
    pub windows_name: String,
    pub hidden: Hidden,
    /// Jmeno nebo cil odkazu se do zpravy nevesel a je zkraceny (jen
    /// u extremne dlouhych jmen pres WinFsp).
    pub truncated: bool,
    pub kind: Kind,
    /// Identita objektu: `oid` + `generation` (vznik) se nemeni pri
    /// prejmenovani ani presunu.
    pub oid: u64,
    pub generation: u64,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    pub btime: u64,
    /// U adresare: soucet velikosti souboru v podstromu (okamzite).
    pub tree_bytes: u64,
    /// U adresare: pocet souboru v podstromu.
    pub tree_files: u64,
    /// U adresare: pocet adresaru v podstromu (bez sebe).
    pub tree_dirs: u64,
    /// U symbolickeho odkazu jeho cil, jinak prazdne.
    pub link_target: String,
}

impl EntryInfo {
    pub fn put(&self, w: &mut Writer) {
        w.str(&self.name)
            .str(&self.windows_name)
            .u8(self.hidden.raw())
            .u8(self.truncated as u8)
            .u8(self.kind.raw())
            .u64(self.oid)
            .u64(self.generation)
            .u64(self.size)
            .u64(self.mtime)
            .u64(self.ctime)
            .u64(self.btime)
            .u64(self.tree_bytes)
            .u64(self.tree_files)
            .u64(self.tree_dirs)
            .str(&self.link_target);
    }

    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            name: r.str()?,
            windows_name: r.str()?,
            hidden: Hidden::from_raw(r.u8()?),
            truncated: r.u8()? != 0,
            kind: Kind::from_raw(r.u8()?)?,
            oid: r.u64()?,
            generation: r.u64()?,
            size: r.u64()?,
            mtime: r.u64()?,
            ctime: r.u64()?,
            btime: r.u64()?,
            tree_bytes: r.u64()?,
            tree_files: r.u64()?,
            tree_dirs: r.u64()?,
            link_target: r.str()?,
        })
    }

    /// Kolik bajtu polozka zabere ve zprave.
    pub fn encoded_len(&self) -> usize {
        2 + self.name.len() + 2 + self.windows_name.len() + 3 + 8 * 9 + 2 + self.link_target.len()
    }

    /// Zkrati polozku, aby se vesla do `budget` bajtu: nejdriv cil odkazu,
    /// pak jmena (na hranici znaku). Nastavi `truncated`.
    pub fn fit(&mut self, budget: usize) {
        if self.encoded_len() <= budget {
            return;
        }
        self.truncated = true;
        self.link_target.clear();
        let fixed = self.encoded_len() - self.name.len() - self.windows_name.len();
        let room = budget.saturating_sub(fixed) / 2;
        cut(&mut self.name, room);
        cut(&mut self.windows_name, room);
    }
}

fn cut(s: &mut String, max: usize) {
    if s.len() > max {
        let mut n = max;
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        s.truncate(n);
    }
}

/// Stranka vypisu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    pub entries: Vec<EntryInfo>,
    /// Kurzor dalsi stranky; prazdny = konec vypisu.
    pub next: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: u64,
    pub name: String,
    /// Generace ulozeneho stavu, ktery snapshot zachytil.
    pub generation: u64,
}

/// Uspesna odpoved; tvar odpovida operaci pozadavku.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    Hello(HelloInfo),
    VolumeInfo(VolumeInfo),
    Entry(EntryInfo),
    List(ListPage),
    Data(Vec<u8>),
    /// Stranka snapshotu; `next` = `after` dalsi stranky, 0 = konec.
    Snapshots {
        list: Vec<SnapshotInfo>,
        next: u64,
    },
    /// `SnapshotCreate`: id noveho snapshotu.
    Created(u64),
    /// `Rename`, `Commit`: hotovo; nese generaci ulozeneho stavu.
    Done(u64),
}

/// Hlavicka odpovedi: `MAGIC`, `VERSION`, `op`, `status`.
fn header(w: &mut Writer, op: u16, status: i32) {
    w.u32(MAGIC).u16(VERSION).u16(op).i32(status);
}

/// Velikost hlavicky odpovedi.
pub const RESPONSE_HEADER: usize = 12;

/// Zakoduje chybovou odpoved.
pub fn encode_error(op: u16, status: i32) -> Vec<u8> {
    let mut w = Writer::new();
    header(&mut w, op, status);
    w.buf
}

impl Response {
    pub fn encode(&self, op: u16) -> Vec<u8> {
        let mut w = Writer::new();
        header(&mut w, op, error::OK);
        match self {
            Response::Hello(h) => {
                w.u16(h.version)
                    .u64(h.features)
                    .str(&h.server)
                    .u32(h.max_message);
            }
            Response::VolumeInfo(v) => {
                w.str(&v.label)
                    .u32(v.block_size)
                    .u64(v.block_count)
                    .u64(v.free_blocks)
                    .u64(v.deferred_blocks)
                    .u64(v.generation)
                    .u8(v.algo)
                    .u8(v.read_only as u8)
                    .u64(v.fallback)
                    .u64(v.replayed);
            }
            Response::Entry(e) => e.put(&mut w),
            Response::List(p) => {
                w.u32(p.entries.len() as u32);
                for e in &p.entries {
                    e.put(&mut w);
                }
                w.str(&p.next);
            }
            Response::Data(d) => {
                w.bytes(d);
            }
            Response::Snapshots { list, next } => {
                w.u32(list.len() as u32);
                for s in list {
                    w.u64(s.id).str(&s.name).u64(s.generation);
                }
                w.u64(*next);
            }
            Response::Created(id) => {
                w.u64(*id);
            }
            Response::Done(g) => {
                w.u64(*g);
            }
        }
        w.buf
    }

    /// Dekoduje odpoved na pozadavek s operaci `op`. `Err(kod)` = server
    /// vratil chybu nebo odpoved neni platna (`CORRUPTED`).
    pub fn decode(op: u16, b: &[u8]) -> Result<Self, i32> {
        let mut r = Reader::new(b);
        let bad = error::CORRUPTED;
        if r.u32() != Some(MAGIC) || r.u16() != Some(VERSION) || r.u16() != Some(op) {
            return Err(bad);
        }
        let status = r.i32().ok_or(bad)?;
        if status != error::OK {
            return Err(status);
        }
        let resp = (|| {
            Some(match op {
                op::HELLO => Response::Hello(HelloInfo {
                    version: r.u16()?,
                    features: r.u64()?,
                    server: r.str()?,
                    max_message: r.u32()?,
                }),
                op::VOLUME_INFO => Response::VolumeInfo(VolumeInfo {
                    label: r.str()?,
                    block_size: r.u32()?,
                    block_count: r.u64()?,
                    free_blocks: r.u64()?,
                    deferred_blocks: r.u64()?,
                    generation: r.u64()?,
                    algo: r.u8()?,
                    read_only: r.u8()? != 0,
                    fallback: r.u64()?,
                    replayed: r.u64()?,
                }),
                op::STAT => Response::Entry(EntryInfo::get(&mut r)?),
                op::LIST => {
                    let n = r.u32()? as usize;
                    // Kazda polozka ma aspon 81 bajtu: horni mez z delky
                    // zpravy, at poskozeny pocet nealokuje gigabajty.
                    if n > b.len() / 81 {
                        return None;
                    }
                    let mut entries = Vec::with_capacity(n);
                    for _ in 0..n {
                        entries.push(EntryInfo::get(&mut r)?);
                    }
                    Response::List(ListPage {
                        entries,
                        next: r.str()?,
                    })
                }
                op::READ => Response::Data(r.bytes()?),
                op::SNAPSHOTS => {
                    let n = r.u32()? as usize;
                    if n > b.len() / 18 {
                        return None;
                    }
                    let mut v = Vec::with_capacity(n);
                    for _ in 0..n {
                        v.push(SnapshotInfo {
                            id: r.u64()?,
                            name: r.str()?,
                            generation: r.u64()?,
                        });
                    }
                    Response::Snapshots {
                        list: v,
                        next: r.u64()?,
                    }
                }
                op::SNAPSHOT_CREATE => Response::Created(r.u64()?),
                op::RENAME | op::COMMIT => Response::Done(r.u64()?),
                _ => return None,
            })
        })()
        .ok_or(bad)?;
        // Novejsi server muze na konec pridat polozky: zbytek se ignoruje.
        Ok(resp)
    }
}
