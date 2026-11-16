//! `fileops::Vfs` nad relaci FS uzivatele.

use alloc::string::String;
use alloc::vec::Vec;

use fileops::{Kind, Meta, Vfs};
use fsproto::{FsClient, MAX_IO, avail, kind};
use libos::{Error, Result};

pub struct FsVfs<'a>(pub &'a FsClient);

fn kind_of(k: u8) -> Kind {
    match k {
        kind::DIR | kind::DATASET => Kind::Dir,
        kind::SYMLINK => Kind::Link,
        _ => Kind::File,
    }
}

impl Vfs for FsVfs<'_> {
    fn stat(&mut self, path: &str) -> Result<Meta> {
        let s = self.0.stat(path)?;
        let k = kind_of(s.kind);
        let tree = (s.known & avail::TREE != 0).then_some(s.tree_bytes);
        Ok(Meta { kind: k, size: s.size, tree_bytes: if k == Kind::Dir { tree } else { None } })
    }

    fn list(&mut self, path: &str) -> Result<Vec<(String, Kind)>> {
        Ok(self.0.list(path)?.into_iter().map(|e| (e.name, kind_of(e.kind))).collect())
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        self.0.create(path, kind::DIR)
    }

    fn create_file(&mut self, path: &str) -> Result<()> {
        self.0.create(path, kind::FILE)
    }

    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            let part = self.0.read(path, offset + out.len() as u64, (len - out.len()).min(MAX_IO))?;
            if part.is_empty() {
                break;
            }
            out.extend_from_slice(&part);
        }
        Ok(out)
    }

    fn write(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<()> {
        for (i, part) in data.chunks(MAX_IO).enumerate() {
            self.0.write(path, offset + (i * MAX_IO) as u64, part)?;
        }
        Ok(())
    }

    fn clone_path(&mut self, src: &str, dst: &str) -> Result<()> {
        let r = self.0.clone_path(src, dst).map(|_| ());
        // Kopie ctenim a zapisem (cil na vymennem mediu: FAT, exFAT, NTFS
        // rozsirene atributy nemaji): puvod stazeneho souboru se ztrati.
        if r == Err(Error::NotSupported) && self.0.get_xattr(src, fsproto::perm::ORIGIN_XATTR).is_ok() {
            libos::println!("files-copy: {src}: puvod souboru (sys.origin) se do {dst} neprenese, cil atributy nema");
        }
        r
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.0.rename(from, to)
    }

    /// Do kose; na vymennem mediu kos neni (`NOT_SUPPORTED`): trvale
    /// smazat (zdroj presunu z media, prepsany cil na mediu).
    fn trash(&mut self, path: &str) -> Result<()> {
        match self.0.trash(path) {
            Err(Error::NotSupported) => self.remove_tree(path),
            r => r.map(|_| ()),
        }
    }

    fn remove_tree(&mut self, path: &str) -> Result<()> {
        let st = self.0.stat(path)?;
        if matches!(st.kind, kind::DIR) {
            for e in self.0.list(path)? {
                self.remove_tree(&fileops::path::join(path, &e.name))?;
            }
        } else if st.kind == kind::DATASET {
            // Docasne jmeno uloh nikdy neni dataset.
            return Err(Error::WrongType);
        }
        self.0.remove(path)
    }

    fn dataset_of(&mut self, path: &str) -> Result<String> {
        self.0.dataset_of(path)
    }

    /// Nic: FS uklada zmeny atomicky po commitech v poradi, v jakem
    /// prisly, takze ulozeny stav ulohy nikdy nepredbehne data (po
    /// vypadku je stav i data z tehoz commitu).
    fn sync(&mut self) -> Result<()> {
        Ok(())
    }
}
