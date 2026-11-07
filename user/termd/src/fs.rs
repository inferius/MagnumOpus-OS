//! Souborovy system shellu nad sluzbou fsd (`fsproto`).

use alloc::vec::Vec;

use fsproto::{FsClient, Stat, kind};
use libos::Error;
use shlang::host::{DirEntry, Fs, FsResult, Meta, NodeKind};

pub struct FsdFs {
    pub client: FsClient,
}

fn node(k: u8) -> NodeKind {
    match k {
        kind::DIR | kind::DATASET => NodeKind::Dir,
        kind::SYMLINK => NodeKind::Symlink,
        _ => NodeKind::File,
    }
}

fn meta(s: &Stat) -> Meta {
    let t = s.mtime;
    Meta {
        kind: node(s.kind),
        size: s.size,
        mtime: Some(t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64)),
    }
}

impl FsdFs {
    fn entries(&self, list: Vec<fsproto::DirEntry>, dir: &str, snap: Option<&str>) -> Vec<DirEntry> {
        list.into_iter()
            .map(|e| {
                let full = shlang::vfs::path::join(dir, &e.name);
                let st = match snap {
                    Some(s) => self.client.snapshot_stat(s, &full),
                    None => self.client.stat(&full),
                };
                let meta = st.as_ref().map(meta).unwrap_or(Meta { kind: node(e.kind), size: 0, mtime: None });
                DirEntry { name: e.name, meta }
            })
            .collect()
    }
}

impl Fs for FsdFs {
    fn stat(&mut self, path: &str) -> FsResult<Meta> {
        self.client.stat(path).map(|s| meta(&s))
    }

    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        let l = self.client.list(path)?;
        Ok(self.entries(l, path, None))
    }

    fn read(&mut self, path: &str) -> FsResult<Vec<u8>> {
        let st = self.client.stat(path)?;
        if node(st.kind) == NodeKind::Dir {
            return Err(Error::IsADirectory);
        }
        self.client.read(path, 0, st.size as usize)
    }

    /// Zapis je atomicky: data do docasneho souboru vedle, pak
    /// `RENAME_REPLACE`. Pri chybe zustane puvodni soubor cely.
    fn write(&mut self, path: &str, data: &[u8]) -> FsResult<()> {
        match self.client.stat(path) {
            Ok(s) if node(s.kind) == NodeKind::Dir => return Err(Error::IsADirectory),
            Ok(_) => {}
            Err(Error::NotFound) => {
                self.client.create(path, kind::FILE)?;
                return self.client.write(path, 0, data);
            }
            Err(e) => return Err(e),
        }
        let tmp = alloc::format!("{path}.~termd");
        let _ = self.client.remove(&tmp);
        self.client.create(&tmp, kind::FILE)?;
        let r = self.client.write(&tmp, 0, data).and_then(|_| self.client.rename_replace(&tmp, path));
        if r.is_err() {
            let _ = self.client.remove(&tmp);
        }
        r
    }

    fn mkdir(&mut self, path: &str) -> FsResult<()> {
        self.client.create(path, kind::DIR)
    }

    fn remove(&mut self, path: &str) -> FsResult<()> {
        self.client.remove(path)
    }

    fn rename(&mut self, from: &str, to: &str) -> FsResult<()> {
        self.client.rename(from, to)
    }

    fn space(&mut self) -> FsResult<(u64, u64)> {
        let i = self.client.info()?;
        let bs = i.block_size as u64;
        Ok((i.block_count * bs, i.free_blocks * bs))
    }

    fn snapshot(&mut self, name: &str) -> FsResult<()> {
        self.client.snapshot(name).map(|_| ())
    }

    fn delete_snapshot(&mut self, name: &str) -> FsResult<()> {
        self.client.delete_snapshot(name)
    }

    fn snapshot_stat(&mut self, snap: &str, path: &str) -> FsResult<Meta> {
        self.client.snapshot_stat(snap, path).map(|s| meta(&s))
    }

    fn snapshot_list(&mut self, snap: &str, path: &str) -> FsResult<Vec<DirEntry>> {
        let l = self.client.snapshot_list(snap, path)?;
        Ok(self.entries(l, path, Some(snap)))
    }

    fn snapshot_read(&mut self, snap: &str, path: &str) -> FsResult<Vec<u8>> {
        let st = self.client.snapshot_stat(snap, path)?;
        self.client.snapshot_read(snap, path, 0, st.size as usize)
    }
}

/// FS, kdyz sluzba fsd neni (zadny disk): vse `NOT_FOUND`, jen koren.
pub struct NoFs;

impl Fs for NoFs {
    fn stat(&mut self, path: &str) -> FsResult<Meta> {
        if path == "/" {
            return Ok(Meta { kind: NodeKind::Dir, size: 0, mtime: None });
        }
        Err(Error::NotFound)
    }
    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        if path == "/" { Ok(Vec::new()) } else { Err(Error::NotFound) }
    }
    fn read(&mut self, _: &str) -> FsResult<Vec<u8>> {
        Err(Error::NotFound)
    }
    fn write(&mut self, _: &str, _: &[u8]) -> FsResult<()> {
        Err(Error::ReadOnly)
    }
    fn mkdir(&mut self, _: &str) -> FsResult<()> {
        Err(Error::ReadOnly)
    }
    fn remove(&mut self, _: &str) -> FsResult<()> {
        Err(Error::ReadOnly)
    }
    fn rename(&mut self, _: &str, _: &str) -> FsResult<()> {
        Err(Error::ReadOnly)
    }
}
