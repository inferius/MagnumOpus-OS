//! Uloziste: cache uzlu, copy-on-write, uvolnovani bloku a commit.
//!
//! Pravidla uvolneni bloku s generaci vzniku `birth` (zaklad konzistence):
//! - `birth == txg`: blok vznikl v rozpracovane transakci, zadny ulozeny stav
//!   na nej neukazuje - volny hned (ne u sdilenych bloku),
//! - `birth > snapshot_generation`: volny po odlozeni (`REUSE_DELAY`),
//! - jinak ho drzi snapshot: jde do seznamu mrtvych bloku (deadlist) a
//!   uvolni se az se smazanim snapshotu.
//!
//! **Sdilene bloky** (okamzite kopie, deduplikace) maji v tabulce sdileni
//! pocet odkazu. Kazde "definitivni" uvolneni datoveho useku (odlozene,
//! z deadlistu) proto jde pres frontu `unref`, kterou `Fs` pred commitem
//! zpracuje: usek v tabulce jen snizi pocet a uvolni se az s poslednim
//! odkazem. Dokud je tabulka prazdna (`share_live == false`), fronta se
//! obchazi.
//!
//! **Metadata 2x:** kazdy uzel stromu se zapise dvakrat (druha kopie od
//! konce disku, tedy daleko od prvni a pri vic AG v jine skupine). Cteni,
//! ktere u prvni kopie narazi na spatny soucet nebo chybu I/O, vezme druhou
//! a prvni prepise spravnym obsahem (samoleceni; obsah je dany souctem,
//! takze prepis na miste nic nerozbije).
//!
//! Commit: (data a deadlisty zapise `Fs`), uzly stromu od listu ke koreni do
//! jednoho souvisleho useku, space logy, flush, superblok do dalsiho slotu
//! kruhu, flush. Vypadek kdykoli pred zapisem superbloku = plati predchozi
//! stav, ktery nebyl nicim prepsan.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::device::BlockDevice;
use crate::hash::{Algo, xxh3};
use crate::layout::{
    BLOCK_SIZE, BlockPtr, DATA_START, DeadExtent, FIRST_FREE_OID, INODE_SIZE, Inode, KIND_DIR, Key,
    NODE_BLOCKS, NODE_SIZE, REUSE_DELAY, ROOT_OID, SB_COUNT, SB_FIRST, Superblock, item,
};
use crate::node::{Body, Child, Node};
use crate::space::{Class, INDEX_PER_BLOCK, Space, TABLE_PER_BLOCK};

/// Vysledek scrubu useku (`Store::scrub_units`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrubUnits {
    /// Spatnych kopii (kazda kopie zvlast).
    pub bad: u64,
    /// Opravenych kopii.
    pub repaired: u64,
    /// Jednotek bez jedine dobre kopie.
    pub lost: u64,
    /// Relativni blok prvni ztracene jednotky.
    pub first_lost: Option<u64>,
}

/// Kolik cistych uzlu drzet v pameti, nez se cache vyprazdni.
const CACHE_LIMIT: usize = 8192;
/// Nejmensi podporovane zarizeni (bloky).
pub const MIN_BLOCKS: u64 = 64;
/// Vychozi velikost alokacni skupiny: 1 GiB.
pub const DEFAULT_AG_BLOCKS: u64 = 262_144;

/// UUID svazku: zadane, jinak odvozene z jmena, velikosti a adresy
/// promenne na zasobniku. POZOR: v `no_std` neni zdroj nahody, takze
/// odvozene UUID neni zarucene jedinecne (dva stejne svazky formatovane
/// stejnym programem muzou mit stejne). Kdo formatuje, ma predat nahodne
/// UUID (xtask: `RandomState`; v systemu az bude sluzba nahody).
fn volume_uuid(label: &str, blocks: u64, given: Option<[u8; 16]>) -> [u8; 16] {
    if let Some(u) = given {
        return u;
    }
    let marker = 0u8;
    let mut seed = alloc::vec::Vec::new();
    seed.extend_from_slice(label.as_bytes());
    seed.extend_from_slice(&blocks.to_le_bytes());
    seed.extend_from_slice(&(&marker as *const u8 as u64).to_le_bytes());
    xxh3(&seed).0
}

/// Parametry formatovani.
#[derive(Debug, Clone)]
pub struct FormatOptions {
    pub label: alloc::string::String,
    pub algo: Algo,
    /// Velikost alokacni skupiny v blocich (None = vychozi, pro velke disky
    /// se zvetsi, aby tabulka skupin zustala mala).
    pub ag_blocks: Option<u64>,
    /// UUID svazku (None = odvodit).
    pub uuid: Option<[u8; 16]>,
    /// Cas vytvoreni korenoveho adresare.
    pub time: crate::layout::Timestamp,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            label: alloc::string::String::new(),
            algo: Algo::Xxh3,
            ag_blocks: None,
            uuid: None,
            time: crate::layout::Timestamp::UNIX_EPOCH,
        }
    }
}

pub struct Store<D: BlockDevice> {
    pub dev: D,
    pub algo: Algo,
    /// Posledni ulozeny stav.
    pub sb: Superblock,
    /// Rozpracovana transakce (= sb.generation + 1).
    pub txg: u64,
    pub root: Child,
    pub next_oid: u64,
    pub next_snapshot: u64,
    pub snapshot_generation: u64,
    pub dead_seq: u64,
    pub space: Space,
    /// Mrtve bloky drzene snapshoty, cekajici na zapis do deadlistu.
    pub dead: Vec<DeadExtent>,
    cache: BTreeMap<u64, Node>,
    dirty: BTreeMap<u64, Node>,
    next_mem: u64,
    changed: bool,
    /// Commit selhal uprostred: stav v pameti neodpovida disku, dalsi zmeny
    /// jsou zakazane (nutne znovu pripojit).
    pub poisoned: bool,
    /// Pripojeno jen pro cteni (zarizeni bez zaruky trvanlivosti, neznamy
    /// ro_compat, nejista obnova).
    pub read_only: bool,
    /// Kolik slotu superbloku neslo precist (chyba I/O) - jen pro hlaseni;
    /// o rezimu jen pro cteni rozhoduje `newest_uncertain`.
    pub unreadable_slots: u32,
    /// Necitelny je slot, kde by lezela novejsi generace: nejnovejsi stav
    /// nejde bezpecne urcit, proto jen pro cteni.
    pub newest_uncertain: bool,
    /// Intent log nesel precist cely (chyba I/O): prehrana je jen cast,
    /// log zustava na disku pro dalsi pokus, jen pro cteni.
    pub log_unreadable: bool,
    /// O kolik generaci se pripojeni vratilo kvuli poskozenemu stavu.
    pub fallback: u64,
    /// Sloty superbloku s novejsi (poskozenou) generaci, nez se pripojila.
    /// Pred prvnim commitem se vynuluji - jinak by po dalsim pripojeni
    /// vyhraly a ukazovaly na bloky, ktere mezitim dostal nekdo jiny.
    stale_slots: Vec<u64>,
    /// Posilat zarizeni TRIM pro uvolnene bloky.
    pub discard: bool,
    // --- intent log aktualni epochy ---
    pub zil_blocks: Vec<u64>,
    pub zil_next: u64,
    pub zil_seq: u64,
    /// Bloky dat, na ktere odkazuje intent log teto epochy (zacatek -> delka):
    /// do commitu se nesmi znovu pouzit, i kdyz vznikly v teto transakci.
    pub zil_logged: BTreeMap<u64, u64>,
    // --- sdilene bloky ---
    /// Tabulka sdileni muze mit zaznamy (nastavi `Fs` pri pripojeni a pri
    /// prvnim sdileni); jinak se uvolnuje primo.
    pub share_live: bool,
    /// Datove useky k definitivnimu uvolneni pres tabulku sdileni:
    /// (zacatek, delka, kopie).
    pub unref: Vec<(u64, u64, u64)>,
    // --- samoleceni ---
    /// Kolik poskozenych kopii (uzly, data) se od pripojeni opravilo
    /// z druhe kopie.
    pub healed: u64,
    /// Smi cteni opravovat spatnou kopii zapisem? Az kdyz pripojeni
    /// rozhodlo o rezimu (pripojeni jen pro cteni nesmi zapsat nic).
    pub heal: bool,
}

/// Velikost AG pro disk: vychozi, ale tolik, aby tabulka mela nejvys
/// ~10 tisic zaznamu (index ma 127 bloku tabulky).
fn ag_size(blocks: u64, wish: Option<u64>) -> u64 {
    let max_ags = (TABLE_PER_BLOCK * INDEX_PER_BLOCK) as u64;
    let mut ag = wish.unwrap_or(DEFAULT_AG_BLOCKS).max(MIN_BLOCKS);
    while blocks.div_ceil(ag) > max_ags {
        ag *= 2;
    }
    ag
}

impl<D: BlockDevice> Store<D> {
    /// Tvar casu na tomto svazku.
    pub fn time_format(&self) -> crate::layout::TimeFormat {
        crate::layout::TimeFormat::of(self.sb.features_incompat)
    }

    /// Vytvori prazdny souborovy system (jen korenovy adresar).
    pub fn format(mut dev: D, opts: &FormatOptions) -> Result<D> {
        let blocks = dev.block_count();
        // Ukazatel ma na cislo bloku 48 bitu.
        if blocks > crate::layout::MAX_BLOCKS {
            return Err(Error::NotSupported);
        }
        if blocks < MIN_BLOCKS {
            return Err(Error::InvalidArgs);
        }
        // Stare superbloky zrusit, aby pripojeni nenaslo predchozi system.
        let zero = vec![0u8; BLOCK_SIZE];
        for i in 0..SB_COUNT {
            dev.write_block(SB_FIRST + i, &zero)?;
        }
        dev.flush()?;
        let mut label = [0u8; 32];
        let n = opts.label.len().min(32);
        label[..n].copy_from_slice(&opts.label.as_bytes()[..n]);
        let ag_blocks = ag_size(blocks, opts.ag_blocks);
        let sb = Superblock {
            generation: 0,
            block_count: blocks,
            algo: opts.algo,
            ag_blocks,
            uuid: volume_uuid(&opts.label, blocks, opts.uuid),
            features_compat: 0,
            features_ro_compat: crate::layout::feature::RO_COMPAT_HISTORY,
            features_incompat: crate::layout::feature::INCOMPAT_TIME_SECS
                | crate::layout::feature::INCOMPAT_PTR_COPY
                | crate::layout::feature::INCOMPAT_DATA,
            root: BlockPtr::default(),
            space: BlockPtr::default(),
            zil_block: 0,
            zil_seed: 0,
            next_oid: FIRST_FREE_OID,
            snapshot_generation: 0,
            next_snapshot: 1,
            dead_seq: 0,
            unicode_version: crate::names::UNICODE_VERSION,
            device_index: 0,
            device_count: 1,
            label,
        };
        let mut s = Self::with_state(dev, sb, Space::new(blocks, ag_blocks), Child::Mem(0));
        let root_dir = Inode::new(KIND_DIR, 0, opts.time, 0);
        let tf = s.time_format();
        debug_assert_eq!(root_dir.encode(tf).len(), INODE_SIZE);
        let id = s.new_node(
            0,
            Body::Leaf(vec![(
                Key::new(ROOT_OID, item::INODE, 0),
                root_dir.encode(tf),
            )]),
        );
        s.root = Child::Mem(id);
        s.commit()?;
        Ok(s.dev)
    }

    fn with_state(dev: D, sb: Superblock, space: Space, root: Child) -> Self {
        Self {
            algo: sb.algo,
            txg: sb.generation + 1,
            root,
            next_oid: sb.next_oid,
            next_snapshot: sb.next_snapshot,
            snapshot_generation: sb.snapshot_generation,
            dead_seq: sb.dead_seq,
            space,
            dead: Vec::new(),
            cache: BTreeMap::new(),
            dirty: BTreeMap::new(),
            next_mem: 1,
            changed: false,
            poisoned: false,
            read_only: false,
            unreadable_slots: 0,
            newest_uncertain: false,
            log_unreadable: false,
            stale_slots: Vec::new(),
            fallback: 0,
            discard: false,
            zil_blocks: Vec::new(),
            zil_next: sb.zil_block,
            zil_seq: 0,
            zil_logged: BTreeMap::new(),
            share_live: false,
            unref: Vec::new(),
            healed: 0,
            heal: false,
            dev,
            sb,
        }
    }

    /// Pripoji system: nejnovejsi platny superblok, jehoz stav jde nacist.
    /// Kdyz je poskozeny (disk ztratil zapisy, prestoze potvrdil flush),
    /// zkusi az `REUSE_DELAY - 1` starsich - ty zarucene nikdo neprepsal.
    /// Pripoji svazek.
    ///
    /// 1. **Vyber:** ze slotu s platnou hlavickou (podpis rodiny formatu
    ///    a vlastni soucet) se vezme nejvyssi generace. Necitelny slot nebo
    ///    slot se spatnym souctem je neplatny slot, ostatni se zkusi.
    ///    Necitelny slot (chyba I/O) ale mohl drzet novejsi generaci, takze
    ///    ji nejde bezpecne urcit: pripojeni je pak jen pro cteni.
    /// 2. **Umim ji otevrit?** Jina verze formatu nebo neznamy bit incompat =
    ///    `NOT_SUPPORTED` (nikdy tichy prechod na starsi generaci). Neznamy
    ///    bit ro_compat = pripojeni jen pro cteni (replay intent logu jen
    ///    v pameti, zadny zapis).
    /// 3. **Obnova po poskozeni:** kdyz stav vybrane generace nejde nacist
    ///    kvuli poskozeni (`CORRUPTED`, vcetne nesmyslnych hodnot
    ///    v superbloku), zkusi se starsi (nejvys o `REUSE_DELAY - 1`)
    ///    a `fallback` rekne o kolik. Jina chyba (I/O, pamet) se vrati hned:
    ///    prechodna chyba nesmi vest k navratu na starsi stav. Novejsi sloty
    ///    se pred prvnim commitem vynuluji.
    pub fn open(mut dev: D) -> Result<Self> {
        // (slot, generace, obsah)
        let mut found: Vec<(u64, u64, Vec<u8>)> = Vec::new();
        let mut buf = vec![0u8; BLOCK_SIZE];
        let mut last_err = Error::NotFound;
        let mut unreadable: Vec<u64> = Vec::new();
        // Sloty s hlavickou, ktera nesedi (spatny soucet): roztrzeny zapis,
        // nebo pozdeji poskozeny superblok dokonceneho commitu.
        let mut damaged: Vec<u64> = Vec::new();
        for i in 0..SB_COUNT {
            let slot = SB_FIRST + i;
            if let Err(e) = dev.read_block(slot, &mut buf) {
                last_err = e;
                unreadable.push(slot);
                continue;
            }
            match Superblock::decode_header(&buf) {
                Ok((generation, _)) => found.push((slot, generation, buf.clone())),
                Err(Error::NotFound) => {}
                Err(e) => {
                    damaged.push(slot);
                    last_err = e;
                }
            }
        }
        found.sort_by_key(|&(_, g, _)| core::cmp::Reverse(g));
        let Some(newest) = found.first().map(|&(_, g, _)| g) else {
            return Err(last_err);
        };
        // Commity jdou postupne: generace newest + k lezi ve slotu
        // 1 + (newest + k) mod 8 a existuje jen, kdyz existuje i newest + k - 1.
        // Projdou se sloty za nejnovejsim: slot s citelnou starsi generaci
        // (nebo prazdny) = novejsi generace neexistuje, jistota. Poskozeny
        // slot (spatny soucet) mohl byt dokonceny commit, jde se dal.
        // Necitelny slot na teto ceste = nejnovejsi stav nejde urcit. Vadny
        // sektor jinde nic nezpochybnuje (jinak by byl svazek navzdy jen pro
        // cteni).
        let mut uncertain = false;
        for k in 1..SB_COUNT {
            let slot = SB_FIRST + (newest + k) % SB_COUNT;
            if unreadable.contains(&slot) {
                uncertain = true;
                break;
            }
            if !damaged.contains(&slot) {
                break;
            }
        }
        for (k, (_, generation, raw)) in found.iter().enumerate() {
            if generation + REUSE_DELAY <= newest {
                break;
            }
            let sb = match Superblock::decode(raw) {
                Ok(sb) if sb.block_count <= dev.block_count() => sb,
                // Hlavicka sedi, obsah ne (nebo vetsi nez zarizeni): poskozeni.
                Ok(_) | Err(Error::Corrupted) => {
                    last_err = Error::Corrupted;
                    continue;
                }
                // Jina verze, neznamy incompat: tento ovladac nesmi dal.
                Err(e) => return Err(e),
            };
            let read_only = sb.ro_compat_unknown();
            match Self::try_open(dev, sb) {
                Ok(mut s) => {
                    s.fallback = newest - generation;
                    // Necitelny slot mohl drzet novejsi generaci: zapis by
                    // ji po dalsim commitu prepsal. Jen pro cteni + hlaseni.
                    s.unreadable_slots = unreadable.len() as u32;
                    s.newest_uncertain = uncertain;
                    s.read_only = read_only || uncertain;
                    s.stale_slots = found[..k]
                        .iter()
                        .filter(|&&(_, g, _)| g > *generation)
                        .map(|&(slot, _, _)| slot)
                        .collect();
                    return Ok(s);
                }
                Err((d, Error::Corrupted)) => {
                    dev = d;
                    last_err = Error::Corrupted;
                }
                Err((_, e)) => return Err(e),
            }
        }
        Err(last_err)
    }

    fn try_open(mut dev: D, sb: Superblock) -> core::result::Result<Self, (D, Error)> {
        let txg = sb.generation + 1;
        let space = match Space::load(
            &mut dev,
            sb.algo,
            sb.block_count,
            sb.ag_blocks,
            &sb.space,
            txg,
        ) {
            Ok(s) => s,
            Err(e) => return Err((dev, e)),
        };
        let mut s = Self::with_state(dev, sb.clone(), space, Child::Disk(sb.root));
        if sb.zil_block != 0 {
            s.zil_blocks.push(sb.zil_block);
        }
        s.space.begin_txg(txg);
        let _ = s.space.take_discards();
        if let Err(e) = s.load(&sb.root) {
            return Err((s.dev, e));
        }
        Ok(s)
    }

    /// Volne bloky vcetne tech, ktere cekaji na odlozene uvolneni.
    pub fn free_blocks(&self) -> u64 {
        self.space.free_blocks() + self.space.deferred_blocks()
    }

    /// Rezerva pro commit (metadata, logy), kterou bezne zapisy nesmi pouzit.
    pub fn reserve(&self) -> u64 {
        (self.sb.block_count / 128).clamp(24, 65_536)
    }

    /// Je misto pro `extra` dalsich bloku (krome rozpracovanych uzlu a rezervy)?
    ///
    /// Pocita se jen misto, kam se vejdou cele uzly (4 souvisle bloky): data
    /// zapsana pri commitu muzou zabrat i takove useky, a commit by pak nemel
    /// kam ulozit strom (selhal by a svazek by presel do rezimu jen pro
    /// cteni). Cena: na silne rozdrobenem disku hlasi `NO_SPACE` o par bloku
    /// driv, nez je opravdu plny.
    pub fn has_room(&self, extra: u64) -> bool {
        self.has_room_keeping(extra, self.reserve())
    }

    /// Jako `has_room`, ale operace uvolnujici misto (smazani souboru nebo
    /// snapshotu, zkraceni) smi sahnout do poloviny rezervy - jinak by na
    /// plnem svazku neslo misto uvolnit. Druha polovina zustava pro commit.
    pub fn has_room_freeing(&self, extra: u64) -> bool {
        self.has_room_keeping(extra, self.reserve() / 2)
    }

    fn has_room_keeping(&self, extra: u64, keep: u64) -> bool {
        // Kazdy uzel se zapise dvakrat (metadata 2x).
        let nodes = self.dirty.len() as u64 * NODE_BLOCKS * 2;
        nodes + extra + keep <= self.space.node_room()
    }

    pub fn check_writable(&self) -> Result<()> {
        if self.read_only {
            Err(Error::ReadOnly)
        } else if self.poisoned {
            Err(Error::IoError)
        } else {
            Ok(())
        }
    }

    /// Uvolni uzel stromu (obe kopie) podle pravidel v hlavicce modulu.
    pub fn release_ptr(&mut self, p: &BlockPtr) {
        self.release_range(p.block, NODE_BLOCKS, p.copy, p.birth, false, false);
    }

    /// Uvolni fyzicke bloky `from..to` (relativne k zacatku) extentu.
    pub fn release_extent(&mut self, e: &crate::layout::Extent, from: u64, to: u64) {
        let copy = if e.copy != 0 { e.copy + from } else { 0 };
        self.release_range(e.phys + from, to - from, copy, e.birth, true, e.is_shared());
    }

    /// Uvolni usek (a jeho kopii) podle pravidel v hlavicce modulu.
    /// `data` = datovy usek (muze byt v tabulce sdileni), `shared` = extent
    /// s priznakem sdileni (nikdy se neuvolni hned).
    pub fn release_range(&mut self, start: u64, len: u64, copy: u64, birth: u64, data: bool, shared: bool) {
        self.changed = true;
        if birth == self.txg && !shared && !self.logged(start, len) {
            self.space.free(start, len, self.txg, true);
            if copy != 0 {
                self.space.free(copy, len, self.txg, true);
            }
        } else if birth > self.snapshot_generation {
            self.free_final(start, len, copy, data);
        } else {
            self.dead.push(DeadExtent { start, len, birth, copy });
        }
    }

    /// Definitivni (odlozene) uvolneni useku, ktery uz nikdo z ulozenych
    /// stavu nedrzi. Datovy usek jde pres tabulku sdileni (fronta `unref`).
    pub fn free_final(&mut self, start: u64, len: u64, copy: u64, data: bool) {
        self.changed = true;
        if data && self.share_live {
            self.unref.push((start, len, copy));
            return;
        }
        self.space.free(start, len, self.txg, false);
        if copy != 0 {
            self.space.free(copy, len, self.txg, false);
        }
    }

    /// Prekryva usek bloky, na ktere odkazuje intent log?
    fn logged(&self, start: u64, len: u64) -> bool {
        self.zil_logged
            .range(..start + len)
            .next_back()
            .is_some_and(|(&s, &l)| s + l > start)
    }

    /// Alokace pro data.
    pub fn alloc_data(&mut self, want: u64, goal: Option<u64>) -> Result<(u64, u64)> {
        self.changed = true;
        self.space.alloc(want, goal, Class::Data, self.txg)
    }

    /// Alokace jednoho bloku metadat (intent log).
    pub fn alloc_meta(&mut self) -> Result<u64> {
        self.changed = true;
        Ok(self.space.alloc(1, None, Class::Meta, self.txg)?.0)
    }

    /// Lezi usek `start..start+len` ve svazku (za superbloky)?
    fn inside(&self, start: u64, len: u64) -> bool {
        start >= DATA_START && start.checked_add(len).is_some_and(|e| e <= self.sb.block_count)
    }

    /// Precte `buf.len()` bajtu od `start` a overi je funkci `ok`; kdyz
    /// prvni kopie nesedi (nebo nejde precist) a existuje druha (`copy`),
    /// vezme druhou a prvni opravi. `Corrupted` = ani jedna kopie nesedi.
    pub fn read_verified(
        &mut self,
        start: u64,
        copy: u64,
        buf: &mut [u8],
        ok: impl Fn(Algo, &[u8]) -> bool,
    ) -> Result<()> {
        let first = self.dev.read_blocks(start, buf);
        if first.is_ok() && ok(self.algo, buf) {
            return Ok(());
        }
        if copy == 0 {
            return first.and(Err(Error::Corrupted));
        }
        let second = self.dev.read_blocks(copy, buf);
        if second.is_ok() && ok(self.algo, buf) {
            // Samoleceni: prvni kopii prepsat spravnym obsahem. Chyba
            // zapisu neni fatalni (cteni uspelo, scrub to zkusi znovu).
            if self.heal && !self.read_only && self.dev.write_blocks(start, buf).is_ok() {
                self.healed += 1;
            }
            return Ok(());
        }
        // Obe spatne: chyba I/O ma prednost (prechodna), jinak poskozeni.
        match (first, second) {
            (Err(e), _) | (_, Err(e)) if e != Error::Corrupted => Err(e),
            _ => Err(Error::Corrupted),
        }
    }

    /// Nacte ulozeny uzel a overi jeho kontrolni soucet (pri chybe z druhe
    /// kopie, viz `read_verified`).
    pub fn load(&mut self, ptr: &BlockPtr) -> Result<&Node> {
        if !self.cache.contains_key(&ptr.block) {
            let copy_ok = ptr.copy == 0 || (self.inside(ptr.copy, NODE_BLOCKS) && ptr.copy_dev == 0);
            if !self.inside(ptr.block, NODE_BLOCKS) || !copy_ok || ptr.len as u64 != NODE_BLOCKS || ptr.dev != 0 {
                return Err(Error::Corrupted);
            }
            // Komprimovany/sifrovany uzel tento ovladac neumi (svazek s
            // takovymi uzly ma incompat bit, sem se dostane jen po chybe).
            if ptr.flags != 0 {
                return Err(Error::NotSupported);
            }
            let mut buf = vec![0u8; NODE_SIZE];
            let csum = ptr.csum;
            self.read_verified(ptr.block, ptr.copy, &mut buf, |a, b| a.sum(b) == csum)?;
            let node = Node::decode(&buf)?;
            if self.cache.len() >= CACHE_LIMIT {
                self.cache.clear();
            }
            self.cache.insert(ptr.block, node);
        }
        Ok(&self.cache[&ptr.block])
    }

    /// Uzel podle odkazu (rozpracovany nebo ulozeny).
    pub fn node(&mut self, c: &Child) -> Result<&Node> {
        match c {
            Child::Mem(id) => self.dirty.get(id).ok_or(Error::Corrupted),
            Child::Disk(p) => self.load(p),
        }
    }

    /// Copy-on-write: vrati rozpracovany uzel (pri prvni zmene v transakci
    /// zkopiruje ulozeny a stary blok uvolni).
    pub fn cow(&mut self, c: &Child) -> Result<u64> {
        match *c {
            Child::Mem(id) => Ok(id),
            Child::Disk(p) => {
                let mut node = self.load(&p)?.clone();
                node.generation = self.txg;
                self.cache.remove(&p.block);
                self.release_ptr(&p);
                Ok(self.insert_dirty(node))
            }
        }
    }

    fn insert_dirty(&mut self, node: Node) -> u64 {
        let id = self.next_mem;
        self.next_mem += 1;
        self.dirty.insert(id, node);
        self.changed = true;
        id
    }

    /// Novy rozpracovany uzel urovne `level` (0 = list).
    pub fn new_node(&mut self, level: u8, body: Body) -> u64 {
        let generation = self.txg;
        self.insert_dirty(Node {
            generation,
            level,
            body,
        })
    }

    pub fn dirty_mut(&mut self, id: u64) -> &mut Node {
        self.dirty
            .get_mut(&id)
            .expect("uzel neni rozpracovany (chybi cow)")
    }

    /// Uvolni uzel stromu.
    pub fn free_node(&mut self, c: &Child) {
        match *c {
            Child::Mem(id) => {
                self.dirty.remove(&id);
            }
            Child::Disk(p) => {
                self.cache.remove(&p.block);
                self.release_ptr(&p);
            }
        }
        self.changed = true;
    }

    /// Precte souvisly usek nekomprimovanych dat a overi kontrolni soucet
    /// kazdeho bloku; `copy` = zacatek druhe kopie tehoz useku (0 = neni).
    pub fn read_data(
        &mut self,
        phys: u64,
        copy: u64,
        csums: &[crate::hash::Csum],
        buf: &mut [u8],
    ) -> Result<()> {
        debug_assert_eq!(buf.len(), csums.len() * BLOCK_SIZE);
        let n = csums.len() as u64;
        if !self.inside(phys, n) || (copy != 0 && !self.inside(copy, n)) {
            return Err(Error::Corrupted);
        }
        self.read_verified(phys, copy, buf, |a, b| {
            csums.iter().zip(b.chunks(BLOCK_SIZE)).all(|(c, blk)| a.sum(blk) == *c)
        })
    }

    /// Scrub useku: precte obe kopie (`copy` = 0: jen prvni), overi kazdou
    /// jednotku `unit` bloku proti `csums` a spatnou kopii prepise dobrou.
    /// Na rozdil od `read_verified` overi i druhou kopii, kdyz je prvni
    /// v poradku (jinak by se poskozena kopie prozradila az ve chvili, kdy
    /// je potreba).
    pub fn scrub_units(&mut self, start: u64, copy: u64, unit: u64, csums: &[crate::hash::Csum]) -> Result<ScrubUnits> {
        let n = unit * csums.len() as u64;
        if !self.inside(start, n) || (copy != 0 && !self.inside(copy, n)) {
            return Err(Error::Corrupted);
        }
        let size = unit as usize * BLOCK_SIZE;
        let read = |s: &mut Self, at: u64| {
            let mut b = vec![0u8; n as usize * BLOCK_SIZE];
            s.dev.read_blocks(at, &mut b).ok().map(|()| b)
        };
        let a = read(self, start);
        let b = if copy != 0 { read(self, copy) } else { None };
        let mut r = ScrubUnits::default();
        for (i, c) in csums.iter().enumerate() {
            let part = |x: &Option<Vec<u8>>| x.as_ref().map(|v| v[i * size..(i + 1) * size].to_vec());
            let (pa, pb) = (part(&a), part(&b));
            let ok_a = pa.as_ref().is_some_and(|d| self.algo.sum(d) == *c);
            let ok_b = copy != 0 && pb.as_ref().is_some_and(|d| self.algo.sum(d) == *c);
            let off = i as u64 * unit;
            if !ok_a {
                r.bad += 1;
            }
            if copy != 0 && !ok_b {
                r.bad += 1;
            }
            match (ok_a, ok_b) {
                (true, false) if copy != 0 => {
                    if self.heal && !self.read_only && self.dev.write_blocks(copy + off, pa.as_ref().unwrap()).is_ok() {
                        r.repaired += 1;
                    }
                }
                (false, true) => {
                    if self.heal && !self.read_only && self.dev.write_blocks(start + off, pb.as_ref().unwrap()).is_ok() {
                        r.repaired += 1;
                    }
                }
                (false, false) => {
                    r.lost += 1;
                    r.first_lost.get_or_insert(off);
                }
                _ => {}
            }
        }
        Ok(r)
    }

    /// Precte `plen` bloku komprimovanych dat (jeden soucet za cely usek).
    pub fn read_packed(&mut self, phys: u64, copy: u64, plen: u64, csum: crate::hash::Csum) -> Result<Vec<u8>> {
        if !self.inside(phys, plen) || (copy != 0 && !self.inside(copy, plen)) {
            return Err(Error::Corrupted);
        }
        let mut buf = vec![0u8; plen as usize * BLOCK_SIZE];
        self.read_verified(phys, copy, &mut buf, |a, b| a.sum(b) == csum)?;
        Ok(buf)
    }

    pub fn mark_changed(&mut self) {
        self.changed = true;
    }

    pub fn has_changes(&self) -> bool {
        self.changed || !self.dirty.is_empty()
    }

    /// Zapise rozpracovane uzly stromu s korenem `root` (potomci pred
    /// rodici, do souvislych useku); vraci ukazatel na novy koren.
    pub fn write_tree(&mut self, root: Child) -> Result<BlockPtr> {
        let Child::Mem(root_id) = root else {
            let Child::Disk(p) = root else { unreachable!() };
            return Ok(p);
        };
        // Poradi zapisu: post-order (potomci drive), jen dosazitelne uzly.
        let mut order = Vec::new();
        let mut stack = vec![(root_id, false)];
        while let Some((id, expanded)) = stack.pop() {
            if expanded {
                order.push(id);
                continue;
            }
            stack.push((id, true));
            if let Body::Internal(children) = &self.dirty[&id].body {
                for (_, c) in children.iter().rev() {
                    if let Child::Mem(cid) = c {
                        stack.push((*cid, false));
                    }
                }
            }
        }
        // Kazdy uzel = NODE_BLOCKS souvislych bloku; useky od alokatoru se
        // pouziji po celych uzlech, zbytek se hned vrati.
        let mut starts = Vec::with_capacity(order.len());
        while starts.len() < order.len() {
            let want = (order.len() - starts.len()) as u64 * NODE_BLOCKS;
            // Jen useky aspon na jeden cely uzel (jinak by se smycka
            // na rozdrobenem disku tocila donekonecna).
            let (s, l) = self.space.alloc_min(want, NODE_BLOCKS, Class::Meta, self.txg)?;
            let usable = l - l % NODE_BLOCKS;
            if usable < l {
                self.space.free(s + usable, l - usable, self.txg, true);
            }
            starts.extend((0..usable / NODE_BLOCKS).map(|k| s + k * NODE_BLOCKS));
        }
        // Druhe kopie: od konce disku (daleko od prvnich, pri vic AG v jine
        // skupine). Na disku bez souvisleho mista pro kopii se zapise jen
        // prvni - radsi bez kopie nez selhany commit; fsck to hlasi.
        let mut copies: Vec<u64> = Vec::with_capacity(order.len());
        while copies.len() < order.len() {
            let want = (order.len() - copies.len()) as u64 * NODE_BLOCKS;
            let Ok((s, l)) = self.space.alloc_min(want, NODE_BLOCKS, Class::MetaHigh, self.txg) else {
                break;
            };
            let usable = l - l % NODE_BLOCKS;
            if usable < l {
                self.space.free(s + usable, l - usable, self.txg, true);
            }
            copies.extend((0..usable / NODE_BLOCKS).map(|k| s + k * NODE_BLOCKS));
        }
        let mut ptrs: BTreeMap<u64, BlockPtr> = BTreeMap::new();
        let mut out: Vec<(u64, Vec<u8>)> = Vec::with_capacity(order.len() * 2);
        for (i, (&id, &block)) in order.iter().zip(&starts).enumerate() {
            let mut node = self.dirty.remove(&id).unwrap();
            if let Body::Internal(children) = &mut node.body {
                for (_, c) in children.iter_mut() {
                    if let Child::Mem(cid) = c {
                        *c = Child::Disk(ptrs[cid]);
                    }
                }
            }
            let data = node.encode();
            let copy = copies.get(i).copied().unwrap_or(0);
            let ptr = BlockPtr {
                block,
                birth: self.txg,
                csum: self.algo.sum(&data),
                len: NODE_BLOCKS as u32,
                copy,
                ..BlockPtr::default()
            };
            ptrs.insert(id, ptr);
            if self.cache.len() < CACHE_LIMIT {
                self.cache.insert(block, node);
            }
            if copy != 0 {
                out.push((copy, data.clone()));
            }
            out.push((block, data));
        }
        write_runs(&mut self.dev, &mut out)?;
        Ok(ptrs[&root_id])
    }

    /// Zapise stav (strom, misto, superblok). Data souboru a deadlisty uz
    /// musi byt ve strome (zarizuje `Fs::commit`).
    pub fn commit(&mut self) -> Result<()> {
        self.check_writable()?;
        if !self.has_changes() && self.sb.generation > 0 {
            return Ok(());
        }
        // Poskozena hodnota z disku vedla k neplatnemu uvolneni: neukladat
        // nic, co by poskozeni zapsalo natrvalo.
        if self.space.corrupt {
            self.poisoned = true;
            return Err(Error::Corrupted);
        }
        let r = self.commit_inner();
        if r.is_err() {
            self.poisoned = true;
        }
        r
    }

    fn commit_inner(&mut self) -> Result<()> {
        let txg = self.txg;
        if !self.stale_slots.is_empty() {
            let zero = vec![0u8; BLOCK_SIZE];
            for &slot in &self.stale_slots {
                self.dev.write_block(slot, &zero)?;
            }
            self.dev.flush()?;
            self.stale_slots.clear();
        }
        let root = self.write_tree(self.root)?;
        self.root = Child::Disk(root);
        debug_assert!(self.dirty.is_empty(), "rozpracovany uzel mimo strom");
        self.dirty.clear();
        // Bloky dosavadni epochy logu: po tomto commitu je nic nepotrebuje.
        // Uvolnit odlozene a uz v tomto commitu (zaznam musi byt v nem, jinak
        // by po vypadku zustaly obsazene); hned je pouzit nejde - do zapisu
        // superbloku je jeste muze cist prehravani.
        for b in core::mem::take(&mut self.zil_blocks) {
            self.space.free(b, 1, txg, false);
        }
        // Nova epocha intent logu: prvni blok predem (zapise se do space logu).
        let zil_block = self.space.alloc(1, None, Class::Meta, txg)?.0;
        let zil_seed = u64::from_le_bytes(
            xxh3(&[&txg.to_le_bytes()[..], &root.csum.0[..]].concat()).0[..8]
                .try_into()
                .unwrap(),
        );
        let space = self.space.commit(&mut self.dev, self.algo, txg)?;
        self.dev.flush()?;
        let sb = Superblock {
            generation: txg,
            root,
            space,
            zil_block,
            zil_seed,
            next_oid: self.next_oid,
            next_snapshot: self.next_snapshot,
            snapshot_generation: self.snapshot_generation,
            dead_seq: self.dead_seq,
            ..self.sb.clone()
        };
        self.dev
            .write_block(SB_FIRST + txg % SB_COUNT, &sb.encode())?;
        self.dev.flush()?;
        // Ulozeno.
        self.sb = sb;
        self.txg += 1;
        self.changed = false;
        self.zil_blocks.push(zil_block);
        self.zil_logged.clear();
        self.zil_next = zil_block;
        self.zil_seq = 0;
        self.space.begin_txg(self.txg);
        let discards = self.space.take_discards();
        if self.discard {
            for (s, l) in discards {
                // TRIM je jen napoveda; chyba neni fatalni.
                let _ = self.dev.discard(s, l);
            }
        }
        Ok(())
    }
}

/// Zapise bloky, souvisle useky jednim pozadavkem.
pub fn write_runs<D: BlockDevice>(dev: &mut D, out: &mut [(u64, Vec<u8>)]) -> Result<()> {
    out.sort_unstable_by_key(|(b, _)| *b);
    let mut i = 0;
    while i < out.len() {
        let mut j = i + 1;
        while j < out.len()
            && out[j].0 == out[j - 1].0 + (out[j - 1].1.len() / BLOCK_SIZE) as u64
            && j - i < 64
        {
            j += 1;
        }
        if j - i == 1 {
            dev.write_block(out[i].0, &out[i].1)?;
        } else {
            let buf = out[i..j].iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>().concat();
            dev.write_blocks(out[i].0, &buf)?;
        }
        i = j;
    }
    Ok(())
}
