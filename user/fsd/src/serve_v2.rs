//! Operace protokolu v2: relace, handly, podminene ulozeni, zamky.
//!
//! Kazdy klient ma relaci (`Session`) s vlastnim kanalem a mnozinou handlu;
//! handle z jine relace = `BAD_HANDLE`. Zavreni relace zavre vsechny jeji
//! handly (docasne soubory podmineneho ulozeni zmizi - maji smazani pri
//! zavreni).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use fsproto::perm::Mount;
use fsproto::v2::{Revision, access as v2access, lock, op, open_flags, posix_lock};
use fsproto::{Reader, Writer};
use libos::{Channel, Error, Handle, Result};

use crate::serve::{ListCache, Service, entries, stat};

/// Kolik cisel docasneho jmena zkusit, nez REPLACE_BEGIN to vzda.
const TEMP_TRIES: usize = 64;

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
    /// Cislo relace (pro `GRANT`/`REVOKE`); ridici relace 0.
    pub id: u64,
    /// Kdo relaci pouziva (`None` = system).
    pub identity: Option<fs::Principal>,
    /// Pripojene cesty (jmenny prostor relace).
    pub mounts: Vec<Mount>,
    /// Ridici relace (zaklada dalsi relace s identitou).
    pub control: bool,
    /// Odlozene `JOURNAL_WATCH` (odpoved posle hlavni smycka).
    pub watch: Option<crate::serve_hist::Watch>,
    /// Klic registrace na portu hlavni smycky (0 = jeste nedostala).
    pub key: u64,
    /// Ceka registrace na portu (po paketu `false` -> obslouzit a obnovit).
    pub armed: bool,
}

impl Session {
    pub fn new(channel: Channel, id: u64, identity: Option<fs::Principal>, mounts: Vec<Mount>) -> Self {
        Self {
            channel,
            handles: BTreeSet::new(),
            list_cache: None,
            replaces: BTreeMap::new(),
            id,
            identity,
            mounts,
            control: false,
            watch: None,
            key: 0,
            armed: false,
        }
    }

    /// Ridici relace: system, cely FS.
    pub fn control(channel: Channel) -> Self {
        let mut s = Self::new(channel, 0, None, alloc::vec![Mount::new("/", "/", 0)]);
        s.control = true;
        s
    }

    /// Cesta relace -> cesta ve FS; `write` = operace meni FS (pripojeni
    /// jen pro cteni ji odmitne).
    pub fn map(&self, path: &str, write: bool) -> Result<String> {
        let (p, ro) = fsproto::perm::map_path(&self.mounts, path)?;
        if write && ro {
            return Err(Error::AccessDenied);
        }
        Ok(p)
    }

    /// Cesta ve FS -> cesta relace (pro cesty, ktere vraci FS). Cesta mimo
    /// pripojene se vrati prazdna (relace ji nevidi).
    pub fn unmap(&self, path: &str) -> String {
        fsproto::perm::unmap_path(&self.mounts, path).unwrap_or_default()
    }

    /// Vidi relace cely FS (`/ -> /`)? Operace nad celym korenovym
    /// datasetem (jeho snapshoty, kontrola) jinak nejdou.
    pub fn full_root(&self) -> Result<()> {
        let full = self.mounts.iter().any(|m| m.name == "/" && m.target == "/" && !m.read_only());
        if full { Ok(()) } else { Err(Error::AccessDenied) }
    }
}

/// Zmena jine relace, kterou provede hlavni smycka (ma vsechny relace).
pub enum SessionOp {
    Grant(u64, Mount),
    Revoke(u64, String),
}

/// Odpoved: telo, predavane handly, pripadne nova relace (`CONNECT`,
/// `CONNECT_AS`) a zmeny jinych relaci.
pub struct Reply {
    pub body: Writer,
    pub handles: Vec<Handle>,
    pub new_session: Option<Session>,
    pub session_ops: Vec<SessionOp>,
    /// Odpoved odlozit (`JOURNAL_WATCH` bez zaznamu).
    pub watch: Option<crate::serve_hist::Watch>,
}

impl Reply {
    pub fn body(body: Writer) -> Self {
        Self {
            body,
            handles: Vec::new(),
            new_session: None,
            session_ops: Vec::new(),
            watch: None,
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
        // Zamky POSIX relace (proces skoncil) a `flock` jejich handlu.
        self.posix.release_owner(sess.id);
        for &h in &sess.handles {
            self.posix.release_owner(flock_owner(h));
        }
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
        if (fsproto::perm::op::CONNECT_AS..=fsproto::perm::op::NOEXEC).contains(&opcode) || opcode == fsproto::perm::op::ACL_VIEW {
            return Some(self.perm_op(sess, opcode, r));
        }
        if !(op::HELLO..=op::REPLACE_ABORT).contains(&opcode) && !matches!(opcode, op::REOPEN | op::POSIX_LOCK | op::OPEN_STREAM) {
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
            // Nova relace se stejnou identitou a cestami. Priznak 1 = ridici
            // relace (jen z ridici: init po restartu sluzby, supervisor).
            let flags = r.u32().unwrap_or(0);
            let control = flags & fsproto::v2::connect_flags::CONTROL != 0;
            if control && !sess.control {
                return Err(Error::AccessDenied);
            }
            let (ours, theirs) = Channel::create()?;
            self.session_counter += 1;
            let mut s = Session::new(ours, self.session_counter, sess.identity.clone(), sess.mounts.clone());
            s.control = control;
            return Ok(Reply {
                body: w,
                handles: alloc::vec![theirs.into_handle()],
                new_session: Some(s),
                session_ops: Vec::new(),
                watch: None,
            });
        }
        let now = crate::serve::fs_now();
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        fs.set_time(now);
        match opcode {
            op::OPEN => {
                let raw = r.str()?;
                let (access, share, flags) = (r.u8()?, r.u8()?, r.u8()?);
                let writes = access & (v2access::WRITE | v2access::DELETE) != 0
                    || flags & (open_flags::CREATE | open_flags::TRUNCATE) != 0;
                let path = sess.map(&raw, writes)?;
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
            op::REOPEN => {
                // Trvaly handle po restartu sluzby (klient drzi identitu
                // objektu); prava overi FS znovu podle identity relace.
                let (ds, oid, generation) = (r.u64()?, r.u64()?, r.u64()?);
                let (access, share) = (r.u8()?, r.u8()?);
                let (h, path) = fs.reopen(ds, oid, generation, access, share)?;
                // Objekt s cestou musi byt v pripojenych cestach relace;
                // sirotek cestu nema (vlastnika overil FS).
                if let Some(p) = &path {
                    let writes = access & (v2access::WRITE | v2access::DELETE) != 0;
                    let mapped = sess.unmap(p);
                    if mapped.is_empty() || sess.map(&mapped, writes).is_err() {
                        let _ = fs.close(h);
                        return Err(Error::NotFound);
                    }
                }
                sess.handles.insert(h);
                w.u64(h);
                stat(fs.stat_h(h)?).put(&mut w);
                put_rev(&mut w, fs.revision_h(h)?);
            }
            op::CLOSE => {
                let h = Self::own(sess, r.u64()?)?;
                // Zamky `fcntl` relace na souboru konci se zavrenim jejiho
                // posledniho handlu na nem; `flock` se zavrenim handlu.
                let key = |fs: &mut fs::Fs<_>, h: u64| fs.revision_h(h).ok().map(|v| (v.dataset, v.oid, posix_lock::FCNTL));
                if let Some(k) = key(fs, h) {
                    let others = sess.handles.iter().filter(|&&x| x != h).any(|&x| key(fs, x) == Some(k));
                    if !others {
                        self.posix.release_file(k, sess.id);
                    }
                }
                self.posix.release_owner(flock_owner(h));
                sess.handles.remove(&h);
                sess.replaces.remove(&h);
                fs.close(h)?;
            }
            op::POSIX_LOCK => {
                let h = Self::own(sess, r.u64()?)?;
                let (cmd, class, write, start, len) = (r.u8()?, r.u8()?, r.u8()? != 0, r.u64()?, r.u64()?);
                let rev = fs.revision_h(h)?;
                let (owner, class) = match class {
                    posix_lock::FCNTL => (sess.id, posix_lock::FCNTL),
                    posix_lock::FLOCK => (flock_owner(h), posix_lock::FLOCK),
                    _ => return Err(Error::InvalidArgs),
                };
                let key = (rev.dataset, rev.oid, class);
                let (s, e) = fs::posix_lock::range(start, len);
                let kind = if write { fs::posix_lock::Kind::Write } else { fs::posix_lock::Kind::Read };
                let conflict = |w: &mut Writer, c: fs::posix_lock::Lock| {
                    let l = if c.end == u64::MAX { 0 } else { c.end - c.start };
                    w.u8(posix_lock::CONFLICT).u8(u8::from(c.kind == fs::posix_lock::Kind::Write)).u64(c.start).u64(l);
                };
                match cmd {
                    posix_lock::TEST => match self.posix.test(key, owner, s, e, kind) {
                        Some(c) => conflict(&mut w, c),
                        None => {
                            w.u8(posix_lock::DONE);
                        }
                    },
                    posix_lock::SET | posix_lock::SET_WAIT => {
                        let wait = cmd == posix_lock::SET_WAIT;
                        match self.posix.set(key, owner, s, e, kind, wait, libos::time::now()) {
                            fs::posix_lock::SetResult::Locked => {
                                w.u8(posix_lock::DONE);
                            }
                            fs::posix_lock::SetResult::Conflict(c) => conflict(&mut w, c),
                            fs::posix_lock::SetResult::Deadlock => {
                                w.u8(posix_lock::DEADLOCK);
                            }
                        }
                    }
                    posix_lock::UNLOCK => {
                        self.posix.unlock(key, owner, s, e);
                        w.u8(posix_lock::DONE);
                    }
                    _ => return Err(Error::InvalidArgs),
                }
            }
            op::OPEN_STREAM => {
                let h = Self::own(sess, r.u64()?)?;
                let (mode, offset) = (r.u8()?, r.u64()?);
                let need = match mode {
                    fsproto::v2::stream_mode::READ => v2access::READ,
                    fsproto::v2::stream_mode::WRITE | fsproto::v2::stream_mode::APPEND => v2access::WRITE,
                    _ => return Err(Error::InvalidArgs),
                };
                // Proud smi jen to, co handle klienta.
                fs.check_access_h(h, need)?;
                if self.streams.len() >= crate::stream::MAX_STREAMS {
                    return Err(Error::NoResources);
                }
                // Vlastni handle tehoz objektu: proud prezije zavreni relace.
                let v = fs.revision_h(h)?;
                let (own, _) = fs.reopen(v.dataset, v.oid, v.generation, need, fs::share::ALL)?;
                let (ours, theirs) = match Channel::create() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = fs.close(own);
                        return Err(e);
                    }
                };
                self.streams.push(crate::stream::Stream::new(ours, own, mode, offset));
                let mut rep = Reply::body(w);
                rep.handles.push(theirs.into_handle());
                return Ok(rep);
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
                let from = sess.map(&r.str()?, true)?;
                let to = sess.map(&r.str()?, true)?;
                fs.rename_replace(&from, &to)?;
            }
            op::REPLACE_BEGIN => {
                let target = sess.map(&r.str()?, true)?;
                let has = r.u8()? != 0;
                let rev = get_rev(r)?;
                let expected = has.then_some(rev);
                let (dir, name) = split(&target)?;
                fs::validate_name(name)?;
                let fs = self.fs.as_mut().ok_or(Error::IoError)?;
                // Vyhradne nove jmeno: existujici soubor (i zbytek po padu,
                // i cizi soubor stejneho jmena) se nikdy nemaze, vezme se
                // dalsi cislo.
                let mut temp = String::new();
                let mut made = Err(Error::AlreadyExists);
                for _ in 0..TEMP_TRIES {
                    self.temp_counter += 1;
                    temp = alloc::format!("{}/{}", dir.trim_end_matches('/'), fsproto::v2::temp_name(name, self.temp_counter));
                    made = fs.create(&temp, fs::Kind::File);
                    if !matches!(made, Err(Error::AlreadyExists)) {
                        break;
                    }
                }
                made?;
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

/// Vlastnik zamku `flock`: handle (odlisi se od cisel relaci hornim bitem).
fn flock_owner(h: u64) -> u64 {
    h | (1 << 63)
}
