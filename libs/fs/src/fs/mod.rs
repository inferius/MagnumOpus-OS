//! Souborovy system: cesty, soubory, adresare, snapshoty, intent log.
//!
//! Zmeny se hromadi v pameti: metadata v rozpracovanych uzlech stromu, data
//! souboru v bufferu (odlozena alokace - bloky dostanou az pri zapisu, takze
//! lezi souvisle). `commit` vse ulozi atomicky; `fsync` udela zmeny trvalymi
//! rychle pres intent log (jeden zapis + flush), commit pak muze probihat
//! zridka. Cteni ze snapshotu pouziva stejny kod nad jinym korenem.
//!
//! Podmoduly: `data` (obsah souboru, extenty), `snap` (snapshoty, deadlisty),
//! `zil` (intent log), `defrag` (defragmentace), `check` (kontrola
//! konzistence).

mod check;
mod data;
mod dataset;
mod defrag;
mod diff;
mod handles;
mod snap;
mod zil;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::btree;
use crate::device::BlockDevice;
use crate::hash::Algo;
use crate::layout::{
    BLOCK_SIZE, Dirent, Inode, KIND_DATASET, KIND_DIR, KIND_FILE, KIND_SYMLINK, Key, ROOT_DATASET,
    ROOT_OID, SYMLINK_MAX,
    decode_dirents, encode_dirents, item,
};
use crate::node::MAX_VALUE;
use crate::names::{fold_name, name_hash, stored_name, validate_name};
use crate::node::Child;
use crate::store::{FormatOptions, Store};

pub use check::CheckReport;
pub use dataset::DatasetInfo;
pub use defrag::DefragReport;
pub use diff::{Change, ChangeKind};
pub use handles::{Handle, Revision, access, share};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// Symbolicky odkaz: cil je text (cesta), knihovna ho pri prochazeni
    /// cesty nenasleduje - to dela klient (VFS), aby slo hlidat smycky.
    Symlink,
    /// Vstup do jineho datasetu (jen ve vypisu adresare; cesta do nej
    /// prejde a `stat` vrati jeho korenovy adresar). Vytvari se
    /// `create_dataset` nebo `clone_snapshot`.
    Dataset,
}

impl Kind {
    fn from_raw(k: u8) -> Self {
        match k {
            KIND_DIR => Kind::Dir,
            KIND_SYMLINK => Kind::Symlink,
            KIND_DATASET => Kind::Dataset,
            _ => Kind::File,
        }
    }

    pub(crate) fn raw(self) -> u8 {
        match self {
            Kind::File => KIND_FILE,
            Kind::Dir => KIND_DIR,
            Kind::Symlink => KIND_SYMLINK,
            Kind::Dataset => KIND_DATASET,
        }
    }
}

/// Metadata objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub oid: u64,
    pub kind: Kind,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
    /// Cas vytvoreni.
    pub btime: u64,
    /// Generace vzniku objektu (spolu s `oid` jednoznacne urcuje objekt).
    pub generation: u64,
    /// U adresare: soucet velikosti vsech souboru v podstromu.
    pub tree_bytes: u64,
    /// U adresare: pocet souboru v podstromu.
    pub tree_files: u64,
    /// U adresare: pocet adresaru v podstromu (bez sebe).
    pub tree_dirs: u64,
    /// Dataset, ve kterem objekt lezi (1 = korenovy).
    pub dataset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub kind: Kind,
    pub oid: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsInfo {
    pub label: String,
    /// Jednoznacne ID svazku (ze superbloku).
    pub uuid: [u8; 16],
    pub block_size: u32,
    pub block_count: u64,
    /// Volne bloky (vcetne tech, ktere cekaji na odlozene uvolneni).
    pub free_blocks: u64,
    /// Z toho cekajici na odlozene uvolneni.
    pub deferred_blocks: u64,
    pub generation: u64,
    pub algo: Algo,
    pub ag_blocks: u64,
    /// O kolik generaci se pripojeni vratilo kvuli poskozenemu stavu (0 = ne).
    pub fallback: u64,
    /// Pripojeno jen pro cteni (zarizeni bez zaruky trvanlivosti, neznamy
    /// ro_compat, nejista obnova - viz dalsi pole).
    pub read_only: bool,
    /// Slotu superbloku, ktere nesly precist (vadne sektory).
    pub unreadable_superblocks: u32,
    /// Necitelny je prave slot, kde by lezela novejsi generace: nejnovejsi
    /// stav nejde bezpecne urcit, proto jen pro cteni.
    pub newest_uncertain: bool,
    /// Necitelny zbytek intent logu se pri pripojeni vedome zahodil
    /// (`MountOptions::discard_unreadable_log`).
    pub log_discarded: bool,
    /// Intent log nesel precist cely: prehrana jen cast, log zachovany.
    pub log_unreadable: bool,
    /// Kolik operaci se pri pripojeni prehralo z intent logu.
    pub replayed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub id: u64,
    pub name: String,
    pub generation: u64,
    /// Kolik zapisovatelnych klonu z nej vzniklo (a existuje).
    pub clones: u32,
}

/// Rozpracovana data jednoho souboru (odlozena alokace).
#[derive(Default)]
struct FileDirty {
    /// Logicky blok -> obsah (BLOCK_SIZE bajtu).
    blocks: BTreeMap<u64, Vec<u8>>,
}

pub struct Fs<D: BlockDevice> {
    s: Store<D>,
    now: u64,
    /// Rozpracovana data souboru podle oid.
    data: BTreeMap<u64, FileDirty>,
    dirty_blocks: u64,
    zil: zil::Pending,
    replayed: u64,
    /// Kde pokracuje defragmentace na pozadi (oid).
    defrag_cursor: u64,
    /// Pocet zmen stavu v pameti (strom, buffer dat) - `guarded` podle nej
    /// pozna, ze operace selhala az po zmene.
    mutations: u64,
    /// Volba pripojeni: necitelny zbytek logu zahodit (viz `MountOptions`).
    discard_unreadable_log: bool,
    log_discarded: bool,
    /// Aktivni dataset (jeho stav je v `s` a polich vyse).
    cur_ds: u64,
    /// Puvod aktivniho datasetu (klon): (dataset, snapshot, generace).
    cur_origin: (u64, u64, u64),
    cur_rec_dirty: bool,
    /// Ostatni nactene datasety.
    others: BTreeMap<u64, dataset::DsCtx>,
    /// Otevrene soubory, zamky rozsahu.
    handles: handles::Handles,
}

/// Volby pripojeni.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MountOptions {
    /// Vynutit jen pro cteni (kontrola nebo zachrana dat bez zapisu).
    pub read_only: bool,
    /// Kdyz intent log nejde precist cely (vadny sektor), **vedome zahodit**
    /// necitelny zbytek a pripojit zapisovatelne. Zmeny po posledni citelne
    /// davce (i potvrzene `fsync`) se ztrati - jen pro obnovu, kdyz chyba
    /// cteni nezmizi; bez volby je svazek jen pro cteni a log zustane.
    pub discard_unreadable_log: bool,
}

/// Nejvetsi hloubka adresaru pri prochazeni rodicu. Vic = cyklus rodicu
/// z poskozeneho zaznamu (`CORRUPTED`), ne zacykleni.
const MAX_DEPTH: usize = 65_536;

/// Kolik rozpracovanych bloku dat drzet v pameti, nez se zapisi (16 MiB).
const DIRTY_LIMIT: u64 = 4096;
/// Nad tolik bloku v bufferu (32 MiB) se dalsi zapis neprijme, dokud nejde
/// buffer zapsat na disk (opakovane selhavajici zarizeni).
const DIRTY_HARD_LIMIT: u64 = 2 * DIRTY_LIMIT;

fn split_path(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|c| !c.is_empty())
}

/// (cesta rodice, jmeno posledni slozky)
fn split_parent(path: &str) -> Result<(&str, &str)> {
    if !path.starts_with('/') {
        return Err(Error::InvalidArgs);
    }
    let trimmed = path.trim_end_matches('/');
    let i = trimmed.rfind('/').ok_or(Error::InvalidArgs)?;
    let name = &trimmed[i + 1..];
    if name.is_empty() {
        return Err(Error::InvalidArgs);
    }
    let parent = &trimmed[..i];
    Ok((if parent.is_empty() { "/" } else { parent }, name))
}

impl<D: BlockDevice> Fs<D> {
    /// Vytvori prazdny souborovy system na zarizeni.
    pub fn format(dev: D, opts: &FormatOptions) -> Result<D> {
        Store::format(dev, opts)
    }

    /// Pripoji souborovy system: stav po poslednim commitu a prehrani
    /// intent logu (zmeny potvrzene `fsync`).
    /// Pripoji svazek s vychozimi volbami. Zarizeni bez zaruky
    /// trvanlivosti (`Durability::None`) se pripoji jen pro cteni
    /// (`FsInfo::read_only`; zmeny vraci `READ_ONLY`).
    pub fn mount(dev: D) -> Result<Self> {
        Self::mount_with(dev, MountOptions::default())
    }

    /// Pripoji svazek s volbami (obnova, kontrola bez zapisu).
    pub fn mount_with(dev: D, opts: MountOptions) -> Result<Self> {
        let no_durability = dev.durability() == crate::device::Durability::None;
        let mut s = Store::open(dev)?;
        // Jen pro cteni i kvuli neznamemu bitu ro_compat a nejiste obnove
        // (nastavi `open`).
        s.read_only |= no_durability || opts.read_only;
        let mut fs = Self {
            s,
            now: 0,
            data: BTreeMap::new(),
            dirty_blocks: 0,
            zil: zil::Pending::default(),
            replayed: 0,
            defrag_cursor: 0,
            mutations: 0,
            discard_unreadable_log: opts.discard_unreadable_log,
            log_discarded: false,
            cur_ds: ROOT_DATASET,
            cur_origin: (0, 0, 0),
            cur_rec_dirty: false,
            others: BTreeMap::new(),
            handles: handles::Handles::default(),
        };
        fs.replay_log()?;
        // Sirotci z minula: po restartu je nikdo nema otevrene.
        if !fs.s.read_only {
            fs.cleanup_orphans()?;
        }
        Ok(fs)
    }

    /// Odpoji system bez commitu a vrati zarizeni (testy, nastroje).
    pub fn into_device(self) -> D {
        self.s.dev
    }

    /// Cas pro nasledujici zmeny (ns; zdroj voli volajici).
    pub fn set_time(&mut self, now: u64) {
        self.now = now;
    }

    /// Posilat zarizeni TRIM pro definitivne uvolnene bloky.
    pub fn set_discard(&mut self, on: bool) {
        self.s.discard = on;
    }

    /// Ulozi vsechny zmeny atomicky.
    pub fn commit(&mut self) -> Result<()> {
        self.s.check_writable()?;
        self.commit_all()
    }

    pub fn info(&self) -> FsInfo {
        let label = &self.s.sb.label;
        let end = label.iter().position(|&b| b == 0).unwrap_or(label.len());
        FsInfo {
            label: String::from_utf8_lossy(&label[..end]).into(),
            uuid: self.s.sb.uuid,
            block_size: BLOCK_SIZE as u32,
            block_count: self.s.sb.block_count,
            free_blocks: self.s.free_blocks(),
            deferred_blocks: self.s.space.deferred_blocks(),
            generation: self.s.sb.generation,
            algo: self.s.algo,
            ag_blocks: self.s.sb.ag_blocks,
            fallback: self.s.fallback,
            read_only: self.s.read_only,
            unreadable_superblocks: self.s.unreadable_slots,
            newest_uncertain: self.s.newest_uncertain,
            log_discarded: self.log_discarded,
            log_unreadable: self.s.log_unreadable,
            replayed: self.replayed,
        }
    }

    /// Pocet bloku space logu skupiny `ag` (testy zhusteni).
    #[cfg(test)]
    pub(crate) fn space_log_blocks(&self, ag: usize) -> usize {
        self.s.space.meta.log_blocks(ag)
    }

    /// Zajisti misto pro `blocks` novych bloku; kdyz chybi a jsou bloky
    /// cekajici na odlozene uvolneni, uvolni je commity.
    fn ensure_room(&mut self, blocks: u64) -> Result<()> {
        self.ensure_room_with(blocks, false)
    }

    /// Misto pro operaci, ktera misto uvolnuje (smi do poloviny rezervy).
    fn ensure_room_freeing(&mut self, blocks: u64) -> Result<()> {
        self.ensure_room_with(blocks, true)
    }

    fn ensure_room_with(&mut self, blocks: u64, freeing: bool) -> Result<()> {
        self.s.check_writable()?;
        let mut tries = 0;
        let room = |s: &Store<D>, n: u64| if freeing { s.has_room_freeing(n) } else { s.has_room(n) };
        while !room(&self.s, self.dirty_blocks + blocks) {
            if self.s.space.deferred_blocks() == 0 || tries > crate::layout::REUSE_DELAY + 1 {
                return Err(Error::NoSpace);
            }
            self.commit()?;
            // Commit bez zmen neposune transakci; prazdna zmena ji posune.
            self.s.mark_changed();
            tries += 1;
        }
        Ok(())
    }

    // --- cteni (nad libovolnym korenem: zivy stav nebo snapshot) ---

    fn inode(&mut self, root: &Child, oid: u64) -> Result<Inode> {
        let raw = btree::get(&mut self.s, root, &Key::new(oid, item::INODE, 0))?
            .ok_or(Error::Corrupted)?;
        Inode::decode(&raw)
    }

    fn live_inode(&mut self, oid: u64) -> Result<Inode> {
        let root = self.s.root;
        self.inode(&root, oid)
    }

    fn lookup_child(&mut self, root: &Child, dir: u64, name: &str) -> Result<Option<Dirent>> {
        let folded = fold_name(name);
        let key = Key::new(dir, item::DIRENT, name_hash(&folded));
        let Some(raw) = btree::get(&mut self.s, root, &key)? else {
            return Ok(None);
        };
        Ok(decode_dirents(&raw)?
            .into_iter()
            .find(|d| fold_name(&d.name) == folded))
    }

    /// Cesta ve strome `root` (snapshot): hranici datasetu neprechazi
    /// (pohled snapshotu na jiny dataset by ukazoval na bloky, ktere uz
    /// mohou patrit jinemu) - `NOT_SUPPORTED`. Zivou cestu prochazi `enter`.
    fn resolve(&mut self, root: &Child, path: &str) -> Result<u64> {
        if !path.starts_with('/') {
            return Err(Error::InvalidArgs);
        }
        let mut oid = ROOT_OID;
        for name in split_path(path) {
            if self.inode(root, oid)?.kind != KIND_DIR {
                return Err(Error::NotADirectory);
            }
            let d = self.lookup_child(root, oid, name)?.ok_or(Error::NotFound)?;
            if d.kind == KIND_DATASET {
                return Err(Error::NotSupported);
            }
            oid = d.oid;
        }
        Ok(oid)
    }

    fn stat_oid(&mut self, root: &Child, oid: u64) -> Result<Stat> {
        let i = self.inode(root, oid)?;
        Ok(Stat {
            oid,
            kind: Kind::from_raw(i.kind),
            size: i.size,
            mtime: i.mtime,
            ctime: i.ctime,
            btime: i.btime,
            generation: i.generation,
            tree_bytes: i.tree_bytes,
            tree_files: i.tree_files,
            tree_dirs: i.tree_dirs,
            dataset: self.cur_ds,
        })
    }

    fn list_oid(&mut self, root: &Child, oid: u64) -> Result<Vec<DirEntry>> {
        if self.inode(root, oid)?.kind != KIND_DIR {
            return Err(Error::NotADirectory);
        }
        let items = btree::collect(
            &mut self.s,
            root,
            &Key::new(oid, item::DIRENT, 0),
            &Key::new(oid, item::DIRENT + 1, 0),
        )?;
        let mut out = Vec::new();
        for (_, v) in items {
            for d in decode_dirents(&v)? {
                out.push(DirEntry {
                    name: d.name,
                    kind: Kind::from_raw(d.kind),
                    oid: d.oid,
                });
            }
        }
        out.sort_by_cached_key(|e| fold_name(&e.name));
        Ok(out)
    }

    // --- verejne cteni zive verze ---

    pub fn stat(&mut self, path: &str) -> Result<Stat> {
        let oid = self.enter(path)?;
        let root = self.s.root;
        self.stat_oid(&root, oid)
    }

    pub fn list(&mut self, path: &str) -> Result<Vec<DirEntry>> {
        let oid = self.enter(path)?;
        let root = self.s.root;
        self.list_oid(&root, oid)
    }

    pub fn read(&mut self, path: &str, offset: u64, len: usize) -> Result<Vec<u8>> {
        let oid = self.enter(path)?;
        let root = self.s.root;
        self.read_oid(&root, oid, offset, len, true)
    }

    /// Cil symbolickeho odkazu (`WrongType`, kdyz `path` neni odkaz).
    pub fn readlink(&mut self, path: &str) -> Result<String> {
        let oid = self.enter(path)?;
        let root = self.s.root;
        self.readlink_oid(&root, oid)
    }

    fn readlink_oid(&mut self, root: &Child, oid: u64) -> Result<String> {
        if self.inode(root, oid)?.kind != KIND_SYMLINK {
            return Err(Error::WrongType);
        }
        let raw = btree::get(&mut self.s, root, &Key::new(oid, item::SYMLINK, 0))?
            .ok_or(Error::Corrupted)?;
        String::from_utf8(raw).map_err(|_| Error::Corrupted)
    }

    // --- zapis do stromu ---

    /// Provede verejnou zmenovou operaci "vse, nebo nic" vuci disku: kdyz
    /// selze az po zmene stavu v pameti (napr. chyba cteni uzlu uprostred),
    /// stav by byl napul hotovy. Svazek pak prejde do stavu chyby (zadny
    /// commit ani fsync nepotvrdi napul hotovou operaci); znovu pripojit =
    /// stav posledniho commitu + intent log. Chyba pred prvni zmenou (spatne
    /// jmeno, neexistuje, malo mista) nic neohrozi.
    fn guarded<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let before = self.mutations;
        let was_poisoned = self.s.poisoned;
        let r = f(self);
        // Neplatne uvolneni se ohlasi jednou jako CORRUPTED; dal uz plati
        // stav chyby (IO_ERROR jako po kazdem selhani).
        if self.s.space.corrupt && !was_poisoned {
            self.s.poisoned = true;
            return Err(Error::Corrupted);
        }
        if r.is_err() && self.mutations != before {
            self.s.poisoned = true;
        }
        r
    }

    /// Testy: primy zapis polozky (simulace poskozene hodnoty na disku).
    #[cfg(test)]
    pub(crate) fn test_put(&mut self, key: Key, value: Vec<u8>) -> Result<()> {
        self.put(key, value)
    }

    /// Testy: primo precte polozku stromu.
    #[cfg(test)]
    pub(crate) fn test_get(&mut self, key: Key) -> Result<Vec<u8>> {
        let root = self.s.root;
        btree::get(&mut self.s, &root, &key)?.ok_or(Error::NotFound)
    }

    /// Testy: blok, do ktereho pujde dalsi davka intent logu.
    #[cfg(test)]
    pub(crate) fn test_zil_next(&self) -> u64 {
        self.s.zil_next
    }

    /// Testy: primy zapis zaznamu objektu.
    #[cfg(test)]
    pub(crate) fn test_inode(&mut self, path: &str) -> Result<(u64, Inode)> {
        let oid = self.enter(path)?;
        let root = self.s.root;
        Ok((oid, self.inode(&root, oid)?))
    }

    /// Vlozi nebo nahradi polozku v zivem stromu.
    fn put(&mut self, key: Key, value: Vec<u8>) -> Result<()> {
        self.mutations += 1;
        let mut root = self.s.root;
        let r = btree::insert(&mut self.s, &mut root, key, value);
        self.s.root = root;
        r
    }

    /// Odstrani polozku ze ziveho stromu; vraci, zda existovala.
    fn del(&mut self, key: &Key) -> Result<bool> {
        self.mutations += 1;
        let mut root = self.s.root;
        let r = btree::remove(&mut self.s, &mut root, key);
        self.s.root = root;
        r
    }

    // --- zmeny adresaru (na urovni oid, aby je slo prehrat z logu) ---

    fn put_inode(&mut self, oid: u64, i: &Inode) -> Result<()> {
        self.put(Key::new(oid, item::INODE, 0), i.encode())
    }

    /// Promitne zmenu do souhrnu adresare `dir` a vsech jeho predku.
    ///
    /// `tree_bytes` saturuje a saturace je *trvala*: ridke soubory muzou mit
    /// dohromady logickou velikost nad 2^64 - 1 a pak je soucet neznamy.
    /// Hodnota `u64::MAX` = "prilis mnoho" a zadne odecteni ji nezmeni (jinak
    /// by vysla mensi, nez je skutecnost). Rozdil je `i128`, aby sel prenest
    /// cely souhrn podstromu (az 2^64 - 1).
    fn adjust_tree(&mut self, mut dir: u64, bytes: i128, files: i64, dirs: i64) -> Result<()> {
        // Sirotek (rodic 0) do zadneho souhrnu nepatri.
        if dir == 0 {
            return Ok(());
        }
        for _ in 0..MAX_DEPTH {
            let mut i = self.live_inode(dir)?;
            if i.tree_bytes != u64::MAX {
                i.tree_bytes = (i.tree_bytes as i128 + bytes).clamp(0, u64::MAX as i128) as u64;
            }
            i.tree_files = i.tree_files.saturating_add_signed(files);
            i.tree_dirs = i.tree_dirs.saturating_add_signed(dirs);
            self.put_inode(dir, &i)?;
            if dir == ROOT_OID {
                return Ok(());
            }
            dir = i.parent;
        }
        Err(Error::Corrupted)
    }

    fn add_dirent(&mut self, dir: u64, d: Dirent) -> Result<()> {
        let (key, value) = self.prepare_dirent(dir, d)?;
        self.put(key, value)
    }

    /// Klic a nova hodnota polozky adresare po pridani `d` - bez zmeny
    /// stromu, aby operace mohla selhat drive, nez neco zmeni.
    fn prepare_dirent(&mut self, dir: u64, d: Dirent) -> Result<(Key, Vec<u8>)> {
        let key = Key::new(dir, item::DIRENT, name_hash(&fold_name(&d.name)));
        let root = self.s.root;
        let mut list = match btree::get(&mut self.s, &root, &key)? {
            Some(raw) => decode_dirents(&raw)?,
            None => Vec::new(),
        };
        list.push(d);
        let value = encode_dirents(&list);
        // Jmena se stejnym hashem sdili jednu hodnotu; s dlouhymi jmeny by
        // mohla prerust uzel (prakticky jen pri umyslne vyrobene kolizi).
        if value.len() > MAX_VALUE {
            return Err(Error::NoResources);
        }
        Ok((key, value))
    }

    fn remove_dirent(&mut self, dir: u64, name: &str) -> Result<()> {
        let folded = fold_name(name);
        let key = Key::new(dir, item::DIRENT, name_hash(&folded));
        let root = self.s.root;
        let mut list =
            decode_dirents(&btree::get(&mut self.s, &root, &key)?.ok_or(Error::NotFound)?)?;
        list.retain(|d| fold_name(&d.name) != folded);
        if list.is_empty() {
            self.del(&key)?;
            Ok(())
        } else {
            self.put(key, encode_dirents(&list))
        }
    }

    fn touch_dir(&mut self, dir: u64) -> Result<()> {
        let mut i = self.live_inode(dir)?;
        i.mtime = self.now;
        self.put_inode(dir, &i)
    }

    fn dir_of(&mut self, path: &str) -> Result<u64> {
        let oid = self.enter(path)?;
        let root = self.s.root;
        if self.inode(&root, oid)?.kind != KIND_DIR {
            return Err(Error::NotADirectory);
        }
        Ok(oid)
    }

    /// Vytvori soubor nebo adresar.
    pub fn create(&mut self, path: &str, kind: Kind) -> Result<()> {
        self.guarded(|fs| {
            // Dataset vytvari `create_dataset` (vlastni strom).
            if kind == Kind::Dataset {
                return Err(Error::InvalidArgs);
            }
            let (parent_path, name) = split_parent(path)?;
            validate_name(name)?;
            let parent = fs.dir_of(parent_path)?;
            fs.ensure_room(4)?;
            let oid = fs.s.next_oid;
            let generation = fs.s.txg;
            fs.create_at(parent, name, kind, oid, generation)
        })
    }

    /// Vytvori symbolicky odkaz `path` s cilem `target` (libovolny text do
    /// `SYMLINK_MAX` bajtu bez NUL; cil nemusi existovat).
    pub fn symlink(&mut self, path: &str, target: &str) -> Result<()> {
        self.guarded(|fs| {
            let (parent_path, name) = split_parent(path)?;
            validate_name(name)?;
            validate_target(target)?;
            let parent = fs.dir_of(parent_path)?;
            fs.ensure_room(4)?;
            let oid = fs.s.next_oid;
            let generation = fs.s.txg;
            fs.symlink_at(parent, name, target, oid, generation)?;
            fs.zil.op(zil::Op::Symlink {
                parent,
                oid,
                generation,
                name: stored_name(name),
                target: target.into(),
                time: fs.now,
            });
            Ok(())
        })
    }

    /// `generation` = generace vzniku (pri prehrani logu ta puvodni).
    fn symlink_at(&mut self, parent: u64, name: &str, target: &str, oid: u64, generation: u64) -> Result<()> {
        self.new_object(parent, name, Kind::Symlink, oid, target.len() as u64, generation)?;
        self.put(Key::new(oid, item::SYMLINK, 0), target.as_bytes().to_vec())
    }

    fn create_at(&mut self, parent: u64, name: &str, kind: Kind, oid: u64, generation: u64) -> Result<()> {
        let name = self.new_object(parent, name, kind, oid, 0, generation)?;
        self.zil.op(zil::Op::Create {
            parent,
            oid,
            generation,
            kind: kind.raw(),
            name,
            time: self.now,
        });
        Ok(())
    }

    /// Zalozi zaznam objektu a polozku v rodici; vraci ulozene jmeno.
    fn new_object(
        &mut self,
        parent: u64,
        name: &str,
        kind: Kind,
        oid: u64,
        size: u64,
        generation: u64,
    ) -> Result<String> {
        let root = self.s.root;
        if self.lookup_child(&root, parent, name)?.is_some() {
            return Err(Error::AlreadyExists);
        }
        let next = oid.checked_add(1).ok_or(Error::NoResources)?;
        let name = stored_name(name);
        // Polozka adresare predem: kdyz se nevejde, nic se nezmenilo.
        let (dkey, dvalue) = self.prepare_dirent(
            parent,
            Dirent {
                oid,
                kind: kind.raw(),
                name: name.clone(),
            },
        )?;
        self.s.next_oid = self.s.next_oid.max(next);
        let mut inode = Inode::new(kind.raw(), parent, self.now, generation);
        inode.size = size;
        self.put_inode(oid, &inode)?;
        self.put(dkey, dvalue)?;
        self.touch_dir(parent)?;
        match kind {
            Kind::File | Kind::Symlink => self.adjust_tree(parent, 0, 1, 0)?,
            Kind::Dir => self.adjust_tree(parent, 0, 0, 1)?,
            Kind::Dataset => return Err(Error::InvalidArgs),
        }
        Ok(name)
    }

    /// Odstrani soubor nebo prazdny adresar.
    pub fn remove(&mut self, path: &str) -> Result<()> {
        self.guarded(|fs| {
            let (parent_path, name) = split_parent(path)?;
            let parent = fs.dir_of(parent_path)?;
            fs.ensure_room_freeing(4)?;
            fs.remove_at(parent, name)
        })
    }

    fn remove_at(&mut self, parent: u64, name: &str) -> Result<()> {
        let root = self.s.root;
        let d = self
            .lookup_child(&root, parent, name)?
            .ok_or(Error::NotFound)?;
        if d.kind == KIND_DATASET {
            // Dataset se maze `delete_dataset` (uvolni jeho bloky).
            return Err(Error::WrongType);
        }
        let inode = self.inode(&root, d.oid)?;
        // Otevreny objekt: smazani musi dovolit sdileni vsech handlu.
        let open = self.is_open(d.oid);
        if open {
            self.check_delete_allowed(d.oid)?;
        }
        if inode.kind == KIND_DIR {
            let first = btree::collect(
                &mut self.s,
                &root,
                &Key::new(d.oid, item::DIRENT, 0),
                &Key::new(d.oid, item::DIRENT + 1, 0),
            )?;
            if !first.is_empty() {
                return Err(Error::NotEmpty);
            }
        }
        if open {
            // Sirotek: data zustanou pro otevrene handly, zmizi s poslednim.
            self.make_orphan(d.oid, inode)?;
        } else {
            if inode.kind == KIND_SYMLINK {
                self.del(&Key::new(d.oid, item::SYMLINK, 0))?;
            } else if inode.kind != KIND_DIR {
                self.drop_content(d.oid)?;
            }
            self.del(&Key::new(d.oid, item::INODE, 0))?;
        }
        self.remove_dirent(parent, &d.name)?;
        self.touch_dir(parent)?;
        match inode.kind {
            KIND_DIR => self.adjust_tree(parent, 0, 0, -1)?,
            KIND_SYMLINK => self.adjust_tree(parent, 0, -1, 0)?,
            // Velikost <= MAX_FILE_SIZE (overeno pri cteni zaznamu).
            _ => self.adjust_tree(parent, -(inode.size as i128), -1, 0)?,
        }
        self.zil.op(zil::Op::Remove {
            parent,
            name: d.name,
            time: self.now,
        });
        self.zil.forget(d.oid);
        Ok(())
    }

    /// Presune nebo prejmenuje polozku. Cil nesmi existovat (krome zmeny
    /// velikosti pismen tehoz jmena).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        self.guarded(|fs| {
            let (fp, fname) = split_parent(from)?;
            let (tp, tname) = split_parent(to)?;
            validate_name(tname)?;
            let src = fs.dir_of(fp)?;
            let src_ds = fs.cur_ds;
            let dst = fs.dir_of(tp)?;
            // Presun mezi datasety = kopie dat (jine stromy), ne prejmenovani.
            if fs.cur_ds != src_ds {
                return Err(Error::NotSupported);
            }
            fs.ensure_room(8)?;
            fs.rename_at(src, fname, dst, tname)
        })
    }

    fn rename_at(&mut self, src: u64, fname: &str, dst: u64, tname: &str) -> Result<()> {
        let root = self.s.root;
        let d = self
            .lookup_child(&root, src, fname)?
            .ok_or(Error::NotFound)?;
        if let Some(existing) = self.lookup_child(&root, dst, tname)?
            && existing.oid != d.oid
        {
            return Err(Error::AlreadyExists);
        }
        // Prejmenovani otevreneho objektu: handly musi sdilet mazani
        // (pravidlo Windows FILE_SHARE_DELETE).
        if d.kind != KIND_DATASET && self.is_open(d.oid) {
            self.check_delete_allowed(d.oid)?;
        }
        if d.kind == KIND_DATASET {
            // Polozka datasetu nema zaznam objektu: jen presun polozky.
            let new_name = stored_name(tname);
            self.prepare_dirent(dst, Dirent { oid: d.oid, kind: d.kind, name: new_name.clone() })?;
            self.remove_dirent(src, &d.name)?;
            self.add_dirent(dst, Dirent { oid: d.oid, kind: d.kind, name: new_name.clone() })?;
            self.touch_dir(src)?;
            self.touch_dir(dst)?;
            self.zil.op(zil::Op::Rename { src, name: d.name, dst, new_name, time: self.now });
            return Ok(());
        }
        // Adresar nejde presunout do sveho podstromu (s mezi pro cyklus
        // rodicu z poskozeneho zaznamu).
        let mut a = dst;
        let mut depth = 0;
        while a != ROOT_OID {
            if a == d.oid {
                return Err(Error::InvalidArgs);
            }
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(Error::Corrupted);
            }
            a = self.inode(&root, a)?.parent;
        }
        // Cil predem: kdyz se polozka nevejde, nic se nezmenilo (konzervativni
        // pri stejnem hashi ve stejnem adresari - odebrani by misto uvolnilo).
        self.prepare_dirent(
            dst,
            Dirent {
                oid: d.oid,
                kind: d.kind,
                name: stored_name(tname),
            },
        )?;
        let mut inode = self.inode(&root, d.oid)?;
        // Pocty objektu se do i64 vejdou vzdy (omezuje je misto na disku).
        let sat = |v: u64| v.min(i64::MAX as u64) as i64;
        let (bytes, files, dirs) = match inode.kind {
            KIND_DIR => (
                inode.tree_bytes as i128,
                sat(inode.tree_files),
                sat(inode.tree_dirs).saturating_add(1),
            ),
            KIND_SYMLINK => (0, 1, 0),
            _ => (inode.size as i128, 1, 0),
        };
        let new_name = stored_name(tname);
        self.remove_dirent(src, &d.name)?;
        self.add_dirent(
            dst,
            Dirent {
                oid: d.oid,
                kind: d.kind,
                name: new_name.clone(),
            },
        )?;
        inode.parent = dst;
        inode.ctime = self.now;
        self.put_inode(d.oid, &inode)?;
        self.touch_dir(src)?;
        self.touch_dir(dst)?;
        if src != dst {
            self.adjust_tree(src, -bytes, -files, -dirs)?;
            self.adjust_tree(dst, bytes, files, dirs)?;
        }
        self.zil.op(zil::Op::Rename {
            src,
            name: d.name,
            dst,
            new_name,
            time: self.now,
        });
        Ok(())
    }
}

/// Cil symbolickeho odkazu: neprazdny, do `SYMLINK_MAX` bajtu, bez NUL
/// (zbytek se nekontroluje - odkaz muze mirit kamkoli, i neexistujici).
fn validate_target(target: &str) -> Result<()> {
    if target.is_empty() || target.len() > SYMLINK_MAX || target.contains('\0') {
        return Err(Error::InvalidArgs);
    }
    Ok(())
}
