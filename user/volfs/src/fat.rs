//! FAT12/16/32 s dlouhymi jmeny (crate `fatfs`, MIT): cteni i zapis.
//!
//! Prava FAT nema: vse patri uzivateli, ktery medium pripojil (fsd).
//! Atribut "jen pro cteni" se hlasi ve `Stat::read_only`. Jmena se
//! porovnavaji bez ohledu na velikost pismen (jako Windows). Po kazde
//! menici operaci jdou zmeny na medium ([`crate::dev::Cached::flush_dirty`]).

use std::io::{Read, Seek, SeekFrom, Write};

use fatfs::{Dir, DirEntry, FileSystem, FsOptions};
use volproto::{Error, Info, Result, Stat, VERSION, kind};

use crate::dev::{Cached, Sectors};
use crate::time::{CLOCK, from_fat};
use crate::{Volume, io_err};

pub struct FatVolume<D: Sectors> {
    fs: Option<FileSystem<Cached<D>>>,
    dev: Cached<D>,
    read_only: bool,
}

/// Cesta bez uvodniho `/` (fatfs: relativne ke koreni).
fn rel(path: &str) -> &str {
    path.trim_start_matches('/')
}

/// (rodic, jmeno) normalizovane cesty; koren = `None`.
fn split(path: &str) -> Option<(&str, &str)> {
    let p = rel(path);
    if p.is_empty() {
        return None;
    }
    Some(match p.rfind('/') {
        Some(i) => (&p[..i], &p[i + 1..]),
        None => ("", p),
    })
}

fn stat_of<D: Sectors>(e: &DirEntry<Cached<D>>, read_only: bool) -> Stat {
    Stat {
        kind: if e.is_dir() { kind::DIR } else { kind::FILE },
        size: if e.is_dir() { 0 } else { e.len() },
        mtime: from_fat(e.modified()),
        btime: from_fat(e.created()),
        read_only: read_only || e.attributes().contains(fatfs::FileAttributes::READ_ONLY),
    }
}

impl<D: Sectors> FatVolume<D> {
    pub fn new(dev: D, read_only: bool) -> Result<Self> {
        let dev = Cached::new(dev, read_only);
        let fs = FileSystem::new(dev.clone(), FsOptions::new().time_provider(&CLOCK)).map_err(io_err)?;
        Ok(Self { fs: Some(fs), dev, read_only })
    }

    fn fs(&self) -> Result<&FileSystem<Cached<D>>> {
        self.fs.as_ref().ok_or(Error::BadState)
    }

    /// Slozka podle cesty (`/` = koren).
    fn dir(&self, path: &str) -> Result<Dir<'_, Cached<D>>> {
        let root = self.fs()?.root_dir();
        let p = rel(path);
        if p.is_empty() { Ok(root) } else { root.open_dir(p).map_err(io_err) }
    }

    /// Polozka cesty (ne koren).
    fn entry(&self, path: &str) -> Result<DirEntry<'_, Cached<D>>> {
        let (parent, name) = split(path).ok_or(Error::InvalidArgs)?;
        let dir = self.dir(parent)?;
        for e in dir.iter() {
            let e = e.map_err(io_err)?;
            if e.file_name().eq_ignore_ascii_case(name) || e.file_name().to_lowercase() == name.to_lowercase() {
                return Ok(e);
            }
        }
        Err(Error::NotFound)
    }

    fn writable(&self) -> Result<()> {
        if self.read_only { Err(Error::ReadOnly) } else { Ok(()) }
    }

    /// Po menici operaci: na medium; po chybe zahodit rozpracovane zmeny
    /// (FS na mediu zustane ve stavu pred operaci, pokud ji crate nezapsal
    /// do pulky - pak opravi chkdsk).
    fn commit<T>(&self, r: Result<T>) -> Result<T> {
        match r {
            Ok(v) => {
                self.dev.flush_dirty().map_err(io_err)?;
                Ok(v)
            }
            Err(e) => {
                self.dev.discard_dirty();
                Err(e)
            }
        }
    }

    fn write_inner(&self, path: &str, off: u64, data: &[u8]) -> Result<usize> {
        let e = self.entry(path)?;
        if e.is_dir() {
            return Err(Error::IsADirectory);
        }
        let mut f = e.to_file();
        let size = e.len();
        if off > size {
            // Dira: doplnit nulami od konce.
            f.seek(SeekFrom::Start(size)).map_err(io_err)?;
            let zeros = vec![0u8; 4096];
            let mut left = off - size;
            while left > 0 {
                let n = left.min(zeros.len() as u64) as usize;
                f.write_all(&zeros[..n]).map_err(io_err)?;
                left -= n as u64;
            }
        } else {
            f.seek(SeekFrom::Start(off)).map_err(io_err)?;
        }
        f.write_all(data).map_err(io_err)?;
        f.flush().map_err(io_err)?;
        Ok(data.len())
    }

    fn set_len_inner(&self, path: &str, len: u64) -> Result<()> {
        let e = self.entry(path)?;
        if e.is_dir() {
            return Err(Error::IsADirectory);
        }
        let size = e.len();
        let mut f = e.to_file();
        if len < size {
            f.seek(SeekFrom::Start(len)).map_err(io_err)?;
            f.truncate().map_err(io_err)?;
        } else if len > size {
            drop(f);
            self.write_inner(path, len, &[])?;
        }
        Ok(())
    }

    fn rename_inner(&self, from: &str, to: &str, replace: bool) -> Result<()> {
        if split(from).is_none() || split(to).is_none() {
            return Err(Error::InvalidArgs);
        }
        let root = self.fs()?.root_dir();
        match self.entry(to) {
            Ok(dst) => {
                // Prejmenovani na sebe (jina velikost pismen) projde.
                let same = from.to_lowercase() == to.to_lowercase();
                if !same {
                    if !replace {
                        return Err(Error::AlreadyExists);
                    }
                    if dst.is_dir() {
                        return Err(Error::IsADirectory);
                    }
                    root.remove(rel(to)).map_err(io_err)?;
                }
            }
            Err(Error::NotFound) => {}
            Err(e) => return Err(e),
        }
        root.rename(rel(from), &root, rel(to)).map_err(io_err)
    }
}

impl<D: Sectors> Volume for FatVolume<D> {
    fn info(&mut self) -> Result<Info> {
        let fs = self.fs()?;
        let st = fs.stats().map_err(io_err)?;
        let cs = st.cluster_size() as u64;
        Ok(Info {
            version: VERSION,
            fs: "fat".into(),
            label: fs.volume_label().trim().to_string(),
            total_bytes: st.total_clusters() as u64 * cs,
            free_bytes: st.free_clusters() as u64 * cs,
            read_only: self.read_only,
        })
    }

    fn stat(&mut self, path: &str) -> Result<Stat> {
        if split(path).is_none() {
            return Ok(Stat { kind: kind::DIR, read_only: self.read_only, ..Stat::default() });
        }
        Ok(stat_of(&self.entry(path)?, self.read_only))
    }

    fn list(&mut self, path: &str) -> Result<Vec<(String, Stat)>> {
        let dir = self.dir(path)?;
        let mut out = Vec::new();
        for e in dir.iter() {
            let e = e.map_err(io_err)?;
            let name = e.file_name();
            if name == "." || name == ".." || e.attributes().contains(fatfs::FileAttributes::VOLUME_ID) {
                continue;
            }
            out.push((name, stat_of(&e, self.read_only)));
        }
        Ok(out)
    }

    fn read(&mut self, path: &str, off: u64, len: usize) -> Result<Vec<u8>> {
        let e = self.entry(path)?;
        if e.is_dir() {
            return Err(Error::IsADirectory);
        }
        let size = e.len();
        if off >= size {
            return Ok(Vec::new());
        }
        let mut f = e.to_file();
        f.seek(SeekFrom::Start(off)).map_err(io_err)?;
        let want = len.min((size - off) as usize);
        let mut buf = vec![0u8; want];
        let mut done = 0;
        while done < want {
            let n = f.read(&mut buf[done..]).map_err(io_err)?;
            if n == 0 {
                break;
            }
            done += n;
        }
        buf.truncate(done);
        Ok(buf)
    }

    fn write(&mut self, path: &str, off: u64, data: &[u8]) -> Result<usize> {
        self.writable()?;
        let r = self.write_inner(path, off, data);
        self.commit(r)
    }

    fn create(&mut self, path: &str, k: u8, exclusive: bool) -> Result<()> {
        self.writable()?;
        if split(path).is_none() {
            return Err(Error::AlreadyExists);
        }
        let r = match self.entry(path) {
            Ok(e) => {
                if exclusive || (k == kind::DIR) != e.is_dir() {
                    Err(Error::AlreadyExists)
                } else {
                    Ok(())
                }
            }
            Err(Error::NotFound) => {
                let root = self.fs()?.root_dir();
                if k == kind::DIR {
                    root.create_dir(rel(path)).map(|_| ()).map_err(io_err)
                } else {
                    root.create_file(rel(path)).map(|_| ()).map_err(io_err)
                }
            }
            Err(e) => Err(e),
        };
        self.commit(r)
    }

    fn remove(&mut self, path: &str) -> Result<()> {
        self.writable()?;
        if split(path).is_none() {
            return Err(Error::AccessDenied);
        }
        let r = self.fs()?.root_dir().remove(rel(path)).map_err(io_err);
        self.commit(r)
    }

    fn rename(&mut self, from: &str, to: &str, replace: bool) -> Result<()> {
        self.writable()?;
        let r = self.rename_inner(from, to, replace);
        self.commit(r)
    }

    fn set_len(&mut self, path: &str, len: u64) -> Result<()> {
        self.writable()?;
        let r = self.set_len_inner(path, len);
        self.commit(r)
    }

    fn sync(&mut self) -> Result<()> {
        self.dev.flush_dirty().map_err(io_err)
    }

    fn unmount(&mut self) -> Result<()> {
        // `unmount` zapise FSInfo a znacku cisteho svazku.
        if let Some(fs) = self.fs.take()
            && !self.read_only
        {
            fs.unmount().map_err(io_err)?;
        }
        self.dev.flush_dirty().map_err(io_err)
    }
}

impl<D: Sectors> FatVolume<D> {
    /// Medium (testy: obsah obrazu po operacich).
    pub fn device(&self) -> &Cached<D> {
        &self.dev
    }
}
