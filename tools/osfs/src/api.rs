//! Serverova cast rozsireneho API (`osfs-proto`): vykonani pozadavku nad
//! `fs::Fs`.
//!
//! Nezavisi na WinFsp, takze jde testovat nad `MemDevice`
//! (`tests/api.rs`). Ovladac (`mount::ops::control`) jen preda bajty
//! pozadavku a vrati bajty odpovedi; co se ma stat kolem (mezipamet jmen,
//! handly, priznak zmeny), resi pres [`Host`].

use fs::{BlockDevice, Error, Fs, Kind, Stat, fold_name};
use osfs_proto::{
    EntryInfo, HelloInfo, Hidden, ListPage, MAX_CURSOR, MAX_MESSAGE, MAX_READ, PathArg,
    RESPONSE_HEADER, Request, Response, SnapshotInfo, VolumeInfo, encode_error, error, feature,
};

use crate::image::join;
use crate::names::{self, Hidden as WinHidden};
use crate::versions::{self, DatasetPath};

/// Okoli serveru (ovladac).
pub trait Host {
    /// Pred zmenou svazku: `Err(kod)` = zmeny nejsou povolene (jen pro
    /// cteni). Jinak si ovladac poznamena, ze je co ukladat.
    fn mutating(&mut self) -> Result<(), i32>;
    /// Po prejmenovani: mezipameti a otevrene handly pod starym jmenem.
    fn renamed(&mut self, from: &str, to: &str);
    /// Po ulozeni (commit, snapshot).
    fn committed(&mut self);
}

/// Snimek vypisu adresare pro strankovani: dalsi stranky stejneho
/// adresare pokracuji v nem (jinak by se adresar cetl cely pro kazdou
/// stranku). Drzi ho handle, pres ktery klient vola.
pub struct ListCache {
    key: String,
    entries: Vec<fs::DirEntry>,
}

/// Odkud pozadavek prisel.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Code {
    Read,
    Write,
}

fn code_of(e: Error) -> i32 {
    e.code()
}

/// Cesta z pozadavku -> cesta v nasem FS.
pub fn resolve(p: &PathArg) -> Result<String, i32> {
    match p {
        PathArg::Fs(s) => {
            if !s.starts_with('/') {
                return Err(error::INVALID_ARGS);
            }
            Ok(s.clone())
        }
        PathArg::Windows(w) => names::path_from_windows(w).map_err(|_| error::NOT_FOUND),
    }
}

fn parent_of(path: &str) -> &str {
    let t = path.trim_end_matches('/');
    match t.rfind('/') {
        Some(0) | None => "/",
        Some(i) => &t[..i],
    }
}

fn last_comp(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or("")
}

fn hidden_of(name: &str, kind: Kind) -> (String, Hidden) {
    if name.is_empty() {
        return (String::new(), Hidden::Visible);
    }
    match names::to_windows_checked(name) {
        Err(h) => (
            String::new(),
            match h {
                WinHidden::PrivateUse => Hidden::PrivateUse,
                WinHidden::Reserved => Hidden::Reserved,
                WinHidden::TrailingDotOrSpace => Hidden::TrailingDotOrSpace,
                WinHidden::TooLong => Hidden::TooLong,
            },
        ),
        Ok(_) if kind == Kind::Symlink => (String::new(), Hidden::Symlink),
        Ok(w) => (w, Hidden::Visible),
    }
}

fn proto_kind(k: Kind) -> osfs_proto::Kind {
    match k {
        Kind::File => osfs_proto::Kind::File,
        Kind::Dir => osfs_proto::Kind::Dir,
        Kind::Symlink => osfs_proto::Kind::Symlink,
        Kind::Dataset => osfs_proto::Kind::Dataset,
    }
}

/// Cas FS -> cas v protokolu (stejny tvar, nic se neztrati).
fn time(t: fs::Timestamp) -> osfs_proto::Time {
    osfs_proto::Time {
        secs: t.secs,
        nanos: t.nanos,
    }
}

/// Polozka odpovedi. `kind` = druh polozky v adresari (u datasetu
/// `Dataset`; `stat` datasetu vraci jeho korenovy adresar).
fn entry(name: &str, s: &Stat, kind: Kind, link_target: String) -> EntryInfo {
    let (windows_name, hidden) = hidden_of(name, kind);
    EntryInfo {
        name: name.into(),
        windows_name,
        hidden,
        truncated: false,
        kind: proto_kind(kind),
        oid: s.oid,
        generation: s.generation,
        size: s.size,
        mtime: time(s.mtime),
        ctime: time(s.ctime),
        btime: time(s.btime),
        tree_bytes: s.tree_bytes,
        tree_files: s.tree_files,
        tree_dirs: s.tree_dirs,
        dataset: s.dataset,
        link_target,
        // Bity jsou stejne jako ve FS; vlastnika a prava polozka nenese.
        known: s.known & ENTRY_AVAIL,
        withheld: s.hidden & ENTRY_AVAIL,
    }
}

/// Bity dostupnosti poli, ktera `EntryInfo` nese.
const ENTRY_AVAIL: u16 = osfs_proto::avail::SIZE
    | osfs_proto::avail::MTIME
    | osfs_proto::avail::CTIME
    | osfs_proto::avail::BTIME
    | osfs_proto::avail::TREE;

// Bity FS a protokolu osfs jsou stejne (predavaji se beze zmeny).
const _: () = assert!(
    fs::avail::SIZE == osfs_proto::avail::SIZE
        && fs::avail::MTIME == osfs_proto::avail::MTIME
        && fs::avail::CTIME == osfs_proto::avail::CTIME
        && fs::avail::BTIME == osfs_proto::avail::BTIME
        && fs::avail::OWNER == osfs_proto::avail::OWNER
        && fs::avail::TREE == osfs_proto::avail::TREE
        && fs::avail::RIGHTS == osfs_proto::avail::RIGHTS
);

struct Ctx<'a, D: BlockDevice> {
    fs: &'a mut Fs<D>,
    host: &'a mut dyn Host,
    now: fs::Timestamp,
}

impl<D: BlockDevice> Ctx<'_, D> {
    /// Dataset, ve kterem lezi `path` (snapshot `at` patri jemu).
    fn dataset(&mut self, path: &str) -> Result<DatasetPath, i32> {
        versions::dataset_of(self.fs, path).map_err(code_of)
    }

    fn stat_at(&mut self, at: &str, path: &str) -> Result<Stat, i32> {
        if at.is_empty() {
            return self.fs.stat(path).map_err(code_of);
        }
        let d = self.dataset(path)?;
        self.fs
            .snapshot_stat_of(&d.root, at, &d.inner)
            .map_err(code_of)
    }

    fn list_at(&mut self, at: &str, path: &str) -> Result<Vec<fs::DirEntry>, i32> {
        if at.is_empty() {
            return self.fs.list(path).map_err(code_of);
        }
        let d = self.dataset(path)?;
        self.fs
            .snapshot_list_of(&d.root, at, &d.inner)
            .map_err(code_of)
    }

    fn read_at(&mut self, at: &str, path: &str, offset: u64, len: usize) -> Result<Vec<u8>, i32> {
        if at.is_empty() {
            return self.fs.read(path, offset, len).map_err(code_of);
        }
        let d = self.dataset(path)?;
        self.fs
            .snapshot_read_of(&d.root, at, &d.inner, offset, len)
            .map_err(code_of)
    }

    /// Cil odkazu (zivy stav i snapshot).
    fn link_target(&mut self, at: &str, path: &str, kind: Kind) -> String {
        if kind != Kind::Symlink {
            return String::new();
        }
        if at.is_empty() {
            return self.fs.readlink(path).unwrap_or_default();
        }
        match self.dataset(path) {
            Ok(d) => self
                .fs
                .snapshot_readlink_of(&d.root, at, &d.inner)
                .unwrap_or_default(),
            Err(_) => String::new(),
        }
    }

    /// Druh objektu na `path` tak, jak ho ukazuje adresar: koren
    /// vnoreneho datasetu = `Dataset` (jeho `stat` vraci adresar).
    fn kind_of(&mut self, path: &str, stat_kind: Kind) -> Kind {
        if stat_kind == Kind::Dir && versions::is_dataset_root(self.fs, path) {
            Kind::Dataset
        } else {
            stat_kind
        }
    }

    /// Ulozene jmeno posledni slozky (skutecna velikost pismen) z vypisu
    /// rodice; koren = "".
    fn stored_name(&mut self, at: &str, path: &str) -> Result<String, i32> {
        let name = last_comp(path);
        if name.is_empty() {
            return Ok(String::new());
        }
        let want = fold_name(name);
        // Koren datasetu: rodic lezi v jinem datasetu, jehoz snapshoty
        // jsou jine - jmeno z zive verze rodice.
        let at = if !at.is_empty() && self.dataset(path)?.inner == "/" {
            ""
        } else {
            at
        };
        let list = self.list_at(at, parent_of(path))?;
        list.into_iter()
            .find(|e| fold_name(&e.name) == want)
            .map(|e| e.name)
            .ok_or(error::NOT_FOUND)
    }

    fn mutate(&mut self) -> Result<(), i32> {
        self.host.mutating()?;
        self.fs.set_time(self.now);
        Ok(())
    }

    fn run(&mut self, req: &Request, cache: &mut Option<ListCache>) -> Result<Response, i32> {
        match req {
            Request::Hello => Ok(Response::Hello(HelloInfo {
                version: osfs_proto::VERSION,
                features: feature::SNAPSHOTS | feature::RENAME | feature::READ | feature::DATASETS,
                server: format!("osfs {} (WinFsp)", env!("CARGO_PKG_VERSION")),
                max_message: MAX_MESSAGE as u32,
            })),
            Request::VolumeInfo => {
                let i = self.fs.info();
                Ok(Response::VolumeInfo(VolumeInfo {
                    label: i.label,
                    block_size: i.block_size,
                    block_count: i.block_count,
                    free_blocks: i.free_blocks,
                    deferred_blocks: i.deferred_blocks,
                    generation: i.generation,
                    algo: i.algo as u8,
                    read_only: i.read_only,
                    fallback: i.fallback,
                    replayed: i.replayed,
                }))
            }
            Request::Stat { at, path } => {
                let path = resolve(path)?;
                let s = self.stat_at(at, &path)?;
                let name = self.stored_name(at, &path)?;
                let target = self.link_target(at, &path, s.kind);
                let kind = self.kind_of(&path, s.kind);
                let mut e = entry(&name, &s, kind, target);
                e.fit(MAX_MESSAGE - RESPONSE_HEADER);
                Ok(Response::Entry(e))
            }
            Request::List { at, path, cursor } => {
                let path = resolve(path)?;
                self.list_page(at, &path, cursor, cache).map(Response::List)
            }
            Request::Read {
                at,
                path,
                offset,
                len,
            } => {
                let path = resolve(path)?;
                let len = (*len).min(MAX_READ) as usize;
                let data = self.read_at(at, &path, *offset, len)?;
                Ok(Response::Data(data))
            }
            Request::Rename { from, to } => {
                let from = resolve(from)?;
                let to = match to {
                    // Cil z Windows musi byt platne jmeno (ne "nenalezeno").
                    PathArg::Windows(w) => {
                        names::path_from_windows(w).map_err(|_| error::INVALID_ARGS)?
                    }
                    p => resolve(p)?,
                };
                self.mutate()?;
                self.fs.rename(&from, &to).map_err(code_of)?;
                self.host.renamed(&from, &to);
                *cache = None;
                Ok(Response::Done(self.fs.info().generation))
            }
            Request::Snapshots { path, after } => {
                let d = self.dataset(&resolve(path)?)?;
                let all = self.fs.snapshots_of(&d.root).map_err(code_of)?;
                // Pocet + `next` na konci.
                let mut used = RESPONSE_HEADER + 4 + 8;
                let mut list = Vec::new();
                let mut next = 0;
                for s in all.into_iter().filter(|s| s.id > *after) {
                    let size = 8 + 2 + s.name.len() + 8;
                    if used + size > MAX_MESSAGE {
                        next = list.last().map(|l: &SnapshotInfo| l.id).unwrap_or(0);
                        break;
                    }
                    used += size;
                    list.push(SnapshotInfo {
                        id: s.id,
                        name: s.name,
                        generation: s.generation,
                    });
                }
                Ok(Response::Snapshots { list, next })
            }
            Request::SnapshotCreate { path, name } => {
                let d = self.dataset(&resolve(path)?)?;
                self.mutate()?;
                let id = self.fs.snapshot_of(&d.root, name).map_err(code_of)?;
                self.host.committed();
                Ok(Response::Created(id))
            }
            Request::Commit => {
                self.mutate()?;
                self.fs.commit().map_err(code_of)?;
                self.host.committed();
                Ok(Response::Done(self.fs.info().generation))
            }
        }
    }

    fn list_page(
        &mut self,
        at: &str,
        path: &str,
        cursor: &str,
        cache: &mut Option<ListCache>,
    ) -> Result<ListPage, i32> {
        let key = format!("{at}\u{0}{}", fold_name(path));
        let reuse = !cursor.is_empty() && cache.as_ref().is_some_and(|c| c.key == key);
        if !reuse {
            let entries = self.list_at(at, path)?;
            *cache = Some(ListCache { key, entries });
        }
        let entries = &cache.as_ref().expect("prave nastaveno").entries;
        // Poradi podle slozeneho jmena (tak vraci `list`). Kurzor:
        // `=` + slozene jmeno posledni vracene polozky (prezije i zmeny
        // adresare mezi strankami), u extremne dlouhych jmen `#` + poradi
        // ve snimku vypisu (plati jen se stejnym handlem).
        let start = if cursor.is_empty() {
            0
        } else if let Some(f) = cursor.strip_prefix('=') {
            entries.partition_point(|e| fold_name(&e.name).as_str() <= f)
        } else if let Some(i) = cursor.strip_prefix('#') {
            if !reuse {
                return Err(error::INVALID_ARGS);
            }
            i.parse::<usize>()
                .ok()
                .filter(|&i| i <= entries.len())
                .ok_or(error::INVALID_ARGS)?
        } else {
            return Err(error::INVALID_ARGS);
        };
        let mut out = Vec::new();
        // Hlavicka + pocet; na konci misto pro kurzor.
        let mut used = RESPONSE_HEADER + 4;
        let reserve = 2 + MAX_CURSOR;
        let mut next = String::new();
        let mut page_full = false;
        let rest: Vec<(usize, String, Kind)> = entries[start..]
            .iter()
            .enumerate()
            .map(|(i, e)| (start + i, e.name.clone(), e.kind))
            .collect();
        for (idx, name, kind) in &rest {
            let full = join(path, name);
            // Vnoreny dataset ma vlastni snapshoty: pohled snapshotu rodice
            // do nej neprechazi, udaje se berou ze zive verze.
            let at_here = if *kind == Kind::Dataset { "" } else { at };
            let s = match self.stat_at(at_here, &full) {
                Ok(s) => s,
                // Smazano od porizeni snimku.
                Err(error::NOT_FOUND) => continue,
                Err(e) => return Err(e),
            };
            let target = self.link_target(at, &full, *kind);
            let mut e = entry(name, &s, *kind, target);
            if used + e.encoded_len() + reserve > MAX_MESSAGE {
                if out.is_empty() {
                    // Jedina polozka se nevejde ani sama: zkratit.
                    e.fit(MAX_MESSAGE - used - reserve);
                } else {
                    page_full = true;
                    break;
                }
            }
            used += e.encoded_len();
            out.push(e);
            // Kurzor za tuto polozku (pouzije se, kdyz dalsi nevejde).
            let folded = fold_name(name);
            next = if folded.len() < MAX_CURSOR {
                format!("={folded}")
            } else {
                format!("#{}", idx + 1)
            };
        }
        if !page_full {
            // Vse vraceno: konec vypisu.
            next.clear();
        }
        Ok(ListPage { entries: out, next })
    }
}

/// Vykona pozadavek a vrati zakodovanou odpoved (vzdy platnou zpravu, i pri
/// chybe). `code` = kterym IOCTL kodem prisel; zapisova operace poslana
/// kodem pro cteni se odmitne.
pub fn handle<D: BlockDevice>(
    fs: &mut Fs<D>,
    host: &mut dyn Host,
    code: Code,
    input: &[u8],
    cache: &mut Option<ListCache>,
    now: fs::Timestamp,
) -> Vec<u8> {
    let req = match Request::decode(input) {
        Ok(r) => r,
        Err(e) => {
            // Operace z hlavicky, kdyz ji jde precist (klient pak pozna,
            // na co je odpoved).
            let op = input
                .get(6..8)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .unwrap_or(0);
            return encode_error(op, e);
        }
    };
    let op = req.op();
    if req.is_write() && code != Code::Write {
        return encode_error(op, error::ACCESS_DENIED);
    }
    let mut ctx = Ctx { fs, host, now };
    match ctx.run(&req, cache) {
        Ok(resp) => {
            let b = resp.encode(op);
            if b.len() > MAX_MESSAGE {
                encode_error(op, error::BUFFER_TOO_SMALL)
            } else {
                b
            }
        }
        Err(e) => encode_error(op, e),
    }
}

/// Kody, ktere ovladac prijima (vse ostatni = neplatny pozadavek).
pub fn code_from_ioctl(ioctl: u32) -> Option<Code> {
    match ioctl {
        osfs_proto::IOCTL_READ => Some(Code::Read),
        osfs_proto::IOCTL_WRITE => Some(Code::Write),
        _ => None,
    }
}
