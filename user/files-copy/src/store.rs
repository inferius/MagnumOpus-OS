//! Ulozeny stav uloh: `<domov>/apps/org.magnumopus.files/ulohy/<id>.uloha`.
//! Zapis pres docasny soubor a `RENAME_REPLACE` (soubor stavu nikdy neni
//! polovicni). Po startu relace se nedokoncene ulohy nactou a pokracuji.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use fileops::job::Job;
use fileops::path::join;
use fsproto::{FsClient, kind};
use libos::{Error, Result};

const SUFFIX: &str = ".uloha";

pub struct Store {
    pub dir: String,
}

impl Store {
    pub fn new(home: &str) -> Self {
        Self { dir: format!("{}/apps/org.magnumopus.files/ulohy", home.trim_end_matches('/')) }
    }

    /// Vytvori slozku stavu (i rodice; existujici se jen projdou - relace
    /// uzivatele nesmi tvorit v `/` ani `/home`).
    pub fn ensure(&self, fs: &FsClient) -> Result<()> {
        let mut p = String::new();
        for part in self.dir.split('/').filter(|s| !s.is_empty()) {
            p.push('/');
            p.push_str(part);
            if fs.stat(&p).is_ok() {
                continue;
            }
            match fs.create(&p, kind::DIR) {
                Ok(()) | Err(Error::AlreadyExists) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn path(&self, id: u64) -> String {
        join(&self.dir, &format!("{id}{SUFFIX}"))
    }

    pub fn save(&self, fs: &FsClient, j: &Job) -> Result<()> {
        let data = j.encode();
        let tmp = join(&self.dir, &format!(".{}{SUFFIX}.nova", j.id));
        let _ = fs.remove(&tmp);
        fs.create(&tmp, kind::FILE)?;
        for (i, part) in data.chunks(fsproto::MAX_IO).enumerate() {
            fs.write(&tmp, (i * fsproto::MAX_IO) as u64, part)?;
        }
        fs.rename_replace(&tmp, &self.path(j.id))
    }

    pub fn remove(&self, fs: &FsClient, id: u64) {
        let _ = fs.remove(&self.path(id));
    }

    /// Ulozene ulohy; poskozeny soubor se prejmenuje na `.vadna` (neztrati
    /// se, ale neblokuje dalsi start).
    pub fn load(&self, fs: &FsClient) -> Vec<Job> {
        let mut out = Vec::new();
        let Ok(list) = fs.list(&self.dir) else { return out };
        for e in list {
            let Some(stem) = e.name.strip_suffix(SUFFIX) else { continue };
            if stem.parse::<u64>().is_err() {
                continue;
            }
            let p = join(&self.dir, &e.name);
            let data = fs.stat(&p).and_then(|s| read_all(fs, &p, s.size));
            match data.and_then(|d| Job::decode(&d)) {
                Ok(j) => out.push(j),
                Err(err) => {
                    libos::println!("files-copy: stav {} nejde precist ({err}), odkladam", e.name);
                    let _ = fs.rename(&p, &format!("{p}.vadna"));
                }
            }
        }
        out.sort_by_key(|j| j.id);
        out
    }
}

fn read_all(fs: &FsClient, p: &str, size: u64) -> Result<Vec<u8>> {
    if size > 4 << 20 {
        return Err(Error::InvalidArgs);
    }
    let mut out = Vec::new();
    while (out.len() as u64) < size {
        let part = fs.read(p, out.len() as u64, fsproto::MAX_IO)?;
        if part.is_empty() {
            break;
        }
        out.extend_from_slice(&part);
    }
    Ok(out)
}
