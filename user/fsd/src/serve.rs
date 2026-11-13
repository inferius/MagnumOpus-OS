//! Obsluha pozadavku protokolu `fsproto`.

use alloc::vec::Vec;

use fs::{Fs, Kind};
use fsproto::hist::op as hop;
use fsproto::{DirEntry, Info, Reader, Stat, StatEntry, Writer, kind, op};
use libos::{Error, Result};

use crate::blkdev::BlkDev;
use crate::serve_v2::{Reply, Session};

/// Cas pro zmeny ve FS: realny cas. Bez realnych hodin cas od startu
/// systemu (data pak vypadaji jako z roku 1970).
pub(crate) fn fs_now() -> fs::Timestamp {
    libos::time::wall().unwrap_or_else(|| fs::Timestamp::new(0, libos::time::now() as i64))
}

// Bity dostupnosti FS a protokolu jsou stejne (predavaji se beze zmeny).
const _: () = assert!(
    fs::avail::SIZE == fsproto::avail::SIZE
        && fs::avail::MTIME == fsproto::avail::MTIME
        && fs::avail::CTIME == fsproto::avail::CTIME
        && fs::avail::BTIME == fsproto::avail::BTIME
        && fs::avail::OWNER == fsproto::avail::OWNER
        && fs::avail::TREE == fsproto::avail::TREE
        && fs::avail::RIGHTS == fsproto::avail::RIGHTS
        && fs::avail::ALL == fsproto::avail::ALL
);

pub(crate) fn stat(s: fs::Stat) -> Stat {
    Stat {
        oid: s.oid,
        kind: match s.kind {
            Kind::File => kind::FILE,
            Kind::Dir => kind::DIR,
            Kind::Symlink => kind::SYMLINK,
            Kind::Dataset => kind::DATASET,
        },
        size: s.size,
        mtime: s.mtime,
        ctime: s.ctime,
        btime: s.btime,
        owner: s.owner,
        rights: s.rights,
        tree_bytes: s.tree_bytes,
        tree_files: s.tree_files,
        tree_dirs: s.tree_dirs,
        known: s.known,
        hidden: s.hidden,
    }
}

/// Bity dostupnosti poli, ktera `StatEntry` nese.
const ENTRY_AVAIL: u16 = fsproto::avail::SIZE | fsproto::avail::MTIME | fsproto::avail::RIGHTS | fsproto::avail::TREE;

fn kind_raw(k: Kind) -> u8 {
    match k {
        Kind::File => kind::FILE,
        Kind::Dir => kind::DIR,
        Kind::Symlink => kind::SYMLINK,
        Kind::Dataset => kind::DATASET,
    }
}

pub(crate) fn entries(list: Vec<fs::DirEntry>) -> Vec<DirEntry> {
    list.into_iter()
        .map(|e| DirEntry { name: e.name, kind: kind_raw(e.kind), oid: e.oid })
        .collect()
}

fn io_len(r: &mut Reader) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > fsproto::MAX_IO {
        return Err(Error::InvalidArgs);
    }
    Ok(n)
}

/// Stav sluzby.
pub struct Service {
    pub fs: Option<Fs<BlkDev>>,
    /// Povolene testovaci operace (`SIMULATE_POWER_LOSS`).
    pub debug: bool,
    /// Pocitadlo jmen docasnych souboru podmineneho ulozeni.
    pub temp_counter: u64,
    /// Klient prave udelal COMMIT (posunout automaticky commit).
    pub commit_requested: bool,
    /// Cislovani relaci (`CONNECT`, `CONNECT_AS`).
    pub session_counter: u64,
    /// Prace v necinnosti: po defragmentaci deduplikace.
    pub dedup_phase: bool,
    /// Bezi scrub (smycka dela kroky mezi pozadavky).
    pub scrubbing: bool,
    /// Prehled uloh (scrub, deduplikace).
    pub tasks: crate::tasks::FsTasks,
    /// Po restartu: kdy uklidit sirotky, ktere nikdo znovu neotevrel.
    pub orphan_cleanup: Option<u64>,
}

/// Klic rozpracovaneho vypisu: (dataset, snapshot) u vypisu snapshotu
/// (`None` = ziva verze) a cesta.
pub type ListKey = (Option<(alloc::string::String, alloc::string::String)>, alloc::string::String);

/// Klic a obsah rozpracovaneho vypisu (polozky serazene podle slozeneho
/// jmena - kurzor se hleda pulenim, slozeny tvar se neuklada).
pub type ListCache = (ListKey, Vec<fs::DirEntry>);

impl Service {
    fn fs(&mut self) -> Result<&mut Fs<BlkDev>> {
        self.fs.as_mut().ok_or(Error::IoError)
    }

    /// Zpracuje jeden pozadavek relace `sess`; vraci odpoved. FS po dobu
    /// pozadavku kontroluje opravneni podle identity relace.
    pub fn handle(&mut self, sess: &mut Session, msg: &[u8]) -> Result<Reply> {
        if let Some(fs) = self.fs.as_mut() {
            fs.set_principal(sess.identity.clone());
        }
        let r = self.handle_inner(sess, msg);
        if let Some(fs) = self.fs.as_mut() {
            fs.set_principal(None);
        }
        r
    }

    fn handle_inner(&mut self, sess: &mut Session, msg: &[u8]) -> Result<Reply> {
        let mut r = Reader::new(msg);
        let opcode = r.u32()?;
        // Rozpracovany vypis zahodi kazda jina operace (nikdy neni zastaraly).
        if !matches!(opcode, op::LIST_PAGE | op::SNAP_LIST_PAGE | op::LIST_STAT_PAGE | hop::SNAP_LIST_OF) {
            sess.list_cache = None;
        }
        if opcode == op::COMMIT {
            self.commit_requested = true;
        }
        if let Some(res) = self.handle_v2(sess, opcode, &mut r) {
            return res;
        }
        if let Some(res) = self.handle_hist(sess, opcode, &mut r) {
            return res;
        }
        if let Some(res) = self.handle_data(sess, opcode, &mut r) {
            return res;
        }
        let mut w = Writer::new();
        if opcode == op::SIMULATE_POWER_LOSS {
            if !self.debug {
                return Err(Error::AccessDenied);
            }
            let dev = self.fs.take().ok_or(Error::IoError)?.into_device();
            let fs = Fs::mount(dev)?;
            w.u64(fs.info().replayed);
            self.fs = Some(fs);
            return Ok(Reply::body(w));
        }
        // Dlouha operace nad celym svazkem: jen v rezimu `debug` (v1 nema
        // overeni klienta; s protokolem v2 pujde podle opravneni).
        if opcode == op::CHECK && !self.debug {
            return Err(Error::AccessDenied);
        }
        // Operace nad celym korenovym datasetem (jeho snapshoty, kontrola,
        // defragmentace) jen relace, ktera vidi cely FS.
        if matches!(
            opcode,
            op::SNAPSHOT
                | op::SNAPSHOTS
                | op::DELETE_SNAPSHOT
                | op::SNAP_STAT
                | op::SNAP_LIST
                | op::SNAP_READ
                | op::SNAP_LIST_PAGE
                | op::CHECK
                | op::DEFRAG
        ) {
            sess.full_root()?;
        }
        let now = fs_now();
        let fs = self.fs()?;
        fs.set_time(now);
        match opcode {
            op::INFO => {
                let i = fs.info();
                Info {
                    label: i.label,
                    block_size: i.block_size,
                    block_count: i.block_count,
                    free_blocks: i.free_blocks,
                    deferred_blocks: i.deferred_blocks,
                    generation: i.generation,
                    algo: i.algo as u8,
                    fallback: i.fallback,
                    replayed: i.replayed,
                }
                .put(&mut w);
            }
            op::STAT => stat(fs.stat(&sess.map(&r.str()?, false)?)?).put(&mut w),
            op::LIST => {
                let list = entries(fs.list(&sess.map(&r.str()?, false)?)?);
                fsproto::encode_list(&mut w, &list);
                if w.len() > fsproto::MAX_REPLY {
                    return Err(Error::BufferTooSmall);
                }
            }
            op::LIST_PAGE | op::SNAP_LIST_PAGE | op::LIST_STAT_PAGE | hop::SNAP_LIST_OF => {
                // Zdroj vypisu: (dataset, snapshot) nebo ziva verze. Cesta
                // ve snapshotu je cesta uvnitr datasetu (bez prekladu).
                let snap = match opcode {
                    op::SNAP_LIST_PAGE => Some((alloc::string::String::from("/"), r.str()?)),
                    hop::SNAP_LIST_OF => {
                        let ds = sess.map(&r.str()?, false)?;
                        Some((ds, r.str()?))
                    }
                    _ => None,
                };
                let path = match snap {
                    Some(_) => r.str()?,
                    None => sess.map(&r.str()?, false)?,
                };
                let after = r.str()?;
                let max = r.u32()? as usize;
                let with_stat = opcode == op::LIST_STAT_PAGE;
                if with_stat {
                    // Metadata polozek = STAT kazde z nich: bez TRAVERSE
                    // k adresari by STAT nesel (viz `op::LIST_STAT_PAGE`).
                    if fs.stat(&path)?.rights & fs::rights::TRAVERSE == 0 {
                        return Err(Error::AccessDenied);
                    }
                }
                // Stejny vypis jako LIST_PAGE (sdileny rozpracovany vypis
                // nevadi: obsah je pro obe operace stejny).
                let key = (snap.clone(), path.clone());
                // Vypis pro dalsi stranky drzet v pameti (jinak by se pro
                // kazdou stranku sestavoval znovu); zahodi ho kazda jina
                // operace, takze nikdy neni zastaraly.
                let cached = sess.list_cache.take().filter(|(k, _)| *k == key && !after.is_empty());
                let list = match cached {
                    Some((_, list)) => list,
                    None => {
                        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
                        match &snap {
                            Some((ds, s)) => fs.snapshot_list_of(ds, s, &path)?,
                            None => fs.list(&path)?,
                        }
                    }
                };
                let start = if after.is_empty() {
                    0
                } else {
                    let a = fs::fold_name(&after);
                    // Vypis je serazeny podle slozeneho jmena (fs::list).
                    list.partition_point(|e| fs::fold_name(&e.name) <= a)
                };
                let n = if with_stat {
                    let rest = &list[start..];
                    let n = fsproto::list_stat_fit(rest.iter().map(|e| e.name.as_str()), max, fsproto::MAX_REPLY);
                    let fs = self.fs.as_mut().ok_or(Error::IoError)?;
                    let page = rest[..n]
                        .iter()
                        .map(|e| {
                            let st = fs.stat(&alloc::format!("{}/{}", path.trim_end_matches('/'), e.name))?;
                            // Druh a oid jako LIST (u datasetu vstup, ne jeho koren).
                            Ok(StatEntry {
                                name: e.name.clone(),
                                kind: kind_raw(e.kind),
                                oid: e.oid,
                                size: st.size,
                                mtime: st.mtime,
                                rights: st.rights,
                                tree_bytes: st.tree_bytes,
                                tree_files: st.tree_files,
                                tree_dirs: st.tree_dirs,
                                // Polozka nenese ctime, btime ani vlastnika.
                                known: st.known & ENTRY_AVAIL,
                                hidden: st.hidden & ENTRY_AVAIL,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    fsproto::encode_list_stat_page(&mut w, &page, n < rest.len());
                    n
                } else {
                    let page: Vec<DirEntry> = entries(list[start..].to_vec());
                    fsproto::encode_list_page(&mut w, &page, max, fsproto::MAX_REPLY)
                };
                if start + n < list.len() {
                    sess.list_cache = Some((key, list));
                }
                return Ok(Reply::body(w));
            }
            op::READ => {
                let path = sess.map(&r.str()?, false)?;
                let off = r.u64()?;
                let n = io_len(&mut r)?;
                w.bytes(&fs.read(&path, off, n)?);
            }
            op::WRITE => {
                let path = sess.map(&r.str()?, true)?;
                let off = r.u64()?;
                fs.write(&path, off, r.rest())?;
            }
            op::CREATE => {
                let path = sess.map(&r.str()?, true)?;
                let k = match r.u8()? {
                    kind::FILE => Kind::File,
                    kind::DIR => Kind::Dir,
                    _ => return Err(Error::InvalidArgs),
                };
                fs.create(&path, k)?;
            }
            op::REMOVE => fs.remove(&sess.map(&r.str()?, true)?)?,
            op::RENAME => {
                let from = sess.map(&r.str()?, true)?;
                let to = sess.map(&r.str()?, true)?;
                fs.rename(&from, &to)?;
            }
            op::TRUNCATE => {
                let path = sess.map(&r.str()?, true)?;
                fs.truncate(&path, r.u64()?)?;
            }
            op::COMMIT => fs.commit()?,
            op::FSYNC => fs.fsync()?,
            op::SNAPSHOT => {
                w.u64(fs.snapshot(&r.str()?)?);
            }
            op::SNAPSHOTS => {
                let list = fs.snapshots()?;
                w.u32(list.len() as u32);
                for s in list {
                    w.u64(s.id).u64(s.generation).str(&s.name);
                }
            }
            op::DELETE_SNAPSHOT => fs.delete_snapshot(&r.str()?)?,
            op::SNAP_STAT => {
                let snap = r.str()?;
                stat(fs.snapshot_stat(&snap, &r.str()?)?).put(&mut w);
            }
            op::SNAP_LIST => {
                let snap = r.str()?;
                fsproto::encode_list(&mut w, &entries(fs.snapshot_list(&snap, &r.str()?)?));
                if w.len() > fsproto::MAX_REPLY {
                    return Err(Error::BufferTooSmall);
                }
            }
            op::SNAP_READ => {
                let snap = r.str()?;
                let path = r.str()?;
                let off = r.u64()?;
                let n = io_len(&mut r)?;
                w.bytes(&fs.snapshot_read(&snap, &path, off, n)?);
            }
            hop::SNAP_STAT_OF => {
                let ds = sess.map(&r.str()?, false)?;
                let snap = r.str()?;
                stat(fs.snapshot_stat_of(&ds, &snap, &r.str()?)?).put(&mut w);
            }
            hop::SNAP_READ_OF => {
                let ds = sess.map(&r.str()?, false)?;
                let snap = r.str()?;
                let path = r.str()?;
                let off = r.u64()?;
                let n = io_len(&mut r)?;
                w.bytes(&fs.snapshot_read_of(&ds, &snap, &path, off, n)?);
            }
            op::DEFRAG => {
                w.u64(fs.defrag_file(&r.str()?)?);
            }
            op::FRAGMENTS => {
                w.u64(fs.fragments(&sess.map(&r.str()?, false)?)?);
            }
            op::CHECK => {
                let report = fs.check(r.u8()? != 0)?;
                w.u64(report.generation)
                    .u64(report.problems_total)
                    .u8(report.live_not_checked as u8);
                // Jen tolik popisu, kolik se vejde do odpovedi.
                let mut fit = Vec::new();
                let mut size = 0;
                for p in &report.problems {
                    size += 2 + p.len();
                    if size > fsproto::MAX_REPLY - 64 {
                        break;
                    }
                    fit.push(p);
                }
                w.u32(fit.len() as u32);
                for p in fit {
                    w.str(p);
                }
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(Reply::body(w))
    }
}
