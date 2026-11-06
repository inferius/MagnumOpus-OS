//! Operace protokolu v2: relace, handly, podminene ulozeni, zamky.
//!
//! Kazdy klient ma relaci (`Session`) s vlastnim kanalem a mnozinou handlu;
//! handle z jine relace = `BAD_HANDLE`. Zavreni relace zavre vsechny jeji
//! handly (docasne soubory podmineneho ulozeni zmizi - maji smazani pri
//! zavreni).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::v2::{Revision, lock, op, open_flags};
use fsproto::{Reader, Writer};
use libos::{Channel, Error, Handle, Result};

use crate::serve::{ListCache, Service, entries, stat};

/// Nejvic soucasnych relaci (ochrana sluzby pred zahlcenim).
pub const MAX_SESSIONS: usize = 64;

/// Rozpracovane podminene ulozeni (handle docasneho souboru).
pub struct Replace {
    target: String,
    expected: Option<fs::Revision>,
    temp: String,
}

/// Relace jednoho klienta.
pub struct Session {
    pub channel: Channel,
    pub handles: BTreeSet<u64>,
    pub list_cache: Option<ListCache>,
    pub replaces: BTreeMap<u64, Replace>,
}

impl Session {
    pub fn new(channel: Channel) -> Self {
        Self {
            channel,
            handles: BTreeSet::new(),
            list_cache: None,
            replaces: BTreeMap::new(),
        }
    }
}

/// Odpoved: telo, predavane handly a pripadne nova relace (`CONNECT`).
pub struct Reply {
    pub body: Writer,
    pub handles: Vec<Handle>,
    pub new_session: Option<Channel>,
}

impl Reply {
    pub fn body(body: Writer) -> Self {
        Self {
            body,
            handles: Vec::new(),
            new_session: None,
        }
    }
}

fn put_rev(w: &mut Writer, r: fs::Revision) {
    Revision {
        dataset: r.dataset,
        oid: r.oid,
        generation: r.generation,
        version: r.version,
    }
    .put(w);
}

fn get_rev(r: &mut Reader) -> Result<fs::Revision> {
    let x = Revision::get(r)?;
    Ok(fs::Revision {
        dataset: x.dataset,
        oid: x.oid,
        generation: x.generation,
        version: x.version,
    })
}

fn split(path: &str) -> Result<(&str, &str)> {
    let t = path.trim_end_matches('/');
    let i = t.rfind('/').ok_or(Error::InvalidArgs)?;
    Ok((if i == 0 { "/" } else { &t[..i] }, &t[i + 1..]))
}

impl Service {
    /// Zavre vsechny handly relace (pri jejim konci).
    pub fn close_session(&mut self, sess: &mut Session) {
        let Some(fs) = self.fs.as_mut() else { return };
        for h in core::mem::take(&mut sess.handles) {
            let _ = fs.close(h);
        }
        sess.replaces.clear();
    }

    fn own(sess: &Session, h: u64) -> Result<u64> {
        if sess.handles.contains(&h) { Ok(h) } else { Err(Error::BadHandle) }
    }

    /// Operace v2; `None` = neni v2 (obslouzi v1).
    pub fn handle_v2(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Option<Result<Reply>> {
        if !(op::HELLO..=op::REPLACE_ABORT).contains(&opcode) {
            return None;
        }
        Some(self.v2(sess, opcode, r))
    }

    fn v2(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Result<Reply> {
        let mut w = Writer::new();
        if opcode == op::HELLO {
            w.u32(fsproto::v2::VERSION).u32(fsproto::MAX_IO as u32);
            return Ok(Reply::body(w));
        }
        if opcode == op::CONNECT {
            let (ours, theirs) = Channel::create()?;
            return Ok(Reply {
                body: w,
                handles: alloc::vec![theirs.into_handle()],
                new_session: Some(ours),
            });
        }
        let now = crate::serve::fs_now();
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        fs.set_time(now);
        match opcode {
            op::OPEN => {
                let path = r.str()?;
                let (access, share, flags) = (r.u8()?, r.u8()?, r.u8()?);
                if flags & open_flags::CREATE != 0 {
                    let kind = if flags & open_flags::DIR != 0 { fs::Kind::Dir } else { fs::Kind::File };
                    match fs.create(&path, kind) {
                        Ok(()) => {}
                        Err(Error::AlreadyExists) if flags & open_flags::EXCL == 0 => {}
                        Err(e) => return Err(e),
                    }
                }
                let h = fs.open(&path, access, share)?;
                if flags & open_flags::TRUNCATE != 0
                    && let Err(e) = fs.set_len_h(h, 0)
                {
                    let _ = fs.close(h);
                    return Err(e);
                }
                sess.handles.insert(h);
                w.u64(h);
                stat(fs.stat_h(h)?).put(&mut w);
                put_rev(&mut w, fs.revision_h(h)?);
            }
            op::CLOSE => {
                let h = Self::own(sess, r.u64()?)?;
                sess.handles.remove(&h);
                sess.replaces.remove(&h);
                fs.close(h)?;
            }
            op::READ_H => {
                let h = Self::own(sess, r.u64()?)?;
                let off = r.u64()?;
                let n = (r.u32()? as usize).min(fsproto::MAX_IO);
                w.bytes(&fs.read_h(h, off, n)?);
            }
            op::WRITE_H => {
                let h = Self::own(sess, r.u64()?)?;
                let off = r.u64()?;
                let data = r.rest();
                // Prijato = cela zprava (knihovna prijme vse, nebo nic).
                fs.write_h(h, off, data)?;
                w.u32(data.len() as u32);
            }
            op::SET_LEN => {
                let h = Self::own(sess, r.u64()?)?;
                fs.set_len_h(h, r.u64()?)?;
            }
            op::STAT_H => {
                let h = Self::own(sess, r.u64()?)?;
                stat(fs.stat_h(h)?).put(&mut w);
                put_rev(&mut w, fs.revision_h(h)?);
            }
            op::LIST_H => {
                let h = Self::own(sess, r.u64()?)?;
                let after = r.str()?;
                let max = r.u32()? as usize;
                let list = fs.list_h(h)?;
                let start = if after.is_empty() {
                    0
                } else {
                    let a = fs::fold_name(&after);
                    list.partition_point(|e| fs::fold_name(&e.name) <= a)
                };
                let page = entries(list[start..].to_vec());
                fsproto::encode_list_page(&mut w, &page, max, fsproto::MAX_REPLY);
            }
            op::LOCK => {
                let h = Self::own(sess, r.u64()?)?;
                let (start, len, f) = (r.u64()?, r.u64()?, r.u8()?);
                fs.lock_range(h, start, len, f & lock::EXCLUSIVE != 0, f & lock::MANDATORY != 0)?;
            }
            op::UNLOCK => {
                let h = Self::own(sess, r.u64()?)?;
                fs.unlock_range(h, r.u64()?, r.u64()?)?;
            }
            op::DELETE_ON_CLOSE => {
                let h = Self::own(sess, r.u64()?)?;
                fs.set_delete_on_close(h, r.u8()? != 0)?;
            }
            op::RENAME_REPLACE => {
                let from = r.str()?;
                fs.rename_replace(&from, &r.str()?)?;
            }
            op::REPLACE_BEGIN => {
                let target = r.str()?;
                let has = r.u8()? != 0;
                let rev = get_rev(r)?;
                let expected = has.then_some(rev);
                let (dir, name) = split(&target)?;
                fs::validate_name(name)?;
                self.temp_counter += 1;
                let temp = alloc::format!("{}/{}", dir.trim_end_matches('/'), fsproto::v2::temp_name(name, self.temp_counter));
                let fs = self.fs.as_mut().ok_or(Error::IoError)?;
                fs.create(&temp, fs::Kind::File)?;
                let h = fs.open(&temp, fs::access::READ | fs::access::WRITE | fs::access::DELETE, fs::share::READ | fs::share::DELETE)?;
                // Kdyz klient zmizi, docasny soubor zmizi s nim.
                fs.set_delete_on_close(h, true)?;
                sess.handles.insert(h);
                sess.replaces.insert(h, Replace { target, expected, temp });
                w.u64(h);
            }
            op::REPLACE_COMMIT => {
                let h = Self::own(sess, r.u64()?)?;
                let force = r.u8()? != 0;
                let rep = sess.replaces.get(&h).ok_or(Error::BadHandle)?;
                let expected = if force {
                    match fs.revision(&rep.target) {
                        Ok(x) => Some(x),
                        Err(Error::NotFound) => None,
                        Err(e) => return Err(e),
                    }
                } else {
                    rep.expected
                };
                fs.set_delete_on_close(h, false)?;
                match fs.rename_replace_if(&rep.temp, &rep.target, expected) {
                    Ok(new) => {
                        sess.replaces.remove(&h);
                        sess.handles.remove(&h);
                        fs.close(h)?;
                        put_rev(&mut w, new);
                    }
                    Err(e) => {
                        let _ = fs.set_delete_on_close(h, true);
                        return Err(e);
                    }
                }
            }
            op::REPLACE_ABORT => {
                let h = Self::own(sess, r.u64()?)?;
                sess.replaces.remove(&h).ok_or(Error::BadHandle)?;
                sess.handles.remove(&h);
                fs.close(h)?;
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(Reply::body(w))
    }
}
