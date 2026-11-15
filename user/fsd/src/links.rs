//! Nasledovani symbolickych odkazu pri hledani cesty (docs/cs/souborovy-system.md,
//! kapitola Symbolicke odkazy).
//!
//! Cesta se resi ve **jmennem prostoru relace** (pripojene cesty), ne ve FS:
//! relativni cil od slozky odkazu, absolutni cil od korene relace. Kazdy
//! krok se znovu mapuje pres `Session::map`, takze odkaz nikdy nevede mimo
//! pripojene cesty relace (cil mimo = `NOT_FOUND`) - odkaz nic neprava,
//! prava se kontroluji na cili. `..` po nasledovani podle POSIX: od uz
//! vyresene (fyzicke) cesty. Nejvic `MAX_HOPS` odkazu = `TOO_MANY_LINKS`.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use fs::Fs;
use fsproto::v2::op;
use fsproto::{Reader, Writer};
use libos::{Error, Result};

use crate::blkdev::BlkDev;
use crate::serve::{Service, stat};
use crate::serve_v2::{Reply, Session};

/// Nejvic nasledovanych odkazu na jednu cestu (Linux `MAXSYMLINKS`).
pub const MAX_HOPS: usize = 40;

fn join(c: &[String]) -> String {
    let mut s = String::new();
    for x in c {
        s.push('/');
        s.push_str(x);
    }
    if s.is_empty() {
        s.push('/');
    }
    s
}

fn split(p: &str) -> impl Iterator<Item = String> + '_ {
    p.split('/').filter(|c| !c.is_empty()).map(String::from)
}

impl Session {
    /// Cesta relace `raw` -> cesta ve FS s nasledovanim odkazu (posledni
    /// slozka jen s `last`). `write` = cesta pro zapis (pripojeni jen pro
    /// cteni = `ACCESS_DENIED`, jako `map`).
    pub fn resolve(&self, fs: &mut Fs<BlkDev>, raw: &str, write: bool, last: bool) -> Result<String> {
        self.resolve_both(fs, raw, write, last).map(|(_, p)| p)
    }

    /// Jako [`Session::resolve`], navic kanonicka cesta relace.
    pub fn resolve_both(&self, fs: &mut Fs<BlkDev>, raw: &str, write: bool, last: bool) -> Result<(String, String)> {
        if !raw.starts_with('/') {
            return Err(Error::InvalidArgs);
        }
        let mut phys: Vec<String> = Vec::new();
        let mut pending: VecDeque<String> = split(raw).collect();
        let mut hops = 0;
        loop {
            // Beh obycejnych jmen az do `.`/`..` nebo konce.
            let mut cand = phys.clone();
            while let Some(c) = pending.front() {
                if c == "." || c == ".." {
                    break;
                }
                cand.push(pending.pop_front().unwrap_or_default());
            }
            if cand.len() > phys.len() {
                let follow = !pending.is_empty() || last;
                let fsp = self.map(&join(&cand), false)?;
                if let Some((at, target)) = fs.find_link(&fsp, follow)? {
                    // Index slozky ve FS -> v ceste relace (pripojena cesta
                    // muze mit jiny pocet slozek nez jeji jmeno v relaci).
                    let off = split(&fsp).count() as isize - cand.len() as isize;
                    let si = at as isize - off;
                    if si < 0 {
                        // Odkaz v cili pripojene cesty: mimo jmenny prostor relace.
                        return Err(Error::NotFound);
                    }
                    let si = si as usize;
                    hops += 1;
                    if hops > MAX_HOPS {
                        return Err(Error::TooManyLinks);
                    }
                    let mut next: VecDeque<String> = split(&target).collect();
                    next.extend(cand.drain(si + 1..));
                    next.extend(pending.drain(..));
                    pending = next;
                    cand.truncate(si);
                    phys = if target.starts_with('/') { Vec::new() } else { cand };
                    continue;
                }
                phys = cand;
            }
            match pending.pop_front() {
                None => {
                    let s = join(&phys);
                    let p = self.map(&s, write)?;
                    return Ok((s, p));
                }
                // `..` od fyzicke cesty; nad korenem relace zustane koren.
                Some(c) if c == ".." => {
                    // POSIX: slozka pred `..` musi existovat a byt adresar.
                    if !phys.is_empty() {
                        let st = fs.stat(&self.map(&join(&phys), false)?)?;
                        if !matches!(st.kind, fs::Kind::Dir | fs::Kind::Dataset) {
                            return Err(Error::NotADirectory);
                        }
                    }
                    phys.pop();
                }
                Some(_) => {}
            }
        }
    }
}

impl Service {
    /// Operace s odkazy; `None` = neni jeji.
    pub fn handle_links(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Option<Result<Reply>> {
        if !matches!(opcode, op::SYMLINK | op::READLINK | op::LSTAT | op::REALPATH) {
            return None;
        }
        Some((|| {
            let fs = self.fs.as_mut().ok_or(Error::IoError)?;
            fs.set_time(crate::serve::fs_now());
            let mut w = Writer::new();
            let raw = r.str()?;
            match opcode {
                op::SYMLINK => {
                    // Odkaz nic neprava: vytvoreni chce pravo vytvareni ve
                    // slozce (fs::symlink), cil se pri nasledovani mapuje
                    // znovu pres jmenny prostor relace.
                    let path = sess.resolve(fs, &raw, true, false)?;
                    fs.symlink(&path, &r.str()?)?;
                }
                op::REALPATH => {
                    let (canon, path) = sess.resolve_both(fs, &raw, false, true)?;
                    fs.stat(&path)?;
                    w.str(&canon);
                }
                op::READLINK => {
                    let path = sess.resolve(fs, &raw, false, false)?;
                    w.str(&fs.readlink(&path)?);
                }
                _ => {
                    let path = sess.resolve(fs, &raw, false, false)?;
                    stat(fs.stat(&path)?).put(&mut w);
                }
            }
            Ok(Reply::body(w))
        })())
    }
}
