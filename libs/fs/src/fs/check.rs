//! Kontrola konzistence (fsck) a overeni dat (scrub).
//!
//! Projde zivy strom, vsechny snapshoty, metadata volneho mista a intent log
//! a overi:
//! - kontrolni soucty vsech uzlu (a pri `scrub` i vsech datovych bloku),
//! - ze kazdy pouzity blok je podle alokatoru obsazeny,
//! - ze zadny obsazeny blok neni "ztraceny" (nikdo na nej neukazuje),
//! - ze zivy strom nepouziva jeden blok dvakrat.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::Result;

use super::Fs;
use crate::device::BlockDevice;
use crate::layout::{
    DATA_START, Extent, Inode, KIND_SYMLINK, Key, decode_dirents, item,
};
use crate::node::{Body, Child};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckReport {
    /// Bloky uzlu stromu (vsech verzi).
    pub tree_blocks: u64,
    /// Bloky dat (vsech verzi).
    pub data_blocks: u64,
    /// Bloky metadat volneho mista a intent logu.
    pub meta_blocks: u64,
    /// Uzly bez druhe kopie (zapsane na disk bez souvisleho mista pro ni) -
    /// neni chyba, jen mensi odolnost.
    pub nodes_without_copy: u64,
    /// Zaznamy tabulky sdileni (sdilene useky) a jejich bloky.
    pub shared_ranges: u64,
    pub shared_blocks: u64,
    /// Nalezene problemy (prazdne = v poradku); nejvys `MAX_PROBLEMS`
    /// popisu, celkovy pocet je v `problems_total`.
    pub problems: Vec<String>,
    /// Pocet vsech nalezenych problemu (i tech, ktere se uz nevypsaly).
    pub problems_total: u64,
    /// Ktera generace (ulozeny stav na disku) se kontrolovala.
    pub generation: u64,
    /// V pameti jsou zmeny, ktere kontrola nezahrnula (svazek jen pro cteni
    /// s prehranym intent logem): kontrolovan je jen ulozeny stav.
    pub live_not_checked: bool,
}

/// Nejvic vypsanych problemu (pamet kontroly nesmi rust s poskozenim).
pub const MAX_PROBLEMS: usize = 1000;

impl CheckReport {
    fn problem(&mut self, msg: String) {
        self.problems_total += 1;
        if self.problems.len() < MAX_PROBLEMS {
            self.problems.push(msg);
        }
    }
}

/// Bitova mapa bloku svazku.
struct Bitmap(Vec<u64>);

impl Bitmap {
    fn new(blocks: u64) -> Self {
        Self(alloc::vec![0; blocks.div_ceil(64) as usize])
    }

    fn get(&self, b: u64) -> bool {
        self.0[(b / 64) as usize] & (1 << (b % 64)) != 0
    }

    /// Nastavi bit; vraci, zda uz byl nastaveny.
    fn set(&mut self, b: u64) -> bool {
        let w = &mut self.0[(b / 64) as usize];
        let was = *w & (1 << (b % 64)) != 0;
        *w |= 1 << (b % 64);
        was
    }
}

impl<D: BlockDevice> Fs<D> {
    /// Zkontroluje konzistenci **ulozeneho stavu** (posledni commit na
    /// disku). `scrub` = overit i kontrolni soucty vsech datovych bloku
    /// (cte cely obsah).
    ///
    /// - Zapisovatelny svazek: nejdriv commit, takze ulozeny stav = zivy.
    /// - Jen pro cteni (nejista obnova, zarizeni bez flush): nic se
    ///   nezapisuje; kontroluje se ulozena generace a misto nactene znovu
    ///   z disku. Zmeny prehrane z logu jen v pameti zkontrolovane nejsou -
    ///   rika to `live_not_checked`.
    pub fn check(&mut self, scrub: bool) -> Result<CheckReport> {
        self.require_system()?;
        let read_only = self.s.read_only;
        if !read_only {
            self.commit()?;
        }
        let sb = self.s.sb.clone();
        // Misto podle disku (u zapisovatelneho po commitu totez co v pameti).
        let (mut meta, mut free) = if read_only {
            let space = crate::space::Space::load(
                &mut self.s.dev,
                sb.algo,
                sb.block_count,
                sb.ag_blocks,
                &sb.space,
                sb.generation + 1,
            )?;
            let mut free = space.free_extents();
            free.extend(space.deferred_list());
            (space.meta.blocks(), free)
        } else {
            let mut free = self.s.space.free_extents();
            free.extend(self.s.space.deferred_list());
            (self.s.space.meta.blocks(), free)
        };
        let log_blocks: Vec<u64> = if read_only {
            (sb.zil_block != 0).then_some(sb.zil_block).into_iter().collect()
        } else {
            self.s.zil_blocks.clone()
        };
        let mut r = CheckReport {
            generation: sb.generation,
            live_not_checked: read_only && (self.replayed > 0 || self.s.has_changes()),
            ..CheckReport::default()
        };
        // Pamet kontroly je pevna: 3 bity na blok svazku (1 TiB = 96 MiB),
        // nezavisle na poctu souboru a verzi.
        let n = sb.block_count;
        let mut used = Bitmap::new(n);
        let mut is_data = Bitmap::new(n);
        let mut seen_nodes = Bitmap::new(n);
        // Druhy objektu zive verze (shoda polozek adresaru se zaznamy).
        // Kazdy dataset ma vlastni cisla objektu: kontrola objektu zvlast.
        let mut objects: Vec<Objects> = alloc::vec![Objects::default()];
        let tf = self.s.time_format();
        let stored = Child::Disk(sb.root);
        // (popis, koren, index skupiny objektu u zive verze datasetu)
        let mut roots: Vec<(String, Child, Option<usize>)> =
            alloc::vec![(String::from("ulozeny stav"), stored, Some(0))];
        roots.extend(self.snapshot_roots_of(&stored)?.into_iter().map(|(n, c)| (n, c, None)));
        // Ostatni datasety (zaznamy v ulozenem korenovem stromu) a jejich
        // snapshoty. Klon sdili bloky s puvodnim snapshotem: data se
        // opakuji legitimne, prisna kontrola dvojiho pouziti jen u korene.
        let records = crate::btree::collect(
            &mut self.s,
            &stored,
            &Key::new(crate::layout::META_OID, item::DATASET, 0),
            &Key::new(crate::layout::META_OID, item::DATASET + 1, 0),
        )?;
        for (k, v) in records {
            let rec = match crate::layout::DatasetRecord::decode(&v) {
                Ok(rec) => rec,
                Err(e) => {
                    r.problem(format!("zaznam datasetu {} je poskozeny: {e:?}", k.offset));
                    continue;
                }
            };
            let ds_root = Child::Disk(rec.root);
            objects.push(Objects::default());
            roots.push((format!("dataset {}", k.offset), ds_root, Some(objects.len() - 1)));
            let snaps = self.snapshot_roots_of(&ds_root)?;
            roots.extend(snaps.into_iter().map(|(n, c)| (format!("dataset {}: {n}", k.offset), c, None)));
        }
        // Tabulka sdileni a index deduplikace (ulozeny korenovy strom).
        let share = self.check_share_table(&stored, n, &mut r)?;
        let shared = |b: u64| {
            let i = share.partition_point(|&(s, _)| s <= b);
            i > 0 && b < share[i - 1].0 + share[i - 1].1
        };
        for (index, (name, root, group)) in roots.into_iter().enumerate() {
            let live = index == 0;
            let mut stack = alloc::vec![root];
            while let Some(c) = stack.pop() {
                let Child::Disk(p) = c else { continue };
                let end = p.block.checked_add(crate::layout::NODE_BLOCKS);
                if p.block < DATA_START || end.is_none_or(|e| e > n) {
                    r.problem(format!("{name}: uzel {} lezi mimo svazek", p.block));
                    continue;
                }
                // Uzel uz videny v jine verzi (sdileny se snapshotem): bloky
                // uz jsou zapocitane. Zivy strom datasetu (napr. klon) ho
                // ale musi projit kvuli objektum - jinak by objekty
                // a zaznamy zurnalu ve sdilenych listech chybely.
                let node_shared = seen_nodes.set(p.block);
                if node_shared {
                    if live {
                        r.problem(format!("uzel {} je ve zivem strome vickrat", p.block));
                        continue;
                    }
                    if group.is_none() {
                        continue;
                    }
                } else {
                    for b in p.block..p.block + crate::layout::NODE_BLOCKS {
                        if used.set(b) {
                            r.problem(format!("blok {b} pouziva uzel i neco jineho"));
                        }
                    }
                    r.tree_blocks += crate::layout::NODE_BLOCKS;
                    if p.copy == 0 {
                        r.nodes_without_copy += 1;
                    } else if p.copy < DATA_START || p.copy.checked_add(crate::layout::NODE_BLOCKS).is_none_or(|e| e > n) {
                        r.problem(format!("{name}: kopie uzlu {} lezi mimo svazek", p.block));
                    } else {
                        for b in p.copy..p.copy + crate::layout::NODE_BLOCKS {
                            if used.set(b) {
                                r.problem(format!("blok {b} pouziva kopie uzlu i neco jineho"));
                            }
                        }
                        r.tree_blocks += crate::layout::NODE_BLOCKS;
                    }
                }
                let node = match self.s.load(&p) {
                    Ok(n) => n.clone(),
                    Err(e) => {
                        r.problem(format!("{name}: uzel {} nejde nacist: {e:?}", p.block));
                        continue;
                    }
                };
                match node.body {
                    Body::Internal(children) => stack.extend(children.iter().map(|(_, c)| *c)),
                    Body::Leaf(items) => {
                        for (k, v) in items {
                            if let Some(g) = group {
                                objects[g].note(&k, &v, tf, &mut r);
                            }
                            if node_shared || k.kind != item::EXTENT {
                                continue;
                            }
                            let e = match Extent::decode(&v) {
                                Ok(e) => e,
                                Err(err) => {
                                    r.problem(format!("{name}: extent objektu {}: {err:?}", k.oid));
                                    continue;
                                }
                            };
                            let within = |p: u64| p >= DATA_START && p.checked_add(e.plen()).is_some_and(|x| x <= n);
                            if !within(e.phys) || (e.copy != 0 && !within(e.copy)) {
                                r.problem(format!("{name}: extent objektu {} lezi mimo svazek", k.oid));
                                continue;
                            }
                            let ranges = [(e.phys, true), (e.copy, e.copy != 0)];
                            for (start, present) in ranges {
                                if !present {
                                    continue;
                                }
                                for b in start..start + e.plen() {
                                    if !used.set(b) {
                                        is_data.set(b);
                                        r.data_blocks += 1;
                                    } else if !(is_data.get(b) && (!live || shared(b))) {
                                        // Data snapshotu a sdilene useky (tabulka
                                        // sdileni) se opakuji legitimne; jinak je
                                        // blok pouzity dvakrat.
                                        r.problem(format!("datovy blok {b} je pouzity vickrat"));
                                    }
                                }
                            }
                            if e.is_shared() && !shared(e.phys) && live {
                                r.problem(format!("{name}: sdileny extent objektu {} chybi v tabulce sdileni", k.oid));
                            }
                            if scrub
                                && !e.is_unwritten()
                                && let Err(err) = self.extent_data(&e, 0, e.len())
                            {
                                r.problem(format!("{name}: data objektu {} od bloku {}: {err:?}", k.oid, e.phys));
                            }
                        }
                    }
                }
            }
        }
        for o in &objects {
            o.verify(&mut r);
        }
        // Zaznam tabulky sdileni bez pouzitych bloku = ztracena reference
        // (uvolneni ji nesnizilo).
        for &(s, l) in &share {
            if (s..s + l).any(|b| !used.get(b)) {
                r.problem(format!("sdileny usek {s}+{l} nikdo nepouziva"));
            }
        }
        meta.extend(log_blocks);
        for b in meta {
            if b >= n {
                r.problem(format!("blok metadat {b} lezi mimo svazek"));
            } else if used.set(b) {
                r.problem(format!("blok metadat {b} je pouzity i jinde"));
            }
            r.meta_blocks += 1;
        }
        // Vse pouzite musi byt obsazene, vse obsazene pouzite. Jeden pruchod
        // bloky soubezne se serazenym seznamem volnych useku.
        free.sort_unstable();
        let mut leaked = 0u64;
        let mut first_leak = None;
        let mut fi = 0;
        for b in DATA_START..n {
            while fi < free.len() && free[fi].0 + free[fi].1 <= b {
                fi += 1;
            }
            let is_free = fi < free.len() && free[fi].0 <= b;
            match (used.get(b), is_free) {
                (true, true) => r.problem(format!("blok {b} je pouzity, ale oznaceny jako volny")),
                (false, false) => {
                    leaked += 1;
                    first_leak.get_or_insert(b);
                }
                _ => {}
            }
        }
        if leaked > 0 {
            r.problem(format!("{leaked} ztracenych bloku (prvni {})", first_leak.unwrap()));
        }
        Ok(r)
    }
}

const META: u64 = crate::layout::META_OID;

impl<D: BlockDevice> Fs<D> {
    /// Projde tabulku sdileni a index deduplikace ve strome `root`: zaznamy
    /// se neprekryvaji, lezi ve svazku, indexovane maji polozku indexu
    /// a naopak. Vraci serazene useky (zacatek, delka) vcetne kopii.
    fn check_share_table(&mut self, root: &Child, n: u64, r: &mut CheckReport) -> Result<Vec<(u64, u64)>> {
        use crate::datarec::{ShareEntry, decode_dedup, dedup_key};
        let items = crate::btree::collect(
            &mut self.s,
            root,
            &Key::new(META, item::SHARE, 0),
            &Key::new(META, item::SHARE + 1, 0),
        )?;
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut last_end = 0u64;
        let mut indexed: Vec<(crate::hash::Csum, u64)> = Vec::new();
        for (k, v) in items {
            let e = match ShareEntry::decode(&v) {
                Ok(e) => e,
                Err(err) => {
                    r.problem(format!("zaznam sdileni {}: {err:?}", k.offset));
                    continue;
                }
            };
            let len = e.len as u64;
            let inside = |p: u64| p >= DATA_START && p.checked_add(len).is_some_and(|x| x <= n);
            if !inside(k.offset) || (e.copy != 0 && !inside(e.copy)) || len > crate::layout::MAX_EXTENT {
                r.problem(format!("zaznam sdileni {} lezi mimo svazek", k.offset));
                continue;
            }
            if k.offset < last_end {
                r.problem(format!("zaznamy sdileni se prekryvaji u bloku {}", k.offset));
            }
            last_end = k.offset + len;
            r.shared_ranges += 1;
            r.shared_blocks += len;
            out.push((k.offset, len));
            if e.copy != 0 {
                out.push((e.copy, len));
            }
            if let Some(h) = e.hash {
                indexed.push((h, k.offset));
            }
        }
        out.sort_unstable();
        // Index: kazda polozka ukazuje na indexovany zaznam se stejnym
        // hashem a kazdy indexovany zaznam je v indexu.
        let index = crate::btree::collect(
            &mut self.s,
            root,
            &Key::new(META, item::DEDUP, 0),
            &Key::new(META, item::DEDUP + 1, 0),
        )?;
        let mut listed: Vec<(crate::hash::Csum, u64)> = Vec::new();
        for (k, v) in index {
            match decode_dedup(&v) {
                Ok(list) => {
                    for (h, s) in list {
                        if dedup_key(&h) != k.offset {
                            r.problem(format!("index deduplikace: polozka {} ma cizi hash", k.offset));
                        }
                        listed.push((h, s));
                    }
                }
                Err(err) => r.problem(format!("index deduplikace {}: {err:?}", k.offset)),
            }
        }
        indexed.sort_unstable();
        listed.sort_unstable();
        if indexed != listed {
            r.problem(format!(
                "index deduplikace nesedi s tabulkou sdileni ({} indexovanych, {} v indexu)",
                indexed.len(),
                listed.len()
            ));
        }
        Ok(out)
    }
}

impl Objects {
    /// Kontroly struktur historie (faze 2).
    fn verify_history(&self, r: &mut CheckReport) {
        // Zurnal: souvisla rada first..next, stav musi existovat.
        match self.jstate {
            None if !self.journal.is_empty() => {
                r.problem(format!("zurnal ma {} zaznamu, ale chybi stav", self.journal.len()))
            }
            None => {}
            Some(st) => {
                // Pruchod stromu neni serazeny: seradit.
                let mut seqs = self.journal.clone();
                seqs.sort_unstable();
                let ok = seqs.len() as u64 == st.next - st.first
                    && seqs.iter().enumerate().all(|(i, &q)| q == st.first + i as u64);
                if !ok {
                    r.problem(format!("zurnal: zaznamy nesedi se stavem {}..{}", st.first, st.next));
                }
            }
        }
        for m in &self.snapmeta {
            if !self.snapshots.contains(m) {
                r.problem(format!("metadata snapshotu {m} bez snapshotu"));
            }
        }
        // Kos: zaznam <-> polozka adresare kose.
        for (&id, &oid) in &self.trash {
            if !self.trash_dir.iter().any(|(n, o)| *o == oid && n.parse::<u64>().ok() == Some(id)) {
                r.problem(format!("zaznam kose {id}: objekt {oid} v kosi neni"));
            }
            if self.trash_next.is_none_or(|n| n <= id) {
                r.problem(format!("pocitadlo kose je mensi nez zaznam {id}"));
            }
        }
        for (n, oid) in &self.trash_dir {
            let id = n.parse::<u64>().unwrap_or(0);
            if self.trash.get(&id) != Some(oid) {
                r.problem(format!("objekt {oid} v kosi nema zaznam"));
            }
        }
        // Atributy: pocet v zaznamu objektu.
        for (&oid, &(_, count)) in &self.gens {
            let have = self.xattr_count.get(&oid).copied().unwrap_or(0);
            if have != count {
                r.problem(format!("objekt {oid}: {count} atributu v zaznamu, ulozeno {have}"));
            }
        }
        for &oid in self.xattr_count.keys() {
            if !self.gens.contains_key(&oid) {
                r.problem(format!("atributy neexistujiciho objektu {oid}"));
            }
        }
        // Expirace: atribut <-> index (oid, generace).
        let mut attr: Vec<(u64, u64)> =
            self.expires_attr.iter().map(|&o| (o, self.gens.get(&o).map_or(0, |g| g.0))).collect();
        let mut index = self.expires_index.clone();
        attr.sort_unstable();
        index.sort_unstable();
        if attr != index {
            r.problem(format!(
                "index expirace nesedi s atributy ({} v indexu, {} atributu)",
                index.len(),
                attr.len()
            ));
        }
        for oid in &self.lhash {
            if self.inodes.get(oid).is_none_or(|&(k, _)| k != crate::layout::KIND_FILE) {
                r.problem(format!("pamet hashe neexistujiciho souboru {oid}"));
            }
        }
    }
}

/// Sber druhu objektu ze zive verze pro kontrolu, ze polozky adresaru,
/// zaznamy objektu a cile odkazu k sobe sedi.
#[derive(Default)]
struct Objects {
    /// oid -> (druh, velikost) ze zaznamu objektu.
    inodes: BTreeMap<u64, (u8, u64)>,
    /// oid -> rodic ze zaznamu objektu.
    parents: BTreeMap<u64, u64>,
    /// (oid, druh, slozka) z polozek adresaru.
    dirents: Vec<(u64, u8, u64)>,
    /// oid -> delka cile odkazu.
    links: BTreeMap<u64, u64>,
    /// Popis opravneni -> kolik objektu na nej ukazuje.
    acl_uses: BTreeMap<u64, u64>,
    /// Popis opravneni -> (pocet odkazu v zaznamu, otisk).
    acls: BTreeMap<u64, (u64, u64)>,
    /// Index popisu: (otisk, cislo popisu).
    acl_index: Vec<(u64, u64)>,
    /// Historie (faze 2): generace a pocet atributu objektu.
    gens: BTreeMap<u64, (u64, u32)>,
    /// Zurnal: poradova cisla zaznamu a stav.
    journal: Vec<u64>,
    jstate: Option<crate::records::JournalState>,
    /// Snapshoty a jejich metadata (cisla).
    snapshots: Vec<u64>,
    snapmeta: Vec<u64>,
    /// Kos: zaznamy (cislo -> oid), pocitadlo, polozky adresare kose.
    trash: BTreeMap<u64, u64>,
    trash_next: Option<u64>,
    trash_dir: Vec<(String, u64)>,
    /// Atributy: oid -> pocet; expirace z atributu a z indexu.
    xattr_count: BTreeMap<u64, u32>,
    expires_attr: Vec<u64>,
    expires_index: Vec<(u64, u64)>,
    /// Pamet logickeho hashe (oid).
    lhash: Vec<u64>,
}

impl Objects {
    /// Zaznamena polozku; poskozeny zaznam objektu nebo polozku adresare
    /// nahlasi (drive se tise preskocily - kontrola pak hlasila cisty svazek,
    /// prestoze `stat` vracel CORRUPTED).
    fn note(&mut self, k: &Key, v: &[u8], tf: crate::layout::TimeFormat, r: &mut CheckReport) {
        match k.kind {
            item::INODE => match Inode::decode(v, tf) {
                Ok(i) => {
                    self.inodes.insert(k.oid, (i.kind, i.size));
                    self.parents.insert(k.oid, i.parent);
                    self.gens.insert(k.oid, (i.generation, i.xattrs));
                    if i.acl != 0 {
                        *self.acl_uses.entry(i.acl as u64).or_default() += 1;
                    }
                }
                Err(e) => r.problem(format!("zaznam objektu {} je poskozeny: {e:?}", k.oid)),
            },
            item::DIRENT => match decode_dirents(v) {
                Ok(list) => {
                    if k.oid == crate::layout::TRASH_OID {
                        self.trash_dir.extend(list.iter().map(|d| (d.name.clone(), d.oid)));
                    }
                    for d in &list {
                        if !crate::names::valid_stored(&d.name) {
                            r.problem(format!("adresar {}: neplatne jmeno polozky", k.oid));
                        }
                    }
                    // Polozka datasetu ukazuje na jiny strom (cislo datasetu).
                    self.dirents.extend(
                        list.into_iter()
                            .filter(|d| d.kind != crate::layout::KIND_DATASET)
                            .map(|d| (d.oid, d.kind, k.oid)),
                    );
                }
                Err(e) => r.problem(format!("polozka adresare {} je poskozena: {e:?}", k.oid)),
            },
            item::SYMLINK => {
                self.links.insert(k.oid, v.len() as u64);
            }
            item::ACL if k.oid == crate::layout::META_OID => match crate::perm::decode_acl(v) {
                Ok((a, refs)) => {
                    self.acls.insert(k.offset, (refs, crate::perm::acl_hash(&a)));
                }
                Err(e) => r.problem(format!("popis opravneni {} je poskozeny: {e:?}", k.offset)),
            },
            item::JOURNAL if k.oid == META => match crate::records::JournalEntry::decode(v) {
                Ok(_) => self.journal.push(k.offset),
                Err(e) => r.problem(format!("zaznam zurnalu {} je poskozeny: {e:?}", k.offset)),
            },
            item::JSTATE if k.oid == META => match crate::records::JournalState::decode(v) {
                Ok(st) => self.jstate = Some(st),
                Err(e) => r.problem(format!("stav zurnalu je poskozeny: {e:?}")),
            },
            item::SNAPSHOT if k.oid == META => self.snapshots.push(k.offset),
            item::SNAPMETA if k.oid == META => match crate::records::SnapMeta::decode(v) {
                Ok(_) => self.snapmeta.push(k.offset),
                Err(e) => r.problem(format!("metadata snapshotu {} jsou poskozena: {e:?}", k.offset)),
            },
            item::SETTINGS if k.oid == META => {
                if let Err(e) = crate::records::Settings::decode(v) {
                    r.problem(format!("nastaveni datasetu je poskozene: {e:?}"));
                }
            }
            item::DSLINK if k.oid == META => {
                if let Err(e) = crate::records::DsLink::decode(v) {
                    r.problem(format!("odkaz pripojeni datasetu je poskozeny: {e:?}"));
                }
            }
            item::HEAD if k.oid == META => {
                if let Err(e) = crate::records::Head::decode(v) {
                    r.problem(format!("zaznam HEAD je poskozeny: {e:?}"));
                }
            }
            item::BRANCH if k.oid == META => {
                if let Err(e) = crate::records::decode_branches(v) {
                    r.problem(format!("vetve jsou poskozene: {e:?}"));
                }
            }
            item::TRASH if k.oid == META && k.offset == 0 => {
                if v.len() == 8 {
                    self.trash_next = Some(crate::layout::u64_at(v, 0));
                } else {
                    r.problem(String::from("pocitadlo kose je poskozene"));
                }
            }
            item::TRASH if k.oid == META => match crate::records::TrashRecord::decode(v) {
                Ok(t) => {
                    self.trash.insert(k.offset, t.oid);
                }
                Err(e) => r.problem(format!("zaznam kose {} je poskozeny: {e:?}", k.offset)),
            },
            item::EXPIRY if k.oid == META => match crate::records::decode_pairs(v) {
                Ok(list) => self.expires_index.extend(list),
                Err(e) => r.problem(format!("index expirace je poskozeny: {e:?}")),
            },
            item::XATTR => match crate::records::decode_xattrs(v) {
                Ok(list) => {
                    for (n, val) in &list {
                        if crate::names::name_hash(n) != k.offset {
                            r.problem(format!("objekt {}: atribut {n} pod spatnym klicem", k.oid));
                        }
                        if n == crate::records::XATTR_EXPIRES {
                            if crate::records::decode_expires(val).is_err() {
                                r.problem(format!("objekt {}: neplatne datum expirace", k.oid));
                            }
                            self.expires_attr.push(k.oid);
                        }
                    }
                    *self.xattr_count.entry(k.oid).or_default() += list.len() as u32;
                }
                Err(e) => r.problem(format!("atributy objektu {} jsou poskozene: {e:?}", k.oid)),
            },
            item::LHASH => {
                if crate::records::LogicalHash::decode(v).is_err() {
                    r.problem(format!("pamet hashe objektu {} je poskozena", k.oid));
                }
                self.lhash.push(k.oid);
            }
            item::ACL_INDEX if k.oid == crate::layout::META_OID => {
                if v.is_empty() || v.len() % 8 != 0 {
                    r.problem(format!("index popisu opravneni {:#x} je poskozeny", k.offset));
                }
                for c in v.chunks_exact(8) {
                    self.acl_index.push((k.offset, u64::from_le_bytes(c.try_into().unwrap())));
                }
            }
            _ => {}
        }
    }

    fn verify(&self, r: &mut CheckReport) {
        self.verify_history(r);
        for (&id, &uses) in &self.acl_uses {
            match self.acls.get(&id) {
                None => r.problem(format!("objekty ukazuji na neexistujici popis opravneni {id}")),
                Some(&(refs, _)) if refs != uses => {
                    r.problem(format!("popis opravneni {id}: {refs} odkazu v zaznamu, {uses} objektu"))
                }
                _ => {}
            }
        }
        for (&id, &(_, hash)) in &self.acls {
            if !self.acl_uses.contains_key(&id) {
                r.problem(format!("popis opravneni {id} nema zadny objekt"));
            }
            if !self.acl_index.contains(&(hash, id)) {
                r.problem(format!("popis opravneni {id} chybi v indexu"));
            }
        }
        for &(hash, id) in &self.acl_index {
            if self.acls.get(&id).is_none_or(|&(_, h)| h != hash) {
                r.problem(format!("index popisu opravneni ukazuje na spatny popis {id}"));
            }
        }
        // Kazdy objekt ma nejvys jednu polozku adresare a jeho rodic je
        // slozka s ni (na tom stavi rozdil a slouceni - jmeno hledaji
        // v rodici).
        let mut named: BTreeMap<u64, u64> = BTreeMap::new();
        for &(oid, _, dir) in &self.dirents {
            if named.insert(oid, dir).is_some() {
                r.problem(format!("objekt {oid} ma vic polozek adresare"));
            }
            if let Some(&p) = self.parents.get(&oid)
                && p != dir
            {
                r.problem(format!("objekt {oid}: rodic v zaznamu {p}, polozka ve slozce {dir}"));
            }
        }
        for &(oid, kind, _) in &self.dirents {
            match self.inodes.get(&oid) {
                None => r.problem(format!("polozka adresare ukazuje na neexistujici objekt {oid}")),
                Some(&(k, _)) if k != kind => {
                    r.problem(format!("objekt {oid}: druh v adresari {kind}, v zaznamu {k}"))
                }
                _ => {}
            }
        }
        for (&oid, &(kind, size)) in &self.inodes {
            let link = self.links.get(&oid);
            if kind == KIND_SYMLINK && link != Some(&size) {
                r.problem(format!("odkaz {oid}: cil chybi nebo nesedi delka"));
            }
            if kind != KIND_SYMLINK && link.is_some() {
                r.problem(format!("objekt {oid} neni odkaz, ale ma cil"));
            }
        }
    }
}
