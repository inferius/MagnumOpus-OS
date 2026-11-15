//! Souborovy system shellu nad sluzbou fsd (`fsproto`).

use alloc::string::String;
use alloc::vec::Vec;

use fsproto::hist::Change;
use fsproto::{FsClient, Stat, kind, perm};
use libos::time::WallTime;
use libos::Error;
use shlang::host::{Applied, Aside, DirEntry, Fs, FsChange, FsChangeKind, FsResult, Meta, NodeKind, ViewRef, hidden};
use shlang::vfs::path::{join, rebase};

/// Kde lezi klony bokem (skryta slozka). Zbytky po padu uklidi init pri
/// startu systemu, nez vznikne prvni relace (`user/init/src/terminal.rs`);
/// termd je nemaze - nevi, ktere patri zivym relacim jinych oken a uloh.
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

/// Prava FS v3 (`fs::rights`) -> `shlang::host::access`.
fn access(rights: u8) -> u8 {
    const READ: u8 = 1 << 2;
    const WRITE: u8 = 1 << 3;
    const EXECUTE: u8 = 1 << 6;
    let mut a = 0;
    if rights & READ != 0 {
        a |= shlang::host::access::READ;
    }
    if rights & WRITE != 0 {
        a |= shlang::host::access::WRITE;
    }
    if rights & EXECUTE != 0 {
        a |= shlang::host::access::EXEC;
    }
    a
}

/// Velikost a obsah: u slozky souhrn podstromu (FS ho drzi hned, bez
/// prochazeni). Bez prava MANAGE ho fsd posle jako nuly: velikost a obsah
/// jsou pak skryte (`hidden`), ne prazdna slozka; s pravem jsou nuly
/// prazdna slozka 0 B / 0 polozek.
fn sized(kind_raw: u8, rights: u8, size: u64, bytes: u64, files: u64, dirs: u64) -> (u64, Option<(u64, u64)>, u8) {
    if node(kind_raw) != NodeKind::Dir {
        return (size, None, 0);
    }
    if rights & fsproto::perm::rights::MANAGE == 0 && (bytes | files | dirs) == 0 {
        return (0, None, hidden::SIZE | hidden::TREE);
    }
    (bytes, Some((files, dirs)), 0)
}

/// Cas, ktery FS nevede (nula), je nedostupny.
fn time(t: WallTime) -> Option<i64> {
    (t.secs != 0 || t.nanos != 0).then(|| ns(t))
}

fn meta(s: &Stat) -> Meta {
    let (size, tree, hidden) = sized(s.kind, s.rights, s.size, s.tree_bytes, s.tree_files, s.tree_dirs);
    Meta {
        kind: node(s.kind),
        size,
        mtime: time(s.mtime),
        hidden,
        access: Some(access(s.rights)),
        dataset: s.kind == kind::DATASET,
        tree,
    }
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

/// Popis opravneni fsproto -> shell (`shlang::acl`, stejne bity).
fn acl_in(a: perm::Acl) -> shlang::acl::Acl {
    shlang::acl::Acl {
        protected: a.flags & perm::ACL_PROTECTED != 0,
        grants: a.grants.into_iter().map(|g| shlang::acl::Grant { who: g.who, rights: g.rights, flags: g.flags }).collect(),
    }
}

fn acl_out(a: &shlang::acl::Acl) -> perm::Acl {
    perm::Acl {
        flags: if a.protected { perm::ACL_PROTECTED } else { 0 },
        grants: a.grants.iter().map(|g| perm::Grant { who: g.who, rights: g.rights, flags: g.flags }).collect(),
    }
}

impl Fs for FsdFs {
    fn stat(&mut self, path: &str) -> FsResult<Meta> {
        self.client.stat(path).map(|s| meta(&s))
    }

    fn acl_view(&mut self, path: &str) -> FsResult<shlang::acl::View> {
        let v = self.client.acl_view(path)?;
        Ok(shlang::acl::View {
            perms: shlang::acl::Perms { owner: v.perms.owner, rights: v.perms.rights, acl: v.perms.acl.map(acl_in) },
            inherited: v
                .inherited
                .into_iter()
                .map(|g| shlang::acl::Inherited { who: g.who, rights: g.rights, from: g.from })
                .collect(),
        })
    }

    fn set_acl(&mut self, path: &str, acl: Option<&shlang::acl::Acl>) -> FsResult<()> {
        self.client.set_acl(path, acl.map(acl_out).as_ref())
    }

    /// Vypis i s metadaty jednim dotazem na stranku (`LIST_STAT_PAGE`).
    fn list(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        Ok(self
            .client
            .list_stat(path)?
            .into_iter()
            .map(|e| {
                let (size, tree, hidden) = sized(e.kind, e.rights, e.size, e.tree_bytes, e.tree_files, e.tree_dirs);
                DirEntry {
                    name: e.name,
                    meta: Meta {
                        kind: node(e.kind),
                        size,
                        mtime: time(e.mtime),
                        access: Some(access(e.rights)),
                        dataset: e.kind == kind::DATASET,
                        tree,
                        hidden,
                    },
                }
            })
            .collect())
    }

    fn list_names(&mut self, path: &str) -> FsResult<Vec<DirEntry>> {
        Ok(self
            .client
            .list(path)?
            .into_iter()
            .map(|e| DirEntry { name: e.name, meta: Meta { dataset: e.kind == kind::DATASET, ..Meta::basic(node(e.kind), 0) } })
            .collect())
    }

    fn read(&mut self, path: &str) -> FsResult<Vec<u8>> {
        let st = self.client.stat(path)?;
        if node(st.kind) == NodeKind::Dir {
            return Err(Error::IsADirectory);
        }
        self.client.read(path, 0, st.size as usize)
    }

    fn read_range(&mut self, path: &str, offset: u64, len: usize) -> FsResult<Vec<u8>> {
        self.client.read(path, offset, len)
    }

    /// Zapis je atomicky: data do docasneho souboru vedle, pak
    /// `REPLACE_COMMIT`. Pri chybe zustane puvodni soubor cely.
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
        // Nahrazeni pres `REPLACE_BEGIN`/`COMMIT`: docasny soubor zalozi
        // fsd vyhradne pro tento zapis (jedinecne jmeno, nikdy cizi soubor),
        // smaze ho pri chybe i padu a cil nahradi atomicky. Potvrzeni
        // `force` = prepsat aktualni stav (shell prepisuje; soubeh s jinymi
        // zapisy resi koncept radku - `SH_CONFLICT`). Neni treba zjistovat
        // revizi, takze staci pravo zapisu (bez cteni).
        let h = self.client.replace_begin(path, None)?;
        if let Err((_, e)) = self.client.write_h(h, 0, data) {
            let _ = self.client.replace_abort(h);
            return Err(e);
        }
        match self.client.replace_commit(h, true) {
            Ok(_) => Ok(()),
            Err(e) => {
                let _ = self.client.replace_abort(h);
                Err(e)
            }
        }
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

    fn aside_apply(&mut self, a: &Aside) -> FsResult<Applied> {
        let r = self.client.aside_apply(&a.root, None)?;
        let complete = r.conflicts_total <= r.conflicts.len();
        let conflicts = r.conflicts.into_iter().map(|c| rebase(&c.path, "/", &a.dataset)).collect();
        Ok(Applied { conflicts, complete })
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
        // Zurnal ukazuje jen potvrzene zaznamy (fsd commituje po intervalu):
        // zmeny programu, ktery prave skoncil, nejdriv potvrdit.
        if self.client.commit().is_err() {
            let changes = self.client.diff(&v.dataset, &v.snapshot, None)?;
            return Ok(changes.into_iter().map(|c| Self::change(&v.dataset, c)).collect());
        }
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
        Ok(l.into_iter().map(|e| DirEntry { name: e.name, meta: Meta::basic(node(e.kind), 0) }).collect())
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
            return Ok(Meta::basic(NodeKind::Dir, 0));
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
