//! Obsluha pozadavku protokolu `fsproto`.

use alloc::vec::Vec;

use fs::{Fs, Kind};
use fsproto::{DirEntry, Info, Reader, Stat, Writer, kind, op};
use libos::{Error, Result};

use crate::blkdev::BlkDev;

fn stat(s: fs::Stat) -> Stat {
    Stat {
        oid: s.oid,
        kind: match s.kind {
            Kind::File => kind::FILE,
            Kind::Dir => kind::DIR,
            Kind::Symlink => kind::SYMLINK,
        },
        size: s.size,
        mtime: s.mtime,
        ctime: s.ctime,
        tree_bytes: s.tree_bytes,
        tree_files: s.tree_files,
        tree_dirs: s.tree_dirs,
    }
}

fn entries(list: Vec<fs::DirEntry>) -> Vec<DirEntry> {
    list.into_iter()
        .map(|e| DirEntry {
            name: e.name,
            kind: match e.kind {
                Kind::File => kind::FILE,
                Kind::Dir => kind::DIR,
                Kind::Symlink => kind::SYMLINK,
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
}

impl Service {
    fn fs(&mut self) -> Result<&mut Fs<BlkDev>> {
        self.fs.as_mut().ok_or(Error::IoError)
    }

    /// Zpracuje jeden pozadavek; vraci telo odpovedi.
    pub fn handle(&mut self, msg: &[u8]) -> Result<Writer> {
        let mut r = Reader::new(msg);
        let opcode = r.u32()?;
        let mut w = Writer::new();
        if opcode == op::SIMULATE_POWER_LOSS {
            if !self.debug {
                return Err(Error::AccessDenied);
            }
            let dev = self.fs.take().ok_or(Error::IoError)?.into_device();
            let fs = Fs::mount(dev)?;
            w.u64(fs.info().replayed);
            self.fs = Some(fs);
            return Ok(w);
        }
        let now = libos::time::now();
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
            op::LIST => fsproto::encode_list(&mut w, &entries(fs.list(&r.str()?)?)),
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
                w.u32(report.problems.len() as u32);
                for p in &report.problems {
                    w.str(p);
                }
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(w)
    }
}
