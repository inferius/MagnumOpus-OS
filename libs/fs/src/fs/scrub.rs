//! Scrub na pozadi (faze 3): overeni vsech ulozenych dat a metadat.
//!
//! Pruchod: datasety podle cisla, v kazdem snapshoty od nejstarsiho a pak
//! ulozena ziva verze. U kazdeho stromu se preskoci podstromy (a extenty)
//! narozene do generace predchoziho snapshotu - ty uz overil pruchod
//! predchozim stromem; klon preskoci i vse do generace puvodu (patri
//! puvodnimu snapshotu). Cena je tak umerna obsazenemu mistu, ne poctu
//! verzi.
//!
//! U kazdeho uzlu a extentu se prectou **obe** kopie: spatna se prepise
//! dobrou (samoleceni); kdyz neni dobra zadna, zapise se chyba s cestou
//! souboru (`item::SCRUBERR`, nejvys `SCRUB_MAX_ERRORS`). Nezapsane
//! (predalokovane) extenty se nectou.
//!
//! Po krocich (`scrub_step`, rozpocet v blocich) s pozici ulozenou ve
//! strome (`item::SCRUB`), takze beh prezije restart; sluzba ho vola pri
//! necinnosti (nizka priorita). Zmena stromu mezi kroky nevadi: kazdy krok
//! zacne nad aktualnim ulozenym stavem od ulozeneho klice.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

use super::Fs;
use crate::btree;
use crate::datarec::{SCRUB_IDLE, SCRUB_MAX_ERRORS, SCRUB_RUNNING, SCRUB_STOPPED, ScrubState};
use crate::device::BlockDevice;
use crate::layout::{BlockPtr, Extent, Key, META_OID, NODE_BLOCKS, ROOT_DATASET, Timestamp, item};
use crate::node::{Body, Child};

/// Ziva verze v pozici scrubu.
const LIVE: u64 = u64::MAX;

/// Stav scrubu pro klienta.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScrubStatus {
    pub state: ScrubState,
    /// Neopravitelne chyby posledniho behu (cesta a popis).
    pub errors: Vec<String>,
}

/// Pruchod jednim stromem v ramci kroku.
struct Walk {
    min_birth: u64,
    budget: i64,
    /// Kde pokracovat (klic polozky, ktera jeste neprobehla).
    resume: Option<Key>,
    label: String,
}

impl<D: BlockDevice> Fs<D> {
    fn scrub_load(&mut self) -> Result<ScrubState> {
        self.in_root(|fs| {
            let root = fs.s.root;
            match btree::get(&mut fs.s, &root, &Key::new(META_OID, item::SCRUB, 0))? {
                Some(v) => ScrubState::decode(&v),
                None => Ok(ScrubState::default()),
            }
        })
    }

    fn scrub_save(&mut self, st: &ScrubState) -> Result<()> {
        self.in_root(|fs| fs.put(Key::new(META_OID, item::SCRUB, 0), st.encode()))
    }

    /// Spusti scrub (zastaveny pokracuje, kde skoncil). Jen system.
    pub fn scrub_start(&mut self, now: Timestamp) -> Result<()> {
        self.require_system()?;
        self.s.check_writable()?;
        let mut st = self.scrub_load()?;
        match st.state {
            SCRUB_RUNNING => return Ok(()),
            SCRUB_STOPPED => st.state = SCRUB_RUNNING,
            _ => {
                // Novy beh: stare chyby pryc.
                self.in_root(|fs| {
                    let root = fs.s.root;
                    let old = btree::collect(
                        &mut fs.s,
                        &root,
                        &Key::new(META_OID, item::SCRUBERR, 0),
                        &Key::new(META_OID, item::SCRUBERR + 1, 0),
                    )?;
                    for (k, _) in old {
                        fs.del(&k)?;
                    }
                    Ok(())
                })?;
                st = ScrubState {
                    state: SCRUB_RUNNING,
                    started: now,
                    finished: st.finished,
                    dataset: ROOT_DATASET,
                    snapshot: 0,
                    key: Key::MIN,
                    ..ScrubState::default()
                };
            }
        }
        self.guarded(|fs| {
            fs.scrub_save(&st)?;
            fs.commit()
        })
    }

    /// Zastavi scrub (pozice zustane, `scrub_start` pokracuje).
    pub fn scrub_stop(&mut self) -> Result<()> {
        self.require_system()?;
        let mut st = self.scrub_load()?;
        if st.state != SCRUB_RUNNING {
            return Ok(());
        }
        st.state = SCRUB_STOPPED;
        self.guarded(|fs| {
            fs.scrub_save(&st)?;
            fs.commit()
        })
    }

    /// Stav scrubu a neopravitelne chyby (cist smi kazdy - je to stav
    /// disku, ne obsah).
    pub fn scrub_status(&mut self) -> Result<ScrubStatus> {
        let state = self.scrub_load()?;
        let errors = self.in_root(|fs| {
            let root = fs.s.root;
            Ok(btree::collect(
                &mut fs.s,
                &root,
                &Key::new(META_OID, item::SCRUBERR, 0),
                &Key::new(META_OID, item::SCRUBERR + 1, 0),
            )?
            .into_iter()
            .map(|(_, v)| String::from_utf8_lossy(&v).into_owned())
            .collect())
        })?;
        Ok(ScrubStatus { state, errors })
    }

    /// Krok scrubu: overi nejvys `budget` bloku. `Ok(true)` = beh skoncil
    /// (nebo nebezi). Volat opakovane pri necinnosti.
    pub fn scrub_step(&mut self, budget: u64, now: Timestamp) -> Result<bool> {
        self.require_system()?;
        self.s.check_writable()?;
        let mut st = self.scrub_load()?;
        if st.state != SCRUB_RUNNING {
            return Ok(true);
        }
        // Overuje se ulozeny stav (koreny na disku).
        self.commit()?;
        let back = self.cur_ds;
        let mut left = budget as i64;
        let r = self.scrub_run(&mut st, &mut left);
        self.activate(back)?;
        let done = r?;
        if done {
            st.state = SCRUB_IDLE;
            st.finished = now;
        }
        self.guarded(|fs| fs.scrub_save(&st))?;
        Ok(done)
    }

    fn scrub_run(&mut self, st: &mut ScrubState, left: &mut i64) -> Result<bool> {
        let healed_before = self.s.healed;
        loop {
            if *left <= 0 {
                return Ok(false);
            }
            let ids = self.all_dataset_ids()?;
            let Some(&ds) = ids.iter().find(|&&i| i >= st.dataset) else {
                st.repaired += self.s.healed - healed_before;
                return Ok(true);
            };
            if ds != st.dataset {
                st.dataset = ds;
                st.snapshot = 0;
                st.key = Key::MIN;
            }
            self.activate(ds)?;
            let live = self.s.root;
            let floor = self.cur_origin.2;
            let snaps = self.records(&live)?;
            // Cil: snapshot s cislem >= pozice, jinak ziva verze.
            let (root, min_birth, label) = if st.snapshot != LIVE
                && let Some((id, rec)) = snaps.iter().find(|(i, _)| *i >= st.snapshot)
            {
                if *id != st.snapshot {
                    st.snapshot = *id;
                    st.key = Key::MIN;
                }
                let prev = snaps.iter().filter(|(i, _)| i < id).map(|(_, r)| r.generation).max().unwrap_or(0);
                (rec.root, prev.max(floor), format!("dataset {ds}, snapshot {}", rec.name))
            } else {
                if st.snapshot != LIVE {
                    st.snapshot = LIVE;
                    st.key = Key::MIN;
                }
                let Child::Disk(p) = self.tree_of(ds)? else {
                    return Err(Error::BadState);
                };
                let last = snaps.iter().map(|(_, r)| r.generation).max().unwrap_or(0);
                (p, last.max(floor), format!("dataset {ds}"))
            };
            let mut w = Walk {
                min_birth,
                budget: *left,
                resume: None,
                label,
            };
            let start = st.key;
            let finished = if root.birth <= min_birth {
                true
            } else {
                self.scrub_node(&root, None, &start, &mut w, st)?
            };
            *left = w.budget;
            if finished {
                // Dalsi strom: snapshot -> dalsi snapshot nebo ziva verze,
                // ziva verze -> dalsi dataset.
                if st.snapshot == LIVE {
                    st.dataset = ds + 1;
                    st.snapshot = 0;
                } else {
                    st.snapshot += 1;
                }
                st.key = Key::MIN;
            } else {
                st.key = w.resume.unwrap_or(st.key);
                return Ok(false);
            }
        }
    }

    /// Zaznam neopravitelne chyby (nejvys `SCRUB_MAX_ERRORS`).
    fn scrub_error(&mut self, st: &mut ScrubState, msg: String) -> Result<()> {
        st.lost += 1;
        if st.lost > SCRUB_MAX_ERRORS {
            return Ok(());
        }
        let seq = st.lost;
        self.in_root(|fs| fs.put(Key::new(META_OID, item::SCRUBERR, seq), msg.into_bytes()))
    }

    /// Projde uzel `p` (a podstrom) od klice `start`. `false` = dosel
    /// rozpocet (pozice ve `w.resume`).
    fn scrub_node(&mut self, p: &BlockPtr, expect: Option<u8>, start: &Key, w: &mut Walk, st: &mut ScrubState) -> Result<bool> {
        let r = self.s.scrub_units(p.block, p.copy, NODE_BLOCKS, &[p.csum])?;
        w.budget -= (NODE_BLOCKS * if p.copy != 0 { 2 } else { 1 }) as i64;
        st.scanned += NODE_BLOCKS;
        st.bad += r.bad;
        st.repaired += r.repaired;
        if r.lost > 0 {
            let msg = format!("{}: uzel {} je poskozeny v obou kopiich", w.label, p.block);
            self.scrub_error(st, msg)?;
            return Ok(true);
        }
        let node = self.s.load(p)?.clone();
        if expect.is_some_and(|e| e != node.level) {
            return Err(Error::Corrupted);
        }
        match node.body {
            Body::Internal(children) => {
                let first = children.partition_point(|(k, _)| k <= start).saturating_sub(1);
                for (_, c) in &children[first..] {
                    let Child::Disk(cp) = c else { continue };
                    if cp.birth <= w.min_birth {
                        continue;
                    }
                    if !self.scrub_node(cp, Some(node.level - 1), start, w, st)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Body::Leaf(items) => {
                let from = items.partition_point(|(k, _)| k < start);
                for (k, v) in &items[from..] {
                    if w.budget <= 0 {
                        w.resume = Some(*k);
                        return Ok(false);
                    }
                    if k.kind != item::EXTENT {
                        continue;
                    }
                    let e = Extent::decode(v)?;
                    if e.is_unwritten() || e.birth <= w.min_birth {
                        continue;
                    }
                    self.scrub_extent(k, &e, w, st)?;
                }
                Ok(true)
            }
        }
    }

    fn scrub_extent(&mut self, k: &Key, e: &Extent, w: &mut Walk, st: &mut ScrubState) -> Result<()> {
        let r = match e.comp {
            Some(c) => self.s.scrub_units(e.phys, e.copy, c.plen as u64, &e.csums)?,
            None => self.s.scrub_units(e.phys, e.copy, 1, &e.csums)?,
        };
        let copies = if e.copy != 0 { 2 } else { 1 };
        w.budget -= (e.plen() * copies) as i64;
        st.scanned += e.plen();
        st.bad += r.bad;
        st.repaired += r.repaired;
        if r.lost > 0 {
            let root = self.s.root;
            let path = self.path_in(&root, k.oid).ok().flatten().unwrap_or_else(|| format!("objekt {}", k.oid));
            let lb = k.offset + if e.is_compressed() { 0 } else { r.first_lost.unwrap_or(0) };
            let msg = format!("{}: {path}: data od bloku {lb} jsou poskozena ({} bloku)", w.label, r.lost);
            self.scrub_error(st, msg)?;
        }
        Ok(())
    }
}
