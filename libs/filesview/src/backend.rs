//! Co spravce souboru potrebuje od systemu. V OS relace FS uzivatele
//! (`FsClient`) a kanal k `files-copy` (`user/gui-files`), na hostiteli
//! [`TestBackend`] nad FS v pameti.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::time::WallTime;
use fileops::mem::{MemFs, Node};
use fileops::proto::ClientMsg;
use fileops::{Kind, Vfs};
use fsproto::datasets::{DatasetEntry, Origin};
use fsproto::hist::{FileVersion, TrashEntry};
use fsproto::{Stat, StatEntry, avail, kind};
use tz::Zone;

use crate::Result;

pub trait Backend {
    /// Domov uzivatele (`/home/<jmeno>`).
    fn home(&self) -> String;
    fn list(&mut self, dir: &str) -> Result<Vec<StatEntry>>;
    fn stat(&mut self, path: &str) -> Result<Stat>;
    fn datasets(&mut self) -> Result<Vec<DatasetEntry>>;
    fn dataset_of(&mut self, path: &str) -> Result<String>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    fn rename(&mut self, from: &str, to: &str) -> Result<()>;
    /// Do kose; vraci cislo polozky kose (pro Vratit).
    fn trash(&mut self, path: &str) -> Result<u64>;
    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>>;
    fn trash_list(&mut self, dataset: &str) -> Result<Vec<TrashEntry>>;
    fn trash_restore(&mut self, dataset: &str, id: u64) -> Result<String>;
    fn trash_purge(&mut self, dataset: &str, id: u64) -> Result<()>;
    fn versions(&mut self, path: &str) -> Result<Vec<FileVersion>>;
    /// Obnovit verzi: `dest = None` na misto souboru (soucasny obsah
    /// zustane v historii), jinak kopie vedle.
    fn restore_version(&mut self, path: &str, snapshot: &str, dest: Option<&str>) -> Result<String>;
    /// Zprava pomocnemu procesu kopirovani.
    fn copier(&mut self, m: ClientMsg);
    /// Otevrit soubor v aplikaci podle typu (sluzba "otevri cestu" v initu,
    /// `libs/openwith`). Neceka se na odpoved: sluzba muze volat akci
    /// tohoto okna (soubor bez aplikace ukaze spravce). `false` = sluzba
    /// neni.
    fn open(&mut self, path: &str) -> bool;
    fn now(&self) -> Option<WallTime>;
    fn zone(&self) -> Option<&Zone>;
}

/// Backend pro testy na hostiteli: FS v pameti (`fileops::mem`), masky
/// dostupnosti, kos, verze a zaznam zprav pro `files-copy`.
#[derive(Debug, Default)]
pub struct TestBackend {
    pub fs: MemFs,
    pub home: String,
    /// Slozky, jejichz souhrn je skryty (relace bez `MANAGE`).
    pub hidden_tree: Vec<String>,
    /// Slozky bez prava vypisu (relace je vidi, ale `list` vrati
    /// `AccessDenied`; jako cizi domov v `/home`).
    pub locked: Vec<String>,
    pub versions: BTreeMap<String, Vec<FileVersion>>,
    pub sent: Vec<ClientMsg>,
    /// Cesty poslane sluzbe "otevri cestu".
    pub opened: Vec<String>,
    pub now: Option<WallTime>,
    /// Kos: (dataset, polozka, obsah).
    pub trash: Vec<(String, TrashEntry, Vec<(String, Node)>)>,
    next_trash: u64,
}

pub const T0: WallTime = WallTime { secs: 1_794_571_200, nanos: 0 };

impl TestBackend {
    /// Domov `/home/jana` s nekolika polozkami a datasetem Projekty.
    pub fn sample() -> Self {
        let mut fs = MemFs::default();
        fs.file("/home/jana/zprava.txt", b"Ahoj,\r\ndruhy radek\n")
            .file("/home/jana/Dokumenty/smlouva 2.txt", b"2")
            .file("/home/jana/Dokumenty/smlouva 10.txt", b"10")
            .file("/home/jana/obrazek.bmp", &sample_bmp(2, 2))
            .dataset("/home/jana/Projekty")
            .file("/home/jana/Projekty/plan.md", b"# plan");
        let mut b = Self { fs, home: String::from("/home/jana"), now: Some(T0), ..Self::default() };
        b.versions.insert(
            String::from("/home/jana/zprava.txt"),
            alloc::vec![FileVersion { snapshot: String::from("auto-1"), snapshot_id: 1, time: WallTime { secs: T0.secs - 3600, nanos: 0 }, size: 5, mtime: WallTime { secs: T0.secs - 4000, nanos: 0 }, version: 1 }],
        );
        b
    }

    fn entry(&mut self, path: &str) -> Result<StatEntry> {
        let m = self.fs.stat(path)?;
        let is_ds = self.fs.datasets.iter().any(|d| d == path);
        let k = match m.kind {
            Kind::File => kind::FILE,
            Kind::Link => kind::SYMLINK,
            Kind::Dir if is_ds => kind::DATASET,
            Kind::Dir => kind::DIR,
        };
        let (mut known, mut hidden) = (avail::SIZE | avail::MTIME | avail::RIGHTS, 0);
        let (mut files, mut dirs) = (0, 0);
        if m.kind == Kind::Dir {
            for p in self.fs.below(path) {
                match self.fs.nodes.get(&p) {
                    Some(Node::Dir) => dirs += 1,
                    _ => files += 1,
                }
            }
            if self.hidden_tree.iter().any(|h| h == path) {
                hidden |= avail::TREE;
                known &= !avail::SIZE;
                hidden |= avail::SIZE;
            } else {
                known |= avail::TREE;
            }
        }
        let tree = known & avail::TREE != 0;
        Ok(StatEntry {
            name: String::from(fileops::path::name(path)),
            kind: k,
            oid: oid(path),
            size: m.size,
            mtime: T0,
            rights: if self.locked.iter().any(|l| l == path) { fsproto::perm::rights::TRAVERSE } else { 0xff },
            tree_bytes: if tree { m.tree_bytes.unwrap_or(0) } else { 0 },
            tree_files: if tree { files } else { 0 },
            tree_dirs: if tree { dirs } else { 0 },
            known,
            hidden,
        })
    }
}

/// Maly obrazek BMP 24 bitu (testy, ukazkova data).
pub fn sample_bmp(w: u32, h: u32) -> Vec<u8> {
    let stride = (w * 3).div_ceil(4) * 4;
    let size = 54 + stride * h;
    let mut b = Vec::new();
    b.extend_from_slice(b"BM");
    for v in [size, 0, 54, 40, w, h] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    for _ in 0..6 {
        b.extend_from_slice(&0u32.to_le_bytes());
    }
    for y in 0..h {
        for x in 0..stride {
            b.push(if x < w * 3 { (x * 40 + y * 90) as u8 } else { 0 });
        }
    }
    b
}

/// Stabilni "oid" podle cesty (FNV-1a).
pub fn oid(path: &str) -> u64 {
    path.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

impl Backend for TestBackend {
    fn home(&self) -> String {
        self.home.clone()
    }

    fn list(&mut self, dir: &str) -> Result<Vec<StatEntry>> {
        if self.locked.iter().any(|l| l == dir) {
            return Err(abi::Error::AccessDenied);
        }
        let names = self.fs.list(dir)?;
        names.into_iter().map(|(n, _)| self.entry(&fileops::path::join(dir, &n))).collect()
    }

    fn stat(&mut self, path: &str) -> Result<Stat> {
        let e = self.entry(path)?;
        Ok(Stat { oid: e.oid, kind: e.kind, size: e.size, mtime: e.mtime, rights: e.rights, tree_bytes: e.tree_bytes, tree_files: e.tree_files, tree_dirs: e.tree_dirs, known: e.known, hidden: e.hidden, ..Stat::default() })
    }

    fn datasets(&mut self) -> Result<Vec<DatasetEntry>> {
        let mut v = alloc::vec![DatasetEntry { path: String::from("/"), id: 1, root: self.stat("/")?, origin: Origin::None }];
        for (i, d) in self.fs.datasets.clone().iter().enumerate() {
            v.push(DatasetEntry { path: d.clone(), id: i as u64 + 2, root: self.stat(d)?, origin: Origin::None });
        }
        Ok(v)
    }

    fn dataset_of(&mut self, path: &str) -> Result<String> {
        self.fs.dataset_of(path)
    }

    fn mkdir(&mut self, path: &str) -> Result<()> {
        self.fs.mkdir(path)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.fs.rename(from, to)
    }

    fn trash(&mut self, path: &str) -> Result<u64> {
        let ds = self.fs.dataset_of(path)?;
        let size = self.fs.stat(path)?.bytes().unwrap_or(0);
        self.fs.trash(path)?;
        let (p, tree) = self.fs.trashed.pop().expect("polozka kose");
        self.next_trash += 1;
        let e = TrashEntry { id: self.next_trash, kind: kind::FILE, oid: oid(&p), time: T0, actor: 1000, size, name: String::from(fileops::path::name(&p)), original_dir: String::from(fileops::path::parent(&p)) };
        self.trash.push((ds, e, tree));
        Ok(self.next_trash)
    }

    fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.fs.read(path, offset, len)
    }

    fn trash_list(&mut self, dataset: &str) -> Result<Vec<TrashEntry>> {
        Ok(self.trash.iter().filter(|(d, _, _)| d == dataset).map(|(_, e, _)| e.clone()).collect())
    }

    fn trash_restore(&mut self, dataset: &str, id: u64) -> Result<String> {
        let i = self.trash.iter().position(|(d, e, _)| d == dataset && e.id == id).ok_or(abi::Error::NotFound)?;
        let (_, e, tree) = self.trash.remove(i);
        let p = fileops::path::join(&e.original_dir, &e.name);
        for (rel, n) in tree {
            let q = if rel.is_empty() { p.clone() } else { fileops::path::join(&p, &rel) };
            self.fs.nodes.insert(q, n);
        }
        Ok(p)
    }

    fn trash_purge(&mut self, dataset: &str, id: u64) -> Result<()> {
        let n = self.trash.len();
        self.trash.retain(|(d, e, _)| !(d == dataset && e.id == id));
        if self.trash.len() == n { Err(abi::Error::NotFound) } else { Ok(()) }
    }

    fn versions(&mut self, path: &str) -> Result<Vec<FileVersion>> {
        Ok(self.versions.get(path).cloned().unwrap_or_default())
    }

    fn restore_version(&mut self, path: &str, snapshot: &str, dest: Option<&str>) -> Result<String> {
        let v = self.versions.get(path).and_then(|v| v.iter().find(|v| v.snapshot == snapshot)).ok_or(abi::Error::NotFound)?;
        let data = alloc::vec![b'v'; v.size as usize];
        let target = String::from(dest.unwrap_or(path));
        self.fs.nodes.insert(target.clone(), Node::File(data));
        Ok(target)
    }

    fn copier(&mut self, m: ClientMsg) {
        self.sent.push(m);
    }

    fn open(&mut self, path: &str) -> bool {
        self.opened.push(String::from(path));
        true
    }

    fn now(&self) -> Option<WallTime> {
        self.now
    }

    fn zone(&self) -> Option<&Zone> {
        None
    }
}
