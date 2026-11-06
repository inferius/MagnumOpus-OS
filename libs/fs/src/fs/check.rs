//! Kontrola konzistence (fsck) a overeni dat (scrub).
//!
//! Projde zivy strom, vsechny snapshoty, metadata volneho mista a intent log
//! a overi:
//! - kontrolni soucty vsech uzlu (a pri `scrub` i vsech datovych bloku),
//! - ze kazdy pouzity blok je podle alokatoru obsazeny,
//! - ze zadny obsazeny blok neni "ztraceny" (nikdo na nej neukazuje),
//! - ze zivy strom nepouziva jeden blok dvakrat.

use alloc::collections::{BTreeMap, BTreeSet};
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
    /// Nalezene problemy (prazdne = v poradku).
    pub problems: Vec<String>,
    /// Ktera generace (ulozeny stav na disku) se kontrolovala.
    pub generation: u64,
    /// V pameti jsou zmeny, ktere kontrola nezahrnula (svazek jen pro cteni
    /// s prehranym intent logem): kontrolovan je jen ulozeny stav.
    pub live_not_checked: bool,
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
        // blok -> kdo ho pouziva (pro hlaseni)
        let mut used: BTreeMap<u64, &'static str> = BTreeMap::new();
        let mut seen_nodes: BTreeSet<u64> = BTreeSet::new();
        // Druhy objektu zive verze (shoda polozek adresaru se zaznamy).
        let mut objects = Objects::default();
        let stored = Child::Disk(sb.root);
        let mut roots = alloc::vec![(String::from("ulozeny stav"), stored)];
        roots.extend(self.snapshot_roots_of(&stored)?);
        for (index, (name, root)) in roots.into_iter().enumerate() {
            let live = index == 0;
            let mut stack = alloc::vec![root];
            while let Some(c) = stack.pop() {
                let Child::Disk(p) = c else { continue };
                if !seen_nodes.insert(p.block) {
                    if live {
                        r.problems
                            .push(format!("uzel {} je ve zivem strome vickrat", p.block));
                    }
                    continue;
                }
                for b in p.block..p.block + crate::layout::NODE_BLOCKS {
                    if used.insert(b, "uzel").is_some() {
                        r.problems.push(format!("blok {b} pouziva uzel i neco jineho"));
                    }
                }
                r.tree_blocks += crate::layout::NODE_BLOCKS;
                let node = match self.s.load(&p) {
                    Ok(n) => n.clone(),
                    Err(e) => {
                        r.problems
                            .push(format!("{name}: uzel {} nejde nacist: {e:?}", p.block));
                        continue;
                    }
                };
                match node.body {
                    Body::Internal(children) => stack.extend(children.iter().map(|(_, c)| *c)),
                    Body::Leaf(items) => {
                        for (k, v) in items {
                            if live {
                                objects.note(&k, &v);
                            }
                            if k.kind != item::EXTENT {
                                continue;
                            }
                            let e = Extent::decode(&v)?;
                            for b in e.phys..e.phys + e.len() {
                                match used.insert(b, "data") {
                                    None => r.data_blocks += 1,
                                    Some("data") if !live => {}
                                    Some(_) => r
                                        .problems
                                        .push(format!("datovy blok {b} je pouzity vickrat")),
                                }
                            }
                            if scrub {
                                let mut buf = alloc::vec![0u8; e.len() as usize * BLOCK_SIZE];
                                if let Err(err) = self.s.read_data(e.phys, &e.csums, &mut buf) {
                                    r.problems.push(format!(
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
        objects.verify(&mut r.problems);
        meta.extend(log_blocks);
        for b in meta {
            if used.insert(b, "metadata mista").is_some() {
                r.problems
                    .push(format!("blok metadat {b} je pouzity i jinde"));
            }
            r.meta_blocks += 1;
        }
        // Vse pouzite musi byt obsazene, vse obsazene pouzite.
        free.sort_unstable();
        let is_free = |b: u64| {
            let i = free.partition_point(|&(s, _)| s <= b);
            i > 0 && b < free[i - 1].0 + free[i - 1].1
        };
        for (&b, what) in &used {
            if is_free(b) {
                r.problems
                    .push(format!("blok {b} ({what}) je oznaceny jako volny"));
            }
        }
        let mut leaked = 0u64;
        let mut first_leak = None;
        for b in DATA_START..self.s.sb.block_count {
            if !used.contains_key(&b) && !is_free(b) {
                leaked += 1;
                first_leak.get_or_insert(b);
            }
        }
        if leaked > 0 {
            r.problems.push(format!(
                "{leaked} ztracenych bloku (prvni {})",
                first_leak.unwrap()
            ));
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
    fn note(&mut self, k: &Key, v: &[u8]) {
        match k.kind {
            item::INODE => {
                if let Ok(i) = Inode::decode(v) {
                    self.inodes.insert(k.oid, (i.kind, i.size));
                }
            }
            item::DIRENT => {
                if let Ok(list) = decode_dirents(v) {
                    self.dirents.extend(list.into_iter().map(|d| (d.oid, d.kind)));
                }
            }
            item::SYMLINK => {
                self.links.insert(k.oid, v.len() as u64);
            }
            _ => {}
        }
    }

    fn verify(&self, problems: &mut Vec<String>) {
        for &(oid, kind) in &self.dirents {
            match self.inodes.get(&oid) {
                None => problems.push(format!("polozka adresare ukazuje na neexistujici objekt {oid}")),
                Some(&(k, _)) if k != kind => {
                    problems.push(format!("objekt {oid}: druh v adresari {kind}, v zaznamu {k}"))
                }
                _ => {}
            }
        }
        for (&oid, &(kind, size)) in &self.inodes {
            let link = self.links.get(&oid);
            if kind == KIND_SYMLINK && link != Some(&size) {
                problems.push(format!("odkaz {oid}: cil chybi nebo nesedi delka"));
            }
            if kind != KIND_SYMLINK && link.is_some() {
                problems.push(format!("objekt {oid} neni odkaz, ale ma cil"));
            }
        }
    }
}
