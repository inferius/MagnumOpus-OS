//! Obsluha pozadavku protokolu `fsproto`.

use alloc::vec::Vec;

use fs::{Fs, Kind};
use fsproto::{DirEntry, Info, Reader, Stat, Writer, kind, op};
use libos::{Error, Result};

use crate::blkdev::BlkDev;
use crate::serve_v2::{Reply, Session};

/// Cas pro zmeny ve FS: realny cas v ns od 1970 (dnesni format casu ve FS
/// je u64 ns, takze cas pred 1970 se orizne na 0). Bez realnych hodin cas od
/// startu systemu (data pak vypadaji jako z roku 1970).
pub(crate) fn fs_now() -> u64 {
    match libos::time::wall() {
        Some(t) => t.as_nanos().clamp(0, u64::MAX as i128) as u64,
        None => libos::time::now(),
    }
}

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
        tree_bytes: s.tree_bytes,
        tree_files: s.tree_files,
        tree_dirs: s.tree_dirs,
    }
}

pub(crate) fn entries(list: Vec<fs::DirEntry>) -> Vec<DirEntry> {
    list.into_iter()
        .map(|e| DirEntry {
            name: e.name,
            kind: match e.kind {
                Kind::File => kind::FILE,
                Kind::Dir => kind::DIR,
                Kind::Symlink => kind::SYMLINK,
            Kind::Dataset => kind::DATASET,
            },
            oid: e.oid,
        })
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
}

/// Klic a obsah rozpracovaneho vypisu (polozky serazene podle slozeneho
/// jmena - kurzor se hleda pulenim, slozeny tvar se neuklada).
pub type ListCache = ((Option<alloc::string::String>, alloc::string::String), Vec<fs::DirEntry>);

impl Service {
    fn fs(&mut self) -> Result<&mut Fs<BlkDev>> {
        self.fs.as_mut().ok_or(Error::IoError)
    }

    /// Zpracuje jeden pozadavek relace `sess`; vraci odpoved.
    pub fn handle(&mut self, sess: &mut Session, msg: &[u8]) -> Result<Reply> {
        let mut r = Reader::new(msg);
        let opcode = r.u32()?;
        // Rozpracovany vypis zahodi kazda jina operace (nikdy neni zastaraly).
        if opcode != op::LIST_PAGE && opcode != op::SNAP_LIST_PAGE {
            sess.list_cache = None;
        }
        if opcode == op::COMMIT {
            self.commit_requested = true;
        }
        if let Some(res) = self.handle_v2(sess, opcode, &mut r) {
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
            op::STAT => stat(fs.stat(&r.str()?)?).put(&mut w),
            op::LIST => {
                let list = entries(fs.list(&r.str()?)?);
                fsproto::encode_list(&mut w, &list);
                if w.len() > fsproto::MAX_REPLY {
                    return Err(Error::BufferTooSmall);
                }
            }
            op::LIST_PAGE | op::SNAP_LIST_PAGE => {
                let snap = if opcode == op::SNAP_LIST_PAGE { Some(r.str()?) } else { None };
                let path = r.str()?;
                let after = r.str()?;
                let max = r.u32()? as usize;
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
                            Some(s) => fs.snapshot_list(s, &path)?,
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
                let page: Vec<DirEntry> = entries(list[start..].to_vec());
                let n = fsproto::encode_list_page(&mut w, &page, max, fsproto::MAX_REPLY);
                if start + n < list.len() {
                    sess.list_cache = Some((key, list));
                }
                return Ok(Reply::body(w));
            }
            op::READ => {
                let path = r.str()?;
                let off = r.u64()?;
                let n = io_len(&mut r)?;
                w.bytes(&fs.read(&path, off, n)?);
            }
            op::WRITE => {
                let path = r.str()?;
                let off = r.u64()?;
                fs.write(&path, off, r.rest())?;
            }
            op::CREATE => {
                let path = r.str()?;
                let k = match r.u8()? {
                    kind::FILE => Kind::File,
                    kind::DIR => Kind::Dir,
                    _ => return Err(Error::InvalidArgs),
                };
                fs.create(&path, k)?;
            }
            op::REMOVE => fs.remove(&r.str()?)?,
            op::RENAME => {
                let from = r.str()?;
                fs.rename(&from, &r.str()?)?;
            }
            op::TRUNCATE => {
                let path = r.str()?;
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
            op::DEFRAG => {
                w.u64(fs.defrag_file(&r.str()?)?);
            }
            op::FRAGMENTS => {
                w.u64(fs.fragments(&r.str()?)?);
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
