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
    BLOCK_SIZE, DATA_START, Extent, Inode, KIND_SYMLINK, Key, decode_dirents, item,
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
                if seen_nodes.set(p.block) {
                    if live {
                        r.problem(format!("uzel {} je ve zivem strome vickrat", p.block));
                    }
                    continue;
                }
                for b in p.block..p.block + crate::layout::NODE_BLOCKS {
                    if used.set(b) {
                        r.problem(format!("blok {b} pouziva uzel i neco jineho"));
                    }
                }
                r.tree_blocks += crate::layout::NODE_BLOCKS;
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
                                objects[g].note(&k, &v, &mut r);
                            }
                            if k.kind != item::EXTENT {
                                continue;
                            }
                            let e = match Extent::decode(&v) {
                                Ok(e) => e,
                                Err(err) => {
                                    r.problem(format!("{name}: extent objektu {}: {err:?}", k.oid));
                                    continue;
                                }
                            };
                            let end = e.phys.checked_add(e.len());
                            if e.phys < DATA_START || end.is_none_or(|x| x > n) {
                                r.problem(format!("{name}: extent objektu {} lezi mimo svazek", k.oid));
                                continue;
                            }
                            for b in e.phys..e.phys + e.len() {
                                if !used.set(b) {
                                    is_data.set(b);
                                    r.data_blocks += 1;
                                } else if !(is_data.get(b) && !live) {
                                    // Sdilena data snapshotu jsou v poradku;
                                    // jinak je blok pouzity dvakrat.
                                    r.problem(format!("datovy blok {b} je pouzity vickrat"));
                                }
                            }
                            if scrub {
                                let mut buf = alloc::vec![0u8; e.len() as usize * BLOCK_SIZE];
                                if let Err(err) = self.s.read_data(e.phys, &e.csums, &mut buf) {
                                    r.problem(format!(
                                        "{name}: data objektu {} od bloku {}: {err:?}",
                                        k.oid, e.phys
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        for o in &objects {
            o.verify(&mut r);
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

/// Sber druhu objektu ze zive verze pro kontrolu, ze polozky adresaru,
/// zaznamy objektu a cile odkazu k sobe sedi.
#[derive(Default)]
struct Objects {
    /// oid -> (druh, velikost) ze zaznamu objektu.
    inodes: BTreeMap<u64, (u8, u64)>,
    /// (oid, druh) z polozek adresaru.
    dirents: Vec<(u64, u8)>,
    /// oid -> delka cile odkazu.
    links: BTreeMap<u64, u64>,
}

impl Objects {
    /// Zaznamena polozku; poskozeny zaznam objektu nebo polozku adresare
    /// nahlasi (drive se tise preskocily - kontrola pak hlasila cisty svazek,
    /// prestoze `stat` vracel CORRUPTED).
    fn note(&mut self, k: &Key, v: &[u8], r: &mut CheckReport) {
        match k.kind {
            item::INODE => match Inode::decode(v) {
                Ok(i) => {
                    self.inodes.insert(k.oid, (i.kind, i.size));
                }
                Err(e) => r.problem(format!("zaznam objektu {} je poskozeny: {e:?}", k.oid)),
            },
            item::DIRENT => match decode_dirents(v) {
                Ok(list) => {
                    for d in &list {
                        if !crate::names::valid_stored(&d.name) {
                            r.problem(format!("adresar {}: neplatne jmeno polozky", k.oid));
                        }
                    }
                    // Polozka datasetu ukazuje na jiny strom (cislo datasetu).
                    self.dirents.extend(
                        list.into_iter()
                            .filter(|d| d.kind != crate::layout::KIND_DATASET)
                            .map(|d| (d.oid, d.kind)),
                    );
                }
                Err(e) => r.problem(format!("polozka adresare {} je poskozena: {e:?}", k.oid)),
            },
            item::SYMLINK => {
                self.links.insert(k.oid, v.len() as u64);
            }
            _ => {}
        }
    }

    fn verify(&self, r: &mut CheckReport) {
        for &(oid, kind) in &self.dirents {
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
