//! Sprava volneho mista.
//!
//! Disk je rozdeleny na alokacni skupiny (AG). Kazda AG ma v pameti volne
//! useky indexovane podle pozice i podle velikosti (rychle best-fit hledani
//! jako XFS) a na disku vlastni **space log** - pripisovany zaznam alokaci a
//! uvolneni (jako ZFS space mapy). Commit zapise jen nove zaznamy, takze cena
//! je umerna zmenam, ne velikosti disku. Kdyz log naroste, prepise se
//! zhustene ("condense").
//!
//! Uvolnene bloky se nepouziji hned: az za [`REUSE_DELAY`] transakci (blok
//! uvolneny v teze transakci, ve ktere vznikl, hned). Do te doby na ne mohou
//! ukazovat starsi ulozene stavy, ke kterym se pripojeni umi vratit. Teprve
//! kdyz se uvolni definitivne, jdou do fronty TRIM.
//!
//! Umisteni: metadata se berou od zacatku disku (first-fit, drzi se
//! pohromade), data best-fit podle velikosti s cilem hned za predchozim
//! extentem souboru; velke useky se zarovnavaji na 1 MiB.

mod log;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use abi::{Error, Result};

use crate::layout::{DATA_START, REUSE_DELAY};

pub use log::{INDEX_PER_BLOCK, SpaceMeta, TABLE_PER_BLOCK};

/// Typ alokace (urcuje umisteni).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// Uzly stromu, logy: od zacatku disku, pohromade.
    Meta,
    /// Data souboru: best-fit, s cilem za predchozim extentem.
    Data,
}

/// Zaznam space logu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rec {
    pub alloc: bool,
    pub start: u64,
    pub len: u64,
    pub txg: u64,
}

/// Zarovnani velkych datovych useku (bloky; 256 = 1 MiB).
const ALIGN: u64 = 256;

#[derive(Default)]
struct Ag {
    /// Volne useky: zacatek -> delka (sousedni se vzdy slouci).
    free: BTreeMap<u64, u64>,
    /// Tytez useky podle (delka, zacatek).
    by_size: BTreeSet<(u64, u64)>,
    free_blocks: u64,
    /// Zaznamy rozpracovane transakce.
    delta: Vec<Rec>,
}

impl Ag {
    fn insert(&mut self, mut start: u64, mut len: u64) {
        // Slouceni s levym a pravym sousedem.
        if let Some((&ls, &ll)) = self.free.range(..start).next_back()
            && ls + ll == start
        {
            self.free.remove(&ls);
            self.by_size.remove(&(ll, ls));
            start = ls;
            len += ll;
        }
        if let Some(&rl) = self.free.get(&(start + len)) {
            self.free.remove(&(start + len));
            self.by_size.remove(&(rl, start + len));
            len += rl;
        }
        self.free.insert(start, len);
        self.by_size.insert((len, start));
    }

    /// Volny usek, ktery obsahuje blok `b`.
    fn containing(&self, b: u64) -> Option<(u64, u64)> {
        let (&s, &l) = self.free.range(..=b).next_back()?;
        (b < s + l).then_some((s, l))
    }

    /// Odebere `start..start+len` z volnych (cely musi byt volny).
    fn take(&mut self, start: u64, len: u64) -> Result<()> {
        let (s, l) = self.containing(start).ok_or(Error::Corrupted)?;
        if start + len > s + l {
            return Err(Error::Corrupted);
        }
        self.free.remove(&s);
        self.by_size.remove(&(l, s));
        if start > s {
            self.free.insert(s, start - s);
            self.by_size.insert((start - s, s));
        }
        let end = start + len;
        if end < s + l {
            self.free.insert(end, s + l - end);
            self.by_size.insert((s + l - end, end));
        }
        self.free_blocks -= len;
        Ok(())
    }

    fn give(&mut self, start: u64, len: u64) {
        self.insert(start, len);
        self.free_blocks += len;
    }
}

pub struct Space {
    pub block_count: u64,
    pub ag_blocks: u64,
    ags: Vec<Ag>,
    /// Odlozene uvolneni: transakce, od ktere smi byt bloky pouzity -> useky.
    deferred: BTreeMap<u64, Vec<(u64, u64)>>,
    deferred_blocks: u64,
    /// Definitivne uvolnene useky pro TRIM.
    discards: Vec<(u64, u64)>,
    /// Kam pokracovat s first-fit alokaci metadat.
    meta_cursor: u64,
    /// Perzistentni cast (space logy, tabulka AG).
    pub meta: SpaceMeta,
}

impl Space {
    /// Prazdne misto pro novy system (vse volne krome rezervovanych bloku).
    pub fn new(block_count: u64, ag_blocks: u64) -> Self {
        let n = block_count.div_ceil(ag_blocks) as usize;
        let mut s = Self {
            block_count,
            ag_blocks,
            ags: (0..n).map(|_| Ag::default()).collect(),
            deferred: BTreeMap::new(),
            deferred_blocks: 0,
            discards: Vec::new(),
            meta_cursor: DATA_START,
            meta: SpaceMeta::new(n),
        };
        for i in 0..n {
            let (start, end) = s.ag_range(i);
            s.ags[i].give(start, end - start);
        }
        s
    }

    /// Rozsah bloku AG `i` (bez rezervovaneho zacatku disku).
    fn ag_range(&self, i: usize) -> (u64, u64) {
        let start = (i as u64 * self.ag_blocks).max(DATA_START);
        let end = ((i as u64 + 1) * self.ag_blocks).min(self.block_count);
        (start, end)
    }

    fn ag_of(&self, block: u64) -> usize {
        (block / self.ag_blocks) as usize
    }

    /// Volne bloky (pouzitelne hned).
    pub fn free_blocks(&self) -> u64 {
        self.ags.iter().map(|a| a.free_blocks).sum()
    }

    /// Bloky cekajici na odlozene uvolneni.
    pub fn deferred_blocks(&self) -> u64 {
        self.deferred_blocks
    }

    /// Useky cekajici na odlozene uvolneni (kontrola konzistence).
    pub fn deferred_list(&self) -> Vec<(u64, u64)> {
        self.deferred.values().flatten().copied().collect()
    }

    /// Vsechny volne useky (TRIM celeho disku, kontrola).
    pub fn free_extents(&self) -> Vec<(u64, u64)> {
        self.ags
            .iter()
            .flat_map(|a| a.free.iter().map(|(&s, &l)| (s, l)))
            .collect()
    }

    fn record(&mut self, alloc: bool, start: u64, len: u64, txg: u64) {
        // Usek muze presahovat hranici AG jen pri uvolneni slouceneho rozsahu;
        // zaznam se rozdeli po skupinach.
        let mut s = start;
        let end = start + len;
        while s < end {
            let ag = self.ag_of(s);
            let e = end.min((ag as u64 + 1) * self.ag_blocks);
            self.ags[ag].delta.push(Rec {
                alloc,
                start: s,
                len: e - s,
                txg,
            });
            s = e;
        }
    }

    /// Alokuje souvisly usek o delce 1..=`want` bloku. `goal` = idealni
    /// zacatek (konec predchoziho extentu souboru).
    pub fn alloc(
        &mut self,
        want: u64,
        goal: Option<u64>,
        class: Class,
        txg: u64,
    ) -> Result<(u64, u64)> {
        debug_assert!(want > 0);
        let (start, len) = match class {
            Class::Meta => self.pick_meta(want),
            Class::Data => self.pick_data(want, goal),
        }
        .ok_or(Error::NoSpace)?;
        let ag = self.ag_of(start);
        self.ags[ag].take(start, len)?;
        self.record(true, start, len, txg);
        Ok((start, len))
    }

    /// Metadata: first-fit od kurzoru (pak od zacatku), drzi se u zacatku disku.
    fn pick_meta(&mut self, want: u64) -> Option<(u64, u64)> {
        for from in [self.meta_cursor, 0] {
            let first = self.ag_of(from.min(self.block_count - 1));
            for ag in first..self.ags.len() {
                let lo = if ag == first { from } else { 0 };
                let a = &self.ags[ag];
                let hit = a
                    .containing(lo)
                    .map(|(s, l)| (lo, s + l - lo))
                    .or_else(|| a.free.range(lo..).next().map(|(&s, &l)| (s, l)));
                if let Some((s, l)) = hit {
                    let len = l.min(want);
                    self.meta_cursor = s + len;
                    return Some((s, len));
                }
            }
        }
        None
    }

    /// Data: presne na cil, jinak nejmensi dost velky usek (od AG cile),
    /// jinak nejvetsi dostupny (volajici si zbytek alokuje zvlast).
    fn pick_data(&self, want: u64, goal: Option<u64>) -> Option<(u64, u64)> {
        if let Some(g) = goal.filter(|&g| g < self.block_count)
            && let Some((s, l)) = self.ags[self.ag_of(g)].containing(g)
        {
            return Some((g, (s + l - g).min(want)));
        }
        let first = goal.map_or(self.ags.len() / 2, |g| {
            self.ag_of(g.min(self.block_count - 1))
        });
        let order = (first..self.ags.len()).chain(0..first);
        let mut largest: Option<(u64, u64)> = None;
        for ag in order {
            let a = &self.ags[ag];
            if let Some(&(l, s)) = a.by_size.range((want, 0)..).next() {
                // Velky pozadavek z velkeho useku zarovnat na 1 MiB.
                if want >= ALIGN && l >= want + ALIGN {
                    let aligned = s.next_multiple_of(ALIGN);
                    if aligned + want <= s + l {
                        return Some((aligned, want));
                    }
                }
                return Some((s, want));
            }
            if let Some(&(l, s)) = a.by_size.iter().next_back()
                && largest.is_none_or(|(_, ll)| l > ll)
            {
                largest = Some((s, l));
            }
        }
        largest
    }

    /// Uvolni usek. `immediate` = bloky vznikly v teto transakci a zadny
    /// ulozeny stav na ne neukazuje.
    pub fn free(&mut self, start: u64, len: u64, txg: u64, immediate: bool) {
        self.record(false, start, len, txg);
        if immediate {
            self.give_range(start, len);
        } else {
            self.deferred
                .entry(txg + REUSE_DELAY)
                .or_default()
                .push((start, len));
            self.deferred_blocks += len;
        }
    }

    fn give_range(&mut self, start: u64, len: u64) {
        let mut s = start;
        let end = start + len;
        while s < end {
            let ag = self.ag_of(s);
            let e = end.min((ag as u64 + 1) * self.ag_blocks);
            self.ags[ag].give(s, e - s);
            s = e;
        }
    }

    /// Zabere usek, ktery je podle logu volny, ale pouziva ho intent log
    /// (prehravani po vypadku).
    pub fn claim(&mut self, start: u64, len: u64, txg: u64) -> Result<()> {
        if start < DATA_START || start + len > self.block_count {
            return Err(Error::Corrupted);
        }
        let ag = self.ag_of(start);
        if self.ag_of(start + len - 1) != ag {
            return Err(Error::Corrupted);
        }
        // Po kouscich: z volnych, nebo z odlozenych. Pri pripojeni se kazde
        // nedavne uvolneni povazuje za odlozene, i kdyz bylo okamzite (blok
        // vznikl i zanikl v teze transakci) a alokator ho pred vypadkem smel
        // hned znovu pouzit.
        let end = start + len;
        let mut pos = start;
        while pos < end {
            if let Some((s, l)) = self.ags[ag].containing(pos) {
                let n = (s + l).min(end) - pos;
                self.ags[ag].take(pos, n)?;
                pos += n;
            } else {
                pos += self.take_deferred(pos, end)?;
            }
        }
        self.record(true, start, len, txg);
        Ok(())
    }

    /// Odebere z odlozenych uvolneni cast `pos..` (nejvys do `end`), ktera
    /// lezi v jednom useku; vraci jeji delku.
    fn take_deferred(&mut self, pos: u64, end: u64) -> Result<u64> {
        let hit = self.deferred.iter().find_map(|(&ready, list)| {
            list.iter()
                .position(|&(s, l)| s <= pos && pos < s + l)
                .map(|i| (ready, i))
        });
        let (ready, i) = hit.ok_or(Error::Corrupted)?;
        let list = self.deferred.get_mut(&ready).unwrap();
        let (s, l) = list.swap_remove(i);
        let stop = (s + l).min(end);
        if s < pos {
            list.push((s, pos - s));
        }
        if stop < s + l {
            list.push((stop, s + l - stop));
        }
        self.deferred_blocks -= stop - pos;
        Ok(stop - pos)
    }

    /// Zacatek transakce `txg`: odlozene bloky, kterym vyprsela lhuta, jsou
    /// volne a jdou do fronty TRIM.
    pub fn begin_txg(&mut self, txg: u64) {
        let ready: Vec<u64> = self.deferred.range(..=txg).map(|(&t, _)| t).collect();
        for t in ready {
            for (s, l) in self.deferred.remove(&t).unwrap() {
                self.deferred_blocks -= l;
                self.give_range(s, l);
                self.discards.push((s, l));
            }
        }
    }

    /// Useky k TRIM (sloucene sousedni).
    pub fn take_discards(&mut self) -> Vec<(u64, u64)> {
        let mut d = core::mem::take(&mut self.discards);
        d.sort_unstable();
        let mut out: Vec<(u64, u64)> = Vec::with_capacity(d.len());
        for (s, l) in d {
            match out.last_mut() {
                Some((ps, pl)) if *ps + *pl == s => *pl += l,
                _ => out.push((s, l)),
            }
        }
        out
    }
}
