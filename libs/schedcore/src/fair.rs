//! Spravedlive deleni casu: EEVDF uvnitr tridy a vazeny cas trid.
//!
//! **EEVDF** (Earliest Eligible Virtual Deadline First, Stoica 1995, Linux
//! od 6.6): kazde vlakno ma virtualni cas `v` (kolik bezelo) a termin
//! `v + kvantum`. Zpusobile je vlakno s `v <= prumer` (dostalo mene nez
//! svuj podil); z nich vyhrava nejdrivejsi termin. Vlakna jedne tridy maji
//! stejnou vahu, takze virtualni cas = skutecny cas behu.
//!
//! **Zpozdeni (lag)** = `prumer - v`: kladne = vlaknu se dluzi. Pri
//! odchodu z fronty (spanek, presun na jine CPU) si vlakno lag odnese
//! a pri zarazeni se umisti na `prumer - lag` (oriznuto na +/- kvantum).
//!
//! Slozitost: vyber je linearni pruchod frontou (O(n)). Fronty jednoho CPU
//! a tridy jsou male (jednotky az desitky vlaken); pro stovky vlaken by
//! se hodil vyvazeny strom s minimem terminu v podstromu (jako Linux).

use alloc::vec::Vec;

use crate::class::{FAIR_CLASSES, WEIGHT_LCM, WEIGHTS};

struct Entry<T> {
    item: T,
    v: u64,
    deadline: u64,
}

/// Prave bezici vlakno fronty (neni v `entries`, ale pocita se do prumeru).
#[derive(Debug, Clone, Copy)]
struct Curr {
    v: u64,
    deadline: u64,
}

/// Fronta EEVDF jedne tridy na jednom CPU.
pub struct FairQueue<T> {
    entries: Vec<Entry<T>>,
    curr: Option<Curr>,
    /// Soucet `v` vsech ve fronte i bezicicho (pro prumer).
    sum: u128,
    count: u64,
    slice: u64,
    /// Posledni prumer, kdyz se fronta vyprazdnila (pro dalsi prichozi).
    last_avg: u64,
}

impl<T> FairQueue<T> {
    pub const fn new(slice_ns: u64) -> Self {
        Self {
            entries: Vec::new(),
            curr: None,
            sum: 0,
            count: 0,
            slice: slice_ns,
            last_avg: 0,
        }
    }

    /// Pocet cekajicich (bez bezicicho).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Ma trida na tomto CPU praci (cekajici nebo bezici vlakno)?
    pub fn active(&self) -> bool {
        self.count > 0
    }

    pub fn has_curr(&self) -> bool {
        self.curr.is_some()
    }

    pub fn slice(&self) -> u64 {
        self.slice
    }

    /// Prumerny virtualni cas (virtualni hodiny fronty).
    pub fn avg(&self) -> u64 {
        if self.count == 0 {
            self.last_avg
        } else {
            (self.sum / self.count as u128) as u64
        }
    }

    /// Zaradi vlakno se zpozdenim `lag` (oriznuto na +/- kvantum). Vraci
    /// jeho termin.
    pub fn enqueue(&mut self, item: T, lag: i64) -> u64 {
        let s = self.slice as i64;
        let lag = lag.clamp(-s, s);
        let v = (self.avg() as i64).saturating_sub(lag).max(0) as u64;
        let deadline = v + self.slice;
        self.push(item, v, deadline);
        deadline
    }

    fn push(&mut self, item: T, v: u64, deadline: u64) {
        self.entries.push(Entry { item, v, deadline });
        self.sum += v as u128;
        self.count += 1;
    }

    fn best_index(&self) -> Option<usize> {
        let avg = self.avg();
        let mut best: Option<usize> = None;
        for (i, e) in self.entries.iter().enumerate() {
            if e.v > avg {
                continue;
            }
            if best.is_none_or(|b| e.deadline < self.entries[b].deadline) {
                best = Some(i);
            }
        }
        // Zaokrouhleni prumeru: aspon vlakno s nejmensim `v` je zpusobile;
        // pojistka pro pripad, ze vsechna cekajici maji `v` nad prumerem
        // (prumer stahuje dolu bezici vlakno).
        best.or_else(|| {
            (0..self.entries.len()).min_by_key(|&i| (self.entries[i].v, self.entries[i].deadline))
        })
    }

    /// Termin, se kterym by vybrane vlakno bezelo (bez vyberu).
    pub fn peek_deadline(&self) -> Option<u64> {
        self.best_index().map(|i| self.entries[i].deadline)
    }

    /// Vybere dalsi vlakno; stane se bezicim. Volat jen bez bezicicho.
    /// Vraci vlakno a zbytek jeho kvanta (ns).
    pub fn pick(&mut self) -> Option<(T, u64)> {
        debug_assert!(self.curr.is_none());
        let i = self.best_index()?;
        let e = self.entries.swap_remove(i);
        self.curr = Some(Curr {
            v: e.v,
            deadline: e.deadline,
        });
        Some((e.item, e.deadline.saturating_sub(e.v)))
    }

    /// Vlakno se stane bezicim primo, bez cekani ve fronte (pokracuje,
    /// protoze ho probudili driv, nez stihlo usnout). Volat bez bezicicho.
    pub fn enter_curr(&mut self, lag: i64) {
        debug_assert!(self.curr.is_none());
        let s = self.slice as i64;
        let v = (self.avg() as i64).saturating_sub(lag.clamp(-s, s)).max(0) as u64;
        self.curr = Some(Curr {
            v,
            deadline: v + self.slice,
        });
        self.sum += v as u128;
        self.count += 1;
    }

    /// Bezici vlakno bezelo dalsich `ran` ns. Po vycerpani kvanta dostane
    /// novy termin (dalsi pozadavek).
    pub fn charge(&mut self, ran: u64) {
        if let Some(c) = &mut self.curr {
            c.v += ran;
            self.sum += ran as u128;
            if c.v >= c.deadline {
                c.deadline = c.v + self.slice;
            }
        }
    }

    /// Bezici vlakno se vzdava zbytku pozadavku (yield): termin se posune
    /// o kvantum, aby prednost dostala ostatni zpusobila vlakna.
    pub fn yield_curr(&mut self) {
        if let Some(c) = &mut self.curr {
            c.deadline = c.deadline.max(c.v) + self.slice;
        }
    }

    /// Termin bezicicho vlakna.
    pub fn curr_deadline(&self) -> Option<u64> {
        self.curr.map(|c| c.deadline)
    }

    /// Vrati bezici vlakno do fronty (preempce) s jeho `v` i terminem.
    pub fn put_curr(&mut self, item: T) {
        if let Some(c) = self.curr.take() {
            self.entries.push(Entry {
                item,
                v: c.v,
                deadline: c.deadline,
            });
        }
    }

    /// Bezici vlakno odchazi (spanek, konec, presun). Vraci jeho lag.
    pub fn take_curr(&mut self) -> i64 {
        let Some(c) = self.curr.take() else { return 0 };
        self.leave(c.v)
    }

    fn leave(&mut self, v: u64) -> i64 {
        let avg = self.avg();
        self.sum -= v as u128;
        self.count -= 1;
        if self.count == 0 {
            self.last_avg = avg;
        }
        avg as i64 - v as i64
    }

    /// Vyjme cekajici vlakno, ktere splni `pred` (kradeni jinym CPU).
    /// Bere od konce (nejdele cekajici maji cache nejstudenejsi). Vraci
    /// vlakno a jeho lag.
    pub fn remove_where(&mut self, mut pred: impl FnMut(&T) -> bool) -> Option<(T, i64)> {
        let i = self.entries.iter().rposition(|e| pred(&e.item))?;
        let e = self.entries.swap_remove(i);
        let lag = self.leave(e.v);
        Some((e.item, lag))
    }

    /// Projde cekajici vlakna.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|e| &e.item)
    }
}

/// Vazeny virtualni cas spravedlivych trid na jednom CPU (uroven nad
/// EEVDF): beh tridy posune jeji cas o `ran * WEIGHT_LCM / vaha`; vyhrava
/// aktivni trida s nejmensim casem (pri shode vyssi trida).
///
/// Trida, ktera byla prazdna, se pri aktivaci posune aspon na `floor`
/// (nejmensi cas aktivnich trid): spanim si nenasetri kredit, ale je hned
/// na rade.
#[derive(Debug, Clone, Default)]
pub struct ClassClock {
    v: [u64; FAIR_CLASSES],
    floor: u64,
}

impl ClassClock {
    pub const fn new() -> Self {
        Self {
            v: [0; FAIR_CLASSES],
            floor: 0,
        }
    }

    pub fn v(&self, c: usize) -> u64 {
        self.v[c]
    }

    /// Trida `c` prave dostala praci.
    pub fn activate(&mut self, c: usize) {
        self.v[c] = self.v[c].max(self.floor);
    }

    /// Trida `c` bezela `ran` ns; `active` = ktere tridy maji praci.
    pub fn charge(&mut self, c: usize, ran: u64, active: [bool; FAIR_CLASSES]) {
        self.v[c] += ran * (WEIGHT_LCM / WEIGHTS[c]);
        self.update_floor(active);
    }

    fn update_floor(&mut self, active: [bool; FAIR_CLASSES]) {
        if let Some(min) = (0..FAIR_CLASSES).filter(|&c| active[c]).map(|c| self.v[c]).min() {
            self.floor = self.floor.max(min);
        }
    }

    /// Vybere tridu: aktivni s nejmensim casem, pri shode vyssi.
    pub fn pick(&mut self, active: [bool; FAIR_CLASSES]) -> Option<usize> {
        self.update_floor(active);
        (0..FAIR_CLASSES)
            .filter(|&c| active[c])
            .min_by_key(|&c| (self.v[c], c))
    }

    /// Predbehne trida `new` bezici tridu `cur`?
    pub fn preempts(&self, new: usize, cur: usize) -> bool {
        new != cur && (self.v[new], new) < (self.v[cur], cur)
    }
}
