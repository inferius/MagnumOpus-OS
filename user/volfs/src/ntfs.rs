//! NTFS jen pro cteni (crate `ntfs`, MIT OR Apache-2.0).
//!
//! ACL NTFS se ignoruji (vse vidi uzivatel, ktery medium pripojil);
//! skryte systemove soubory (`$MFT`, `$Bitmap`, ...) ve vypisu nejsou,
//! jmena DOS 8.3 taky ne (jen dlouha jmena Win32/POSIX). Alternativni
//! datove proudy, komprimovane a sifrovane soubory: cteni proudu `$DATA`
//! tak, jak ho crate umi (komprimovany soubor = chyba I/O).

use std::io::SeekFrom;

use ntfs::indexes::NtfsFileNameIndex;
use ntfs::structured_values::{NtfsFileName, NtfsFileNamespace};
use ntfs::{Ntfs, NtfsFile, NtfsReadSeek};
use volproto::{Error, Info, Result, Stat, VERSION, kind};

use crate::Volume;
use crate::dev::{Cached, Sectors};
use crate::time::from_ntfs;

pub struct NtfsVolume<D: Sectors> {
    ntfs: Ntfs,
    dev: Cached<D>,
}

fn ntfs_err(e: ntfs::NtfsError) -> Error {
    match e {
        ntfs::NtfsError::Io(e) => crate::io_err(e),
        _ => Error::IoError,
    }
}

fn stat_of(f: &NtfsFile) -> Result<Stat> {
    let info = f.info().map_err(ntfs_err)?;
    let dir = f.is_directory();
    Ok(Stat {
        kind: if dir { kind::DIR } else { kind::FILE },
        size: 0,
        mtime: from_ntfs(info.modification_time().nt_timestamp()),
        btime: from_ntfs(info.creation_time().nt_timestamp()),
        read_only: true,
    })
}

impl<D: Sectors> NtfsVolume<D> {
    pub fn new(dev: D) -> Result<Self> {
        let mut dev = Cached::new(dev, true);
        let mut ntfs = Ntfs::new(&mut dev).map_err(ntfs_err)?;
        ntfs.read_upcase_table(&mut dev).map_err(ntfs_err)?;
        Ok(Self { ntfs, dev })
    }

    /// Soubor nebo slozka podle cesty (jmena bez ohledu na velikost pismen,
    /// podle tabulky `$UpCase`).
    fn lookup<'n>(ntfs: &'n Ntfs, dev: &mut Cached<D>, path: &str) -> Result<NtfsFile<'n>> {
        let mut cur = ntfs.root_directory(dev).map_err(ntfs_err)?;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            if !cur.is_directory() {
                return Err(Error::NotADirectory);
            }
            let index = cur.directory_index(dev).map_err(ntfs_err)?;
            let mut finder = index.finder();
            let entry = NtfsFileNameIndex::find(&mut finder, ntfs, dev, part).ok_or(Error::NotFound)?.map_err(ntfs_err)?;
            cur = entry.to_file(ntfs, dev).map_err(ntfs_err)?;
        }
        Ok(cur)
    }

    /// Velikost datoveho proudu souboru.
    fn data_len(&mut self, path: &str) -> Result<u64> {
        let mut dev = self.dev.clone();
        let f = Self::lookup(&self.ntfs, &mut dev, path)?;
        if f.is_directory() {
            return Ok(0);
        }
        let item = match f.data(&mut dev, "") {
            Some(i) => i.map_err(ntfs_err)?,
            None => return Ok(0),
        };
        let attr = item.to_attribute().map_err(ntfs_err)?;
        Ok(attr.value(&mut dev).map_err(ntfs_err)?.len())
    }
}

impl<D: Sectors> Volume for NtfsVolume<D> {
    fn info(&mut self) -> Result<Info> {
        let label = match self.ntfs.volume_name(&mut self.dev) {
            Some(Ok(n)) => n.name().to_string_lossy(),
            _ => String::new(),
        };
        Ok(Info { version: VERSION, fs: "ntfs".into(), label, total_bytes: self.ntfs.size(), free_bytes: 0, read_only: true })
    }

    fn stat(&mut self, path: &str) -> Result<Stat> {
        let mut st = {
            let mut dev = self.dev.clone();
            let f = Self::lookup(&self.ntfs, &mut dev, path)?;
            stat_of(&f)?
        };
        if st.kind == kind::FILE {
            st.size = self.data_len(path)?;
        }
        Ok(st)
    }

    fn list(&mut self, path: &str) -> Result<Vec<(String, Stat)>> {
        let mut dev = self.dev.clone();
        let dir = Self::lookup(&self.ntfs, &mut dev, path)?;
        if !dir.is_directory() {
            return Err(Error::NotADirectory);
        }
        let index = dir.directory_index(&mut dev).map_err(ntfs_err)?;
        let mut iter = index.entries();
        let mut out = Vec::new();
        while let Some(entry) = iter.next(&mut dev) {
            let entry = entry.map_err(ntfs_err)?;
            let Some(key) = entry.key() else { continue };
            let name: NtfsFileName = key.map_err(ntfs_err)?;
            if name.namespace() == NtfsFileNamespace::Dos {
                continue;
            }
            let n = name.name().to_string_lossy();
            // Systemove soubory NTFS ($MFT, ...) v koreni.
            if n.starts_with('$') || n == "." {
                continue;
            }
            out.push((
                n,
                Stat {
                    kind: if name.is_directory() { kind::DIR } else { kind::FILE },
                    size: if name.is_directory() { 0 } else { name.data_size() },
                    mtime: from_ntfs(name.modification_time().nt_timestamp()),
                    btime: from_ntfs(name.creation_time().nt_timestamp()),
                    read_only: true,
                },
            ));
        }
        Ok(out)
    }

    fn read(&mut self, path: &str, off: u64, len: usize) -> Result<Vec<u8>> {
        let mut dev = self.dev.clone();
        let f = Self::lookup(&self.ntfs, &mut dev, path)?;
        if f.is_directory() {
            return Err(Error::IsADirectory);
        }
        let item = match f.data(&mut dev, "") {
            Some(i) => i.map_err(ntfs_err)?,
            None => return Ok(Vec::new()),
        };
        let attr = item.to_attribute().map_err(ntfs_err)?;
        let mut value = attr.value(&mut dev).map_err(ntfs_err)?;
        let size = value.len();
        if off >= size {
            return Ok(Vec::new());
        }
        value.seek(&mut dev, SeekFrom::Start(off)).map_err(ntfs_err)?;
        let want = len.min((size - off) as usize);
        let mut buf = vec![0u8; want];
        let mut done = 0;
        while done < want {
            let n = value.read(&mut dev, &mut buf[done..]).map_err(ntfs_err)?;
            if n == 0 {
                break;
            }
            done += n;
        }
        buf.truncate(done);
        Ok(buf)
    }

    fn write(&mut self, _: &str, _: u64, _: &[u8]) -> Result<usize> {
        Err(Error::ReadOnly)
    }
    fn create(&mut self, _: &str, _: u8, _: bool) -> Result<()> {
        Err(Error::ReadOnly)
    }
    fn remove(&mut self, _: &str) -> Result<()> {
        Err(Error::ReadOnly)
    }
    fn rename(&mut self, _: &str, _: &str, _: bool) -> Result<()> {
        Err(Error::ReadOnly)
    }
    fn set_len(&mut self, _: &str, _: u64) -> Result<()> {
        Err(Error::ReadOnly)
    }
    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
    fn unmount(&mut self) -> Result<()> {
        Ok(())
    }
}
