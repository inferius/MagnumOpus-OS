//! Verzovane commity, logicke ID, vetve.
//!
//! - **Commit** = snapshot druhu `COMMIT` + autor, zprava, cas, rodice
//!   a **logicke ID** (`SnapMeta`). Posledni commit datasetu drzi
//!   `item::HEAD` (i s druhym rodicem rozpracovaneho merge).
//! - **Logicke ID** = BLAKE3 z jednoznacne reprezentace logickeho stromu
//!   (jmena, druhy, obsah, cas zmeny a vzniku, vlastnik, opravneni,
//!   rozsirene atributy) + rodicu a metadat commitu. Nezavisi na rozmisteni
//!   bloku, oid ani generacich - stejny strom na jinem svazku ma stejne ID
//!   (send/receive, sync, vrstva pro git). Pocita se jen pri verzovanem
//!   commitu: obsah souboru se hashuje znovu jen po zmene (`item::LHASH`
//!   pamatuje hash s verzi obsahu), adresare se projdou.
//! - **Vetev** = zapisovatelny klon posledniho commitu (vlastni dataset);
//!   zdrojovy dataset si ji pamatuje (`item::BRANCH`). Commity vetve maji
//!   za rodice commity puvodu, merge vetve zpet je trojcestny (`merge.rs`).

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::{DirEntry, Fs, Kind};
use crate::btree;
use crate::device::BlockDevice;
use crate::layout::{BLOCK_SIZE, Inode, KIND_DATASET, KIND_DIR, KIND_SYMLINK, Key, META_OID, ROOT_OID, Timestamp, item};
use crate::names::{fold_name, name_hash, validate_name};
use crate::node::Child;
use crate::perm::rights;
use crate::records::{
    CommitId, Head, LogicalHash, MAX_MESSAGE, Parent, SnapMeta, decode_branches, encode_branches, snapkind,
};

/// Commit v historii.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub id: CommitId,
    /// Snapshot commitu (cist jako kazdy snapshot) a jeho dataset.
    pub snapshot: String,
    pub dataset: u64,
    pub snapshot_id: u64,
    pub time: Timestamp,
    pub author: u64,
    pub message: String,
    pub parents: Vec<CommitId>,
}

/// Vetev datasetu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    /// Dataset vetve (klon) a jeho cesta (prazdna, kdyz uz neexistuje).
    pub dataset: u64,
    pub path: String,
}

/// Logicke ID jako text (64 hex znaku).
pub fn id_hex(id: &CommitId) -> String {
    let mut s = String::with_capacity(64);
    for b in id {
        s.push_str(&alloc::format!("{b:02x}"));
    }
    s
}

fn put_time(h: &mut blake3::Hasher, t: Timestamp) {
    h.update(&t.secs.to_le_bytes());
    h.update(&t.nanos.to_le_bytes());
}

/// Nove spocitane hashe obsahu (`item::LHASH`) k ulozeni.
pub(super) type Cache = Vec<(Key, Vec<u8>)>;

/// Rozpracovany adresar pri vypoctu hashe (pruchod bez rekurze).
struct Frame {
    oid: u64,
    inode: Inode,
    entries: Vec<DirEntry>,
    next: usize,
    /// (jmeno, druh, hash) hotovych potomku.
    done: Vec<(String, u8, [u8; 32])>,
}

impl<D: BlockDevice> Fs<D> {
    /// Metadata spolecna vsem druhum objektu (pro logicky hash).
    fn hash_meta(&mut self, root: &Child, oid: u64, i: &Inode, h: &mut blake3::Hasher) -> Result<()> {
        put_time(h, i.mtime);
        put_time(h, i.btime);
        h.update(&i.owner.to_le_bytes());
        match self.load_acl(root, i.acl)? {
            Some(a) => {
                let e = crate::perm::encode_acl(&a.normalized()?, 0);
                h.update(&(e.len() as u64).to_le_bytes());
                h.update(&e);
            }
            None => {
                h.update(&0u64.to_le_bytes());
            }
        }
        let attrs = self.xattrs_in(root, oid)?;
        h.update(&(attrs.len() as u64).to_le_bytes());
        for (n, v) in attrs {
            h.update(&(n.len() as u64).to_le_bytes());
            h.update(n.as_bytes());
            h.update(&(v.len() as u64).to_le_bytes());
            h.update(&v);
        }
        Ok(())
    }

    /// BLAKE3 obsahu souboru (s pameti `LHASH`; nove spocitane hashe se
    /// pridaji do `fresh` - ulozi je volajici az po pruchodu, strom se behem
    /// nej nemeni).
    pub(super) fn content_hash(&mut self, root: &Child, oid: u64, i: &Inode, fresh: &mut Cache) -> Result<[u8; 32]> {
        let key = Key::new(oid, item::LHASH, 0);
        if let Some(raw) = btree::get(&mut self.s, root, &key)? {
            let c = LogicalHash::decode(&raw)?;
            if c.version == i.version && c.generation == i.generation {
                return Ok(c.hash);
            }
        }
        let mut h = blake3::Hasher::new();
        let mut off = 0u64;
        while off < i.size {
            let n = ((i.size - off) as usize).min(256 * BLOCK_SIZE);
            let data = self.read_oid(root, oid, off, n, false)?;
            if data.is_empty() {
                return Err(Error::Corrupted);
            }
            h.update(&data);
            off += data.len() as u64;
        }
        let hash = *h.finalize().as_bytes();
        let c = LogicalHash { version: i.version, generation: i.generation, hash };
        fresh.push((key, c.encode()));
        Ok(hash)
    }

    /// Hash souboru nebo odkazu.
    fn leaf_hash(&mut self, root: &Child, oid: u64, i: &Inode, fresh: &mut Cache) -> Result<[u8; 32]> {
        let mut h = blake3::Hasher::new();
        if i.kind == KIND_SYMLINK {
            h.update(b"l");
            let t = self.readlink_oid(root, oid)?;
            h.update(&(t.len() as u64).to_le_bytes());
            h.update(t.as_bytes());
        } else {
            h.update(b"f");
            h.update(&i.size.to_le_bytes());
            let c = self.content_hash(root, oid, i, fresh)?;
            h.update(&c);
        }
        self.hash_meta(root, oid, i, &mut h)?;
        Ok(*h.finalize().as_bytes())
    }

    /// Logicky hash stromu `root` od korene datasetu. Vnoreny dataset se
    /// zapocita jen jmenem (ma vlastni historii).
    pub(super) fn tree_hash(&mut self, root: &Child, fresh: &mut Cache) -> Result<[u8; 32]> {
        let start = |fs: &mut Self, oid: u64| -> Result<Frame> {
            let inode = fs.inode(root, oid)?;
            let mut entries = fs.list_oid(root, oid)?;
            // Jednoznacne poradi: bajty jmena (nezavisle na hashich klicu).
            entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
            Ok(Frame { oid, inode, entries, next: 0, done: Vec::new() })
        };
        let mut stack = alloc::vec![start(self, ROOT_OID)?];
        loop {
            let top = stack.last_mut().unwrap();
            if top.next < top.entries.len() {
                let e = top.entries[top.next].clone();
                top.next += 1;
                match e.kind {
                    Kind::Dir => {
                        if stack.len() >= super::MAX_DEPTH {
                            return Err(Error::Corrupted);
                        }
                        let f = start(self, e.oid)?;
                        stack.push(f);
                    }
                    Kind::Dataset => {
                        let h = *blake3::hash(b"dataset").as_bytes();
                        stack.last_mut().unwrap().done.push((e.name, KIND_DATASET, h));
                    }
                    _ => {
                        let i = self.inode(root, e.oid)?;
                        let h = self.leaf_hash(root, e.oid, &i, fresh)?;
                        stack.last_mut().unwrap().done.push((e.name, i.kind, h));
                    }
                }
                continue;
            }
            // Adresar hotovy.
            let f = stack.pop().unwrap();
            let mut h = blake3::Hasher::new();
            h.update(b"d");
            h.update(&(f.done.len() as u64).to_le_bytes());
            for (n, k, ch) in &f.done {
                h.update(&(n.len() as u64).to_le_bytes());
                h.update(n.as_bytes());
                h.update(&[*k]);
                h.update(ch);
            }
            self.hash_meta(root, f.oid, &f.inode, &mut h)?;
            let hash = *h.finalize().as_bytes();
            match stack.last_mut() {
                None => return Ok(hash),
                Some(p) => {
                    let name = p.entries[p.next - 1].name.clone();
                    p.done.push((name, KIND_DIR, hash));
                }
            }
        }
    }

    /// ID commitu ze stromu a metadat.
    fn commit_id(tree: &[u8; 32], parents: &[Parent], author: u64, time: Timestamp, message: &str) -> CommitId {
        let mut h = blake3::Hasher::new();
        h.update(b"osfs-commit-v1");
        h.update(tree);
        h.update(&(parents.len() as u64).to_le_bytes());
        for p in parents {
            h.update(&p.id);
        }
        h.update(&author.to_le_bytes());
        put_time(&mut h, time);
        h.update(&(message.len() as u64).to_le_bytes());
        h.update(message.as_bytes());
        *h.finalize().as_bytes()
    }

    /// Zaznam `HEAD` aktivniho datasetu; zdedeny zaznam puvodu (klon) neplati
    /// - pak je posledni commit puvodni snapshot klonu, kdyz je commitem.
    pub(super) fn head_active(&mut self) -> Result<Head> {
        let root = self.s.root;
        let h = match btree::get(&mut self.s, &root, &Key::new(META_OID, item::HEAD, 0))? {
            Some(raw) => Head::decode(&raw)?,
            None => Head::default(),
        };
        if h.dataset == self.cur_ds {
            return Ok(h);
        }
        let (ods, osnap, _) = self.cur_origin;
        let mut out = Head { dataset: self.cur_ds, ..Head::default() };
        if ods != 0 {
            let back = self.cur_ds;
            self.activate(ods)?;
            let m = self.snap_meta(osnap)?;
            self.activate(back)?;
            if m.kind == snapkind::COMMIT {
                out.head = Some(Parent { id: m.id, dataset: ods, snapshot: osnap });
            }
        }
        Ok(out)
    }

    pub(super) fn put_head(&mut self, mut h: Head) -> Result<()> {
        h.dataset = self.cur_ds;
        self.put(Key::new(META_OID, item::HEAD, 0), h.encode())
    }

    /// Commit z metadat snapshotu.
    fn commit_info(dataset: u64, id: u64, name: String, m: SnapMeta) -> CommitInfo {
        CommitInfo {
            id: m.id,
            snapshot: name,
            dataset,
            snapshot_id: id,
            time: m.time,
            author: m.author,
            message: m.message,
            parents: m.parents.iter().map(|p| p.id).collect(),
        }
    }

    /// Verzovany commit aktivniho datasetu se zpravou `message`; rodice =
    /// posledni commit (+ rozpracovany merge).
    pub(super) fn commit_active(&mut self, message: &str) -> Result<CommitInfo> {
        if message.len() > MAX_MESSAGE {
            return Err(Error::InvalidArgs);
        }
        let head = self.head_active()?;
        let parents: Vec<Parent> = head.head.into_iter().chain(head.merging).collect();
        if !self.s.read_only {
            self.flush_all_data()?;
        }
        let root = self.s.root;
        let mut fresh = Cache::new();
        let tree = self.tree_hash(&root, &mut fresh)?;
        for (k, v) in fresh {
            self.put(k, v)?;
        }
        let author = self.actor();
        let id = Self::commit_id(&tree, &parents, author, self.now, message);
        let name = alloc::format!("c-{}", &id_hex(&id)[..12]);
        let meta = SnapMeta { kind: snapkind::COMMIT, id, author, parents, message: message.into(), ..SnapMeta::default() };
        let snap_id = self.s.next_snapshot;
        let new_head = Head {
            head: Some(Parent { id, dataset: self.cur_ds, snapshot: snap_id }),
            merging: None,
            dataset: self.cur_ds,
        };
        let (sid, meta) = self.snapshot_with(&name, meta, alloc::vec![(Key::new(META_OID, item::HEAD, 0), new_head.encode())])?;
        debug_assert_eq!(sid, snap_id);
        Ok(Self::commit_info(self.cur_ds, sid, name, meta))
    }

    /// Verzovany commit datasetu `dataset` (snapshot + autor, zprava, cas,
    /// rodice, logicke ID). Vyzaduje `MANAGE`.
    pub fn commit_version(&mut self, dataset: &str, message: &str) -> Result<CommitInfo> {
        self.require(dataset, rights::MANAGE)?;
        self.guarded(|fs| {
            fs.enter_dataset(dataset)?;
            fs.commit_active(message)
        })
    }

    /// Posledni commit datasetu (`None`, kdyz zadny neni).
    pub fn head(&mut self, dataset: &str) -> Result<Option<CommitInfo>> {
        self.require(dataset, rights::READ)?;
        self.enter_dataset(dataset)?;
        let Some(p) = self.head_active()?.head else { return Ok(None) };
        self.commit_at(&p).map(Some)
    }

    /// Commit podle mistni polohy (dataset, snapshot).
    pub(super) fn commit_at(&mut self, p: &Parent) -> Result<CommitInfo> {
        let back = self.cur_ds;
        self.activate(p.dataset)?;
        let r = (|| {
            let root = self.s.root;
            let raw = btree::get(&mut self.s, &root, &Key::new(META_OID, item::SNAPSHOT, p.snapshot))?.ok_or(Error::NotFound)?;
            let rec = crate::layout::SnapshotRecord::decode(&raw)?;
            let m = self.snap_meta(p.snapshot)?;
            if m.kind != snapkind::COMMIT || m.id != p.id {
                return Err(Error::NotFound);
            }
            Ok(Self::commit_info(p.dataset, p.snapshot, rec.name, m))
        })();
        self.activate(back)?;
        r
    }

    /// Rodice commitu (mistni polohy) - prazdne, kdyz commit uz neexistuje.
    pub(super) fn parents_of(&mut self, p: &Parent) -> Result<Vec<Parent>> {
        let back = self.cur_ds;
        self.activate(p.dataset)?;
        let m = self.snap_meta(p.snapshot);
        self.activate(back)?;
        let m = m?;
        Ok(if m.kind == snapkind::COMMIT && m.id == p.id { m.parents } else { Vec::new() })
    }

    /// Historie commitu datasetu od posledniho (vsechny predci, od
    /// nejnovejsiho; nejvic `max`). Commit, jehoz snapshot uz neexistuje,
    /// historii ukonci. Vyzaduje `READ`.
    pub fn log(&mut self, dataset: &str, max: usize) -> Result<Vec<CommitInfo>> {
        self.require(dataset, rights::READ)?;
        self.enter_dataset(dataset)?;
        // Halda podle casu: kazdy commit se precte jednou, nejnovejsi prvni,
        // konec po `max` commitech.
        let mut heap: alloc::collections::BinaryHeap<(Timestamp, CommitId, u64, u64)> = alloc::collections::BinaryHeap::new();
        let mut seen: alloc::collections::BTreeSet<CommitId> = alloc::collections::BTreeSet::new();
        let mut out = Vec::new();
        let push = |fs: &mut Self, p: Parent, heap: &mut alloc::collections::BinaryHeap<_>, seen: &mut alloc::collections::BTreeSet<CommitId>| {
            if seen.insert(p.id)
                && let Ok(c) = fs.commit_at(&p)
            {
                heap.push((c.time, p.id, p.dataset, p.snapshot));
            }
        };
        if let Some(h) = self.head_active()?.head {
            push(self, h, &mut heap, &mut seen);
        }
        while out.len() < max {
            let Some((_, id, dataset, snapshot)) = heap.pop() else { break };
            let p = Parent { id, dataset, snapshot };
            out.push(self.commit_at(&p)?);
            for q in self.parents_of(&p)? {
                push(self, q, &mut heap, &mut seen);
            }
        }
        Ok(out)
    }

    /// Spolecny predek dvou commitu v DAG (nejblizsi k `theirs`).
    pub(super) fn merge_base(&mut self, ours: &Parent, theirs: &Parent) -> Result<Option<Parent>> {
        let mut anc: alloc::collections::BTreeSet<CommitId> = alloc::collections::BTreeSet::new();
        let mut queue = alloc::vec![*ours];
        while let Some(p) = queue.pop() {
            if anc.insert(p.id) {
                queue.extend(self.parents_of(&p)?);
            }
        }
        let mut seen: alloc::collections::BTreeSet<CommitId> = alloc::collections::BTreeSet::new();
        let mut queue = alloc::collections::VecDeque::from([*theirs]);
        while let Some(p) = queue.pop_front() {
            if anc.contains(&p.id) {
                return Ok(Some(p));
            }
            if seen.insert(p.id) {
                queue.extend(self.parents_of(&p)?);
            }
        }
        Ok(None)
    }

    /// Nova vetev `name` datasetu `dataset`: zapisovatelny klon posledniho
    /// commitu na ceste `path`. Dataset musi mit commit (`BAD_STATE`).
    /// Vyzaduje `MANAGE` k datasetu a `CREATE` v cilove slozce.
    pub fn branch(&mut self, dataset: &str, name: &str, path: &str) -> Result<u64> {
        validate_name(name)?;
        self.require(dataset, rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        let src = self.cur_ds;
        let head = self.head_active()?.head.ok_or(Error::BadState)?;
        // Klon by obsahoval vstupy do tychz vnorenych datasetu (dve cesty
        // k jednomu datasetu) - zatim nepodporovano.
        if self.contains_datasets()? {
            return Err(Error::NotSupported);
        }
        if self.branches_active()?.iter().any(|(_, n)| fold_name(n) == fold_name(name)) {
            return Err(Error::AlreadyExists);
        }
        let commit = self.commit_at(&head)?;
        // Snapshot posledniho commitu muze lezet v jinem datasetu (vetev z vetve).
        let ds_path = if head.dataset == src { String::from(dataset) } else { self.dataset_path(head.dataset)? };
        let id = self.clone_snapshot(&ds_path, &commit.snapshot, path)?;
        self.guarded(|fs| {
            fs.activate(src)?;
            let key = Key::new(META_OID, item::BRANCH, name_hash(&fold_name(name)));
            let root = fs.s.root;
            let mut list = match btree::get(&mut fs.s, &root, &key)? {
                Some(raw) => decode_branches(&raw)?,
                None => Vec::new(),
            };
            list.push((id, crate::names::stored_name(name)));
            fs.put(key, encode_branches(&list))?;
            fs.activate(id)?;
            fs.put_head(Head { head: Some(head), merging: None, dataset: id })?;
            fs.commit()
        })?;
        Ok(id)
    }

    fn branches_active(&mut self) -> Result<Vec<(u64, String)>> {
        let root = self.s.root;
        let mut out = Vec::new();
        for (_, v) in btree::collect(&mut self.s, &root, &Key::new(META_OID, item::BRANCH, 0), &Key::new(META_OID, item::BRANCH + 1, 0))? {
            out.extend(decode_branches(&v)?);
        }
        Ok(out)
    }

    /// Vetve datasetu `dataset` (serazene podle jmena). Vyzaduje `READ`.
    pub fn branches(&mut self, dataset: &str) -> Result<Vec<BranchInfo>> {
        self.require(dataset, rights::READ)?;
        self.enter_dataset(dataset)?;
        let list = self.branches_active()?;
        let mut out: Vec<BranchInfo> = Vec::new();
        for (id, name) in list {
            let path = self.dataset_path(id).unwrap_or_default();
            out.push(BranchInfo { path, dataset: id, name });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Zrusi vetev `name` datasetu `dataset` i s jejim datasetem (klonem).
    pub fn delete_branch(&mut self, dataset: &str, name: &str) -> Result<()> {
        self.require(dataset, rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        let src = self.cur_ds;
        let (id, _) = self
            .branches_active()?
            .into_iter()
            .find(|(_, n)| fold_name(n) == fold_name(name))
            .ok_or(Error::NotFound)?;
        if let Ok(p) = self.dataset_path(id) {
            self.delete_dataset(&p)?;
        }
        self.guarded(|fs| {
            fs.activate(src)?;
            let key = Key::new(META_OID, item::BRANCH, name_hash(&fold_name(name)));
            let root = fs.s.root;
            // Smazani datasetu vetve zaznam uz odstranilo (`forget_branch`).
            let Some(raw) = btree::get(&mut fs.s, &root, &key)? else {
                return Ok(());
            };
            let mut list = decode_branches(&raw)?;
            list.retain(|(_, n)| fold_name(n) != fold_name(name));
            if list.is_empty() {
                fs.del(&key)?;
            } else {
                fs.put(key, encode_branches(&list))?;
            }
            fs.commit()
        })
    }

    /// Odstrani zaznamy vetvi, ktere ukazuji na dataset `id` (dataset
    /// zanikl), ve vsech datasetech.
    pub(super) fn forget_branch(&mut self, id: u64) -> Result<()> {
        let back = self.cur_ds;
        for ds in self.all_dataset_ids()? {
            if ds == id {
                continue;
            }
            self.activate(ds)?;
            let root = self.s.root;
            let items = btree::collect(&mut self.s, &root, &Key::new(META_OID, item::BRANCH, 0), &Key::new(META_OID, item::BRANCH + 1, 0))?;
            for (k, v) in items {
                let mut list = decode_branches(&v)?;
                let n = list.len();
                list.retain(|(d, _)| *d != id);
                if list.len() == n {
                    continue;
                }
                if list.is_empty() {
                    self.del(&k)?;
                } else {
                    self.put(k, encode_branches(&list))?;
                }
            }
        }
        self.activate(back)
    }

    /// Logicke ID zive verze datasetu (bez commitu; pro porovnani stromu
    /// napric svazky a testy). Vyzaduje `MANAGE`.
    pub fn tree_id(&mut self, dataset: &str) -> Result<[u8; 32]> {
        self.require(dataset, rights::MANAGE)?;
        self.enter_dataset(dataset)?;
        if !self.s.read_only {
            self.flush_all_data()?;
        }
        let root = self.s.root;
        let mut fresh = Cache::new();
        let h = self.tree_hash(&root, &mut fresh)?;
        if !self.s.read_only {
            self.guarded(|fs| {
                for (k, v) in fresh {
                    fs.put(k, v)?;
                }
                Ok(())
            })?;
        }
        Ok(h)
    }
}
