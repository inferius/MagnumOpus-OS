//! Souborovy system shellu nad sluzbou fsd (`fsproto`).

use alloc::string::String;
use alloc::vec::Vec;

use fsproto::hist::Change;
use fsproto::{FsClient, Stat, kind};
use libos::time::WallTime;
use libos::Error;
use shlang::host::{Aside, DirEntry, Fs, FsChange, FsChangeKind, FsResult, Meta, NodeKind, ViewRef};
use shlang::vfs::path::{join, rebase};

/// Kde lezi klony bokem (skryta slozka; po padu je uklidi termd pri startu).
pub const ASIDE_DIR: &str = "/tmp/.aside";

pub struct FsdFs {
    pub client: FsClient,
    /// Jedinecna znacka relace ve jmenech klonu (`/tmp/.aside/<tag>-<n>`).
    tag: String,
    next: u64,
}

fn node(k: u8) -> NodeKind {
    match k {
        kind::DIR | kind::DATASET => NodeKind::Dir,
        kind::SYMLINK => NodeKind::Symlink,
        _ => NodeKind::File,
    }
}

fn ns(t: WallTime) -> i64 {
    t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64)
}

fn meta(s: &Stat) -> Meta {
    Meta { kind: node(s.kind), size: s.size, mtime: Some(ns(s.mtime)) }
}

impl FsdFs {
    pub fn new(client: FsClient, tag: String) -> Self {
        Self { client, tag, next: 0 }
    }

    /// Cesta uvnitr datasetu `ds` (protokol pro snapshoty a zmeny).
    fn inner(ds: &str, path: &str) -> String {
        rebase(path, ds, "/")
    }

    /// Zmena z protokolu (cesty uvnitr datasetu) -> absolutni cesty.
    fn change(ds: &str, c: Change) -> FsChange {
        let kind = match c.kind {
            1 => FsChangeKind::Added,
            2 => FsChangeKind::Removed,
            4 | 5 => FsChangeKind::Moved,
            _ => FsChangeKind::Modified,
        };
        let old_path = (!c.old_path.is_empty() && kind == FsChangeKind::Moved).then(|| rebase(&c.old_path, "/", ds));
        FsChange { kind, path: rebase(&c.path, "/", ds), old_path, is_dir: c.is_dir }
    }

    fn ensure_dir(&self, path: &str) -> FsResult<()> {
        match self.client.create(path, kind::DIR) {
            Ok(()) | Err(Error::AlreadyExists) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

impl Fs for FsdFs {
    fn stat(&mut self, path: &str) -> FsResult<Meta> {
        self.client.stat(path).map(|s| meta(&s))
    }

    /// Vypis i s metadaty jednim dotazem na stranku (`LIST_STAT_PAGE`).
    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        Ok(self
            .client
            .list_stat(path)?
            .into_iter()
            .map(|e| DirEntry { name: e.name, meta: Meta { kind: node(e.kind), size: e.size, mtime: Some(ns(e.mtime)) } })
            .collect())
    }

    fn list_names(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        Ok(self
            .client
            .list(path)?
            .into_iter()
            .map(|e| DirEntry { name: e.name, meta: Meta { kind: node(e.kind), size: 0, mtime: None } })
            .collect())
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

    fn dataset_of(&mut self, path: &str) -> FsResult<String> {
        self.client.dataset_of(path)
    }

    fn aside_begin(&mut self, dataset: &str) -> FsResult<Aside> {
        self.ensure_dir("/tmp")?;
        self.ensure_dir(ASIDE_DIR)?;
        self.next += 1;
        let root = join(ASIDE_DIR, &alloc::format!("{}-{}", self.tag, self.next));
        self.client.aside_begin(dataset, &root)?;
        Ok(Aside { dataset: dataset.into(), root })
    }

    fn aside_changes(&mut self, a: &Aside) -> FsResult<Vec<FsChange>> {
        Ok(self.client.aside_changes(&a.root)?.into_iter().map(|c| Self::change(&a.dataset, c)).collect())
    }

    fn aside_apply(&mut self, a: &Aside) -> FsResult<Vec<String>> {
        let r = self.client.aside_apply(&a.root, None)?;
        Ok(r.conflicts.into_iter().map(|c| rebase(&c.path, "/", &a.dataset)).collect())
    }

    fn aside_discard(&mut self, a: &Aside) -> FsResult<()> {
        self.client.aside_discard(&a.root)
    }

    fn view(&mut self, dataset: &str) -> FsResult<ViewRef> {
        let v = self.client.view(dataset)?;
        Ok(ViewRef { dataset: dataset.into(), snapshot: v.snapshot, seq: v.seq })
    }

    fn release_view(&mut self, v: &ViewRef) -> FsResult<()> {
        self.client.release_view(&v.dataset, &v.snapshot)
    }

    /// Zurnal rekne, jestli se neco zmenilo (jedna zprava); jen kdyz ano,
    /// `DIFF` vrati zmenene cesty.
    fn changed_since(&mut self, v: &ViewRef) -> FsResult<Vec<FsChange>> {
        let last = match self.client.journal_read(&v.dataset, v.seq + 1, 1) {
            Ok(p) => p.last,
            // Zurnal uz nema tak stare zaznamy: radsi spocitat diff.
            Err(Error::JournalGap) => u64::MAX,
            Err(e) => return Err(e),
        };
        if last <= v.seq {
            return Ok(Vec::new());
        }
        let changes = self.client.diff(&v.dataset, &v.snapshot, None)?;
        Ok(changes.into_iter().map(|c| Self::change(&v.dataset, c)).collect())
    }

    fn view_stat(&mut self, v: &ViewRef, path: &str) -> FsResult<Meta> {
        self.client.snapshot_stat_of(&v.dataset, &v.snapshot, &Self::inner(&v.dataset, path)).map(|s| meta(&s))
    }

    fn view_list(&mut self, v: &ViewRef, path: &str) -> FsResult<Vec<DirEntry>> {
        let l = self.client.snapshot_list_of(&v.dataset, &v.snapshot, &Self::inner(&v.dataset, path))?;
        Ok(l.into_iter().map(|e| DirEntry { name: e.name, meta: Meta { kind: node(e.kind), size: 0, mtime: None } }).collect())
    }

    fn view_read(&mut self, v: &ViewRef, path: &str) -> FsResult<Vec<u8>> {
        self.client.snapshot_read_file_of(&v.dataset, &v.snapshot, &Self::inner(&v.dataset, path))
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
