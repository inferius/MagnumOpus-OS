//! Operace historie (protokol v2, op 54-86): zurnal zmen a jeho
//! sledovani, pohled, kos, vraceni operaci, verze, nastaveni datasetu,
//! commity, vetve a merge, "zkusit zmeny bokem", atributy a expirace.
//!
//! Cesty jsou cesty relace (`Session::map`); cesty, ktere vraci FS, se
//! prelozi zpet (`Session::unmap`). Prava kontroluje FS podle identity.
//!
//! **`JOURNAL_WATCH`** se odpovida hned, kdyz zaznamy jsou; jinak se
//! pozadavek odlozi (`Reply::watch`) a hlavni smycka odpovi po dalsim
//! potvrzeni (commit, fsync) nebo po casovem limitu.

use alloc::string::String;
use alloc::vec::Vec;

use fsproto::hist::{
    AutoRule, Branch, Change, CommitInfo, FileVersion, JournalEntry, JournalPage, MergeConflict, MergeReport,
    Settings, TrashEntry, Undone, conflict, get_opt_time, op, put_opt_time,
};
use fsproto::{Reader, Writer};
use libos::{Error, Result, time};

use crate::serve::Service;
use crate::serve_v2::{Reply, Session};

/// Nejdelsi cekani `JOURNAL_WATCH`.
const WATCH_MAX_MS: u32 = 60_000;

/// Odlozene `JOURNAL_WATCH` relace.
pub struct Watch {
    /// Dataset (cesta ve FS).
    pub dataset: String,
    pub from: u64,
    pub max: u32,
    /// Do kdy cekat (`time::now`).
    pub deadline: u64,
}

fn page(p: fs::JournalPage) -> JournalPage {
    JournalPage {
        first: p.first,
        last: p.last,
        entries: p
            .entries
            .into_iter()
            .map(|(seq, e)| JournalEntry {
                seq,
                op: e.op,
                kind: e.kind,
                time: e.time,
                actor: e.actor,
                oid: e.oid,
                generation: e.generation,
                parent: e.parent,
                new_parent: e.new_parent,
                size: e.size,
                aux: e.aux,
                name: e.name,
                new_name: e.new_name,
            })
            .collect(),
    }
}

fn commit(c: fs::CommitInfo) -> CommitInfo {
    CommitInfo {
        id: c.id,
        snapshot: c.snapshot,
        dataset: c.dataset,
        snapshot_id: c.snapshot_id,
        time: c.time,
        author: c.author,
        message: c.message,
        parents: c.parents,
    }
}

fn report(r: fs::MergeReport) -> MergeReport {
    MergeReport {
        applied: r.applied,
        conflicts: r
            .conflicts
            .into_iter()
            .map(|c| MergeConflict {
                path: c.path,
                kind: match c.kind {
                    fs::ConflictKind::Content => conflict::CONTENT,
                    fs::ConflictKind::ModifiedDeleted => conflict::MODIFIED_DELETED,
                    fs::ConflictKind::DeletedModified => conflict::DELETED_MODIFIED,
                    fs::ConflictKind::BothAdded => conflict::BOTH_ADDED,
                    fs::ConflictKind::BothMoved => conflict::BOTH_MOVED,
                    fs::ConflictKind::ParentMissing => conflict::PARENT_MISSING,
                    fs::ConflictKind::DirNotEmpty => conflict::DIR_NOT_EMPTY,
                    fs::ConflictKind::Cycle => conflict::CYCLE,
                },
                resolved_as: c.resolved_as,
            })
            .collect(),
        commit: r.commit.map(commit),
    }
}

fn kind_raw(k: fs::Kind) -> u8 {
    match k {
        fs::Kind::File => fsproto::kind::FILE,
        fs::Kind::Dir => fsproto::kind::DIR,
        fs::Kind::Symlink => fsproto::kind::SYMLINK,
        fs::Kind::Dataset => fsproto::kind::DATASET,
    }
}

fn put_list<T>(w: &mut Writer, items: &[T], f: impl Fn(&mut Writer, &T)) {
    let n = items.len().min(fsproto::hist::MAX_ITEMS);
    w.u32(n as u32);
    for x in &items[..n] {
        f(w, x);
    }
}

impl Service {
    /// Operace historie; `None` = neni z teto skupiny.
    pub fn handle_hist(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Option<Result<Reply>> {
        if !(op::JOURNAL_READ..=op::DATASET_DELETE).contains(&opcode) {
            return None;
        }
        Some(self.hist(sess, opcode, r))
    }

    fn hist(&mut self, sess: &mut Session, opcode: u32, r: &mut Reader) -> Result<Reply> {
        let mut w = Writer::new();
        let now = crate::serve::fs_now();
        let fs = self.fs.as_mut().ok_or(Error::IoError)?;
        fs.set_time(now);
        match opcode {
            op::JOURNAL_READ => {
                let ds = sess.map(&r.str()?, false)?;
                let (from, max) = (r.u64()?, r.u32()?);
                page(fs.journal_read(&ds, from, max as usize)?).put(&mut w);
            }
            op::JOURNAL_WATCH => {
                let ds = sess.map(&r.str()?, false)?;
                let (from, max, ms) = (r.u64()?, r.u32()?, r.u32()?.min(WATCH_MAX_MS));
                let p = fs.journal_read(&ds, from, max as usize)?;
                if !p.entries.is_empty() || ms == 0 {
                    page(p).put(&mut w);
                } else {
                    let mut reply = Reply::body(w);
                    reply.watch = Some(Watch {
                        dataset: ds,
                        from,
                        max,
                        deadline: time::deadline_after(core::time::Duration::from_millis(ms as u64)),
                    });
                    return Ok(reply);
                }
            }
            op::VIEW => {
                let ds = sess.map(&r.str()?, false)?;
                let v = fs.view(&ds)?;
                w.str(&v.snapshot).u64(v.id).u64(v.seq);
            }
            op::VIEW_RELEASE => {
                let ds = sess.map(&r.str()?, false)?;
                fs.release_view(&ds, &r.str()?)?;
            }
            op::TRASH => {
                let path = sess.map(&r.str()?, true)?;
                w.u64(fs.trash(&path)?);
            }
            op::TRASH_LIST => {
                let ds = sess.map(&r.str()?, false)?;
                let list: Vec<TrashEntry> = fs
                    .trash_list(&ds)?
                    .into_iter()
                    .map(|e| TrashEntry {
                        id: e.id,
                        kind: kind_raw(e.kind),
                        oid: e.oid,
                        time: e.time,
                        actor: e.actor,
                        size: e.size,
                        name: e.name,
                        original_dir: e.original_dir,
                    })
                    .collect();
                put_list(&mut w, &list, |w, e| e.put(w));
            }
            op::TRASH_RESTORE => {
                let ds = sess.map(&r.str()?, true)?;
                let id = r.u64()?;
                let dest = r.str()?;
                let dest = if dest.is_empty() { None } else { Some(sess.map(&dest, true)?) };
                let p = fs.trash_restore(&ds, id, dest.as_deref())?;
                w.str(&sess.unmap(&p));
            }
            op::TRASH_PURGE => {
                let ds = sess.map(&r.str()?, true)?;
                fs.trash_purge(&ds, r.u64()?)?;
            }
            op::UNDO => {
                let ds = sess.map(&r.str()?, true)?;
                let u = match fs.undo(&ds, r.u64()?)? {
                    fs::Undone::MovedBack(p) => Undone::MovedBack(sess.unmap(&p)),
                    fs::Undone::Restored(p) => Undone::Restored(sess.unmap(&p)),
                    fs::Undone::Trashed(id) => Undone::Trashed(id),
                    fs::Undone::FromSnapshot { path, snapshot, trashed } => {
                        Undone::FromSnapshot { path: sess.unmap(&path), snapshot, trashed }
                    }
                };
                u.put(&mut w);
            }
            op::FILE_VERSIONS => {
                let path = sess.map(&r.str()?, false)?;
                let list: Vec<FileVersion> = fs
                    .file_versions(&path)?
                    .into_iter()
                    .map(|v| FileVersion {
                        snapshot: v.snapshot,
                        snapshot_id: v.snapshot_id,
                        time: v.time,
                        size: v.size,
                        mtime: v.mtime,
                        version: v.version,
                    })
                    .collect();
                put_list(&mut w, &list, |w, v| v.put(w));
            }
            op::RESTORE_VERSION => {
                let path = sess.map(&r.str()?, true)?;
                let snap = r.str()?;
                let replace = r.u8()? != 0;
                let dest = r.str()?;
                let p = if replace {
                    fs.restore_version(&path, &snap, fs::RestoreMode::Replace)?
                } else {
                    let dest = sess.map(&dest, true)?;
                    fs.restore_version(&path, &snap, fs::RestoreMode::Copy(&dest))?
                };
                w.str(&sess.unmap(&p));
            }
            op::SETTINGS => {
                let ds = sess.map(&r.str()?, false)?;
                let s = fs.settings(&ds)?;
                Settings {
                    journal_max_entries: s.journal_max_entries,
                    journal_max_age: s.journal_max_age,
                    trash_max_age: s.trash_max_age,
                    rules: s.rules.iter().map(|r| AutoRule { interval: r.interval, keep: r.keep }).collect(),
                }
                .put(&mut w);
            }
            op::SET_SETTINGS => {
                let ds = sess.map(&r.str()?, true)?;
                let s = Settings::get(r)?;
                fs.set_settings(
                    &ds,
                    &fs::Settings {
                        journal_max_entries: s.journal_max_entries,
                        journal_max_age: s.journal_max_age,
                        trash_max_age: s.trash_max_age,
                        rules: s.rules.iter().map(|r| fs::AutoRule { interval: r.interval, keep: r.keep }).collect(),
                    },
                )?;
            }
            op::COMMIT_VERSION => {
                let ds = sess.map(&r.str()?, true)?;
                commit(fs.commit_version(&ds, &r.str()?)?).put(&mut w);
            }
            op::HEAD => {
                let ds = sess.map(&r.str()?, false)?;
                match fs.head(&ds)? {
                    Some(c) => {
                        w.u8(1);
                        commit(c).put(&mut w);
                    }
                    None => {
                        w.u8(0);
                    }
                }
            }
            op::LOG => {
                let ds = sess.map(&r.str()?, false)?;
                let max = (r.u32()? as usize).min(fsproto::hist::MAX_ITEMS);
                let list: Vec<CommitInfo> = fs.log(&ds, max)?.into_iter().map(commit).collect();
                put_list(&mut w, &list, |w, c| c.put(w));
            }
            op::BRANCH => {
                let ds = sess.map(&r.str()?, true)?;
                let name = r.str()?;
                let path = sess.map(&r.str()?, true)?;
                w.u64(fs.branch(&ds, &name, &path)?);
            }
            op::BRANCHES => {
                let ds = sess.map(&r.str()?, false)?;
                let list: Vec<Branch> = fs
                    .branches(&ds)?
                    .into_iter()
                    .map(|b| Branch { name: b.name, dataset: b.dataset, path: sess.unmap(&b.path) })
                    .collect();
                put_list(&mut w, &list, |w, b| {
                    w.str(&b.name).u64(b.dataset).str(&b.path);
                });
            }
            op::DELETE_BRANCH => {
                let ds = sess.map(&r.str()?, true)?;
                fs.delete_branch(&ds, &r.str()?)?;
            }
            op::MERGE => {
                let target = sess.map(&r.str()?, true)?;
                let source = sess.map(&r.str()?, false)?;
                report(fs.merge(&target, &source)?).put(&mut w);
            }
            op::MERGE_ABORT => {
                let target = sess.map(&r.str()?, true)?;
                fs.merge_abort(&target)?;
            }
            op::ASIDE_BEGIN => {
                let ds = sess.map(&r.str()?, false)?;
                let path = sess.map(&r.str()?, true)?;
                w.u64(fs.aside_begin(&ds, &path)?);
            }
            op::ASIDE_APPLY => {
                let clone = sess.map(&r.str()?, false)?;
                let all = r.u8()? != 0;
                let n = r.u32()? as usize;
                if n > fsproto::hist::MAX_ITEMS {
                    return Err(Error::InvalidArgs);
                }
                let sel = (0..n).map(|_| r.str()).collect::<Result<Vec<String>>>()?;
                report(fs.aside_apply(&clone, if all { None } else { Some(&sel) })?).put(&mut w);
            }
            op::ASIDE_DISCARD => {
                let clone = sess.map(&r.str()?, true)?;
                fs.aside_discard(&clone)?;
            }
            op::ASIDE_CHANGES => {
                let clone = sess.map(&r.str()?, false)?;
                let list: Vec<Change> = fs
                    .diff_clone(&clone)?
                    .into_iter()
                    .map(|c| Change {
                        kind: match c.kind {
                            fs::ChangeKind::Added => 1,
                            fs::ChangeKind::Removed => 2,
                            fs::ChangeKind::Modified => 3,
                            fs::ChangeKind::Moved => 4,
                            fs::ChangeKind::MovedModified => 5,
                        },
                        path: c.path,
                        old_path: c.old_path.unwrap_or_default(),
                        oid: c.oid,
                        is_dir: c.is_dir,
                    })
                    .collect();
                put_list(&mut w, &list, |w, c| {
                    w.u8(c.kind).str(&c.path).str(&c.old_path).u64(c.oid).u8(c.is_dir as u8);
                });
            }
            op::XATTR_GET => {
                let path = sess.map(&r.str()?, false)?;
                w.bytes(&fs.get_xattr(&path, &r.str()?)?);
            }
            op::XATTR_SET => {
                let path = sess.map(&r.str()?, true)?;
                let name = r.str()?;
                fs.set_xattr(&path, &name, r.rest())?;
            }
            op::XATTR_REMOVE => {
                let path = sess.map(&r.str()?, true)?;
                fs.remove_xattr(&path, &r.str()?)?;
            }
            op::XATTR_LIST => {
                let path = sess.map(&r.str()?, false)?;
                let names = fs.list_xattrs(&path)?;
                put_list(&mut w, &names, |w, n| {
                    w.str(n);
                });
            }
            op::SET_EXPIRES => {
                let path = sess.map(&r.str()?, true)?;
                fs.set_expires(&path, get_opt_time(r)?)?;
            }
            op::DATASET_CREATE => {
                let path = sess.map(&r.str()?, true)?;
                w.u64(fs.create_dataset(&path)?);
            }
            op::DATASET_DELETE => {
                let path = sess.map(&r.str()?, true)?;
                fs.delete_dataset(&path)?;
            }
            op::EXPIRES => {
                let path = sess.map(&r.str()?, false)?;
                put_opt_time(&mut w, fs.expires(&path)?);
            }
            _ => return Err(Error::NotSupported),
        }
        Ok(Reply::body(w))
    }

    /// Zkusi dokoncit odlozene `JOURNAL_WATCH`: odpoved (stranka), kdyz
    /// jsou zaznamy nebo uplynul limit; jinak `None`.
    pub fn poll_watch(&mut self, sess: &Session, w: &Watch, expired: bool) -> Option<Result<Writer>> {
        let fs = self.fs.as_mut()?;
        fs.set_principal(sess.identity.clone());
        let r = fs.journal_read(&w.dataset, w.from, w.max as usize);
        fs.set_principal(None);
        match r {
            Ok(p) if p.entries.is_empty() && !expired => None,
            Ok(p) => {
                let mut out = Writer::new();
                page(p).put(&mut out);
                Some(Ok(out))
            }
            Err(e) => Some(Err(e)),
        }
    }
}
