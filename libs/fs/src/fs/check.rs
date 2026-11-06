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
use crate::layout::{BLOCK_SIZE, DATA_START, Extent, item};
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
}

impl<D: BlockDevice> Fs<D> {
    /// Ulozi rozpracovane zmeny a zkontroluje konzistenci. `scrub` = overit
    /// i kontrolni soucty vsech datovych bloku (cte cely obsah).
    pub fn check(&mut self, scrub: bool) -> Result<CheckReport> {
        self.commit()?;
        let mut r = CheckReport::default();
        // blok -> kdo ho pouziva (pro hlaseni)
        let mut used: BTreeMap<u64, &'static str> = BTreeMap::new();
        let mut seen_nodes: BTreeSet<u64> = BTreeSet::new();
        let mut roots = alloc::vec![(String::from("zivy stav"), self.s.root)];
        roots.extend(self.snapshot_roots()?);
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
                if used.insert(p.block, "uzel").is_some() {
                    r.problems
                        .push(format!("blok {} pouziva uzel i neco jineho", p.block));
                }
                r.tree_blocks += 1;
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
        let mut meta = self.s.space.meta.blocks();
        meta.extend(self.s.zil_blocks.iter().copied());
        for b in meta {
            if used.insert(b, "metadata mista").is_some() {
                r.problems
                    .push(format!("blok metadat {b} je pouzity i jinde"));
            }
            r.meta_blocks += 1;
        }
        // Vse pouzite musi byt obsazene, vse obsazene pouzite.
        let mut free = self.s.space.free_extents();
        free.extend(self.s.space.deferred_list());
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
