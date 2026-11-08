//! Fronta vlaken jednoho CPU: Realtime s rozpoctem, pak spravedlive tridy.
//!
//! Volajici (jadro, simulace) drzi frontu pod zamkem a predava cas `now`.
//! Bezici vlakno ("curr") fronta nedrzi - drzi ho volajici - ale zna jeho
//! ucetni stav, aby mohla rozhodovat o preempci.
//!
//! Zivotni cyklus vlakna: [`RunQueue::enqueue`] -> [`RunQueue::pick`] ->
//! (bezi) -> [`RunQueue::put_curr`] (preempce, zpet do fronty) nebo
//! [`RunQueue::take_curr`] (spanek, konec, presun; vraci [`Ent`], ktery si
//! vlakno odnese a pri dalsim zarazeni preda zpet).

use alloc::vec::Vec;

use crate::class::{Class, FAIR, FAIR_CLASSES, MIN_GRANULARITY_NS};
use crate::fair::{ClassClock, FairQueue};
use crate::rt::Rt;

/// Planovaci stav vlakna mimo frontu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ent {
    /// Trida, se kterou se ma vlakno zaradit (ucinna, po stropu jobu
    /// a zvednuti). Realtime bez `rt` se zaradi jako Interactive.
    pub class: Class,
    /// Zpozdeni ve tride (viz `fair`), kladne = vlaknu se dluzi.
    pub lag: i64,
    /// Rozpocet Realtime vlakna (i kdyz ted bezi jako Interactive).
    pub rt: Option<Rt>,
}

impl Ent {
    pub const fn new(class: Class) -> Self {
        Self {
            class,
            lag: 0,
            rt: None,
        }
    }
}

struct Slot<T> {
    item: T,
    /// `Some` = Realtime vlakno s vycerpanym rozpoctem (bezi jako Interactive).
    rt: Option<Rt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Running {
    Rt,
    Fair(usize),
}

struct Curr {
    running: Running,
    rt: Option<Rt>,
    /// Do kdy je beh zuctovany.
    last: u64,
    /// Kdy bylo vlakno vybrano (pro `MIN_GRANULARITY_NS`).
    picked_at: u64,
}

/// Kam se vlakno zaradilo (pro rozhodnuti o preempci).
enum Placed {
    Rt(u64),
    Fair { c: usize, deadline: u64 },
}

pub struct RunQueue<T> {
    rt: Vec<(T, Rt)>,
    fair: [FairQueue<Slot<T>>; FAIR_CLASSES],
    clock: ClassClock,
    curr: Option<Curr>,
    /// Cas behu po tridach (index = `Class as usize`).
    run_ns: [u64; 4],
}

impl<T> Default for RunQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> RunQueue<T> {
    pub const fn new() -> Self {
        Self {
            rt: Vec::new(),
            fair: [
                FairQueue::new(3_000_000),
                FairQueue::new(6_000_000),
                FairQueue::new(12_000_000),
            ],
            clock: ClassClock::new(),
            curr: None,
            run_ns: [0; 4],
        }
    }

    /// Pocet cekajicich vlaken (bez bezicicho).
    pub fn len(&self) -> usize {
        self.rt.len() + self.fair.iter().map(FairQueue::len).sum::<usize>()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pocet cekajicich ve tride (Realtime = jen s rozpoctem).
    pub fn class_len(&self, class: Class) -> usize {
        match class.fair_index() {
            None => self.rt.len(),
            Some(c) => self.fair[c].len(),
        }
    }

    /// Zatez CPU pro vyvazovani: soucet vah trid cekajicich i bezicicho.
    pub fn weighted_load(&self) -> u64 {
        let queued: u64 = self.rt.len() as u64 * Class::Realtime.load_weight()
            + (0..FAIR_CLASSES).map(|c| self.fair[c].len() as u64 * FAIR[c].load_weight()).sum::<u64>();
        queued + self.curr_class().map_or(0, Class::load_weight)
    }

    /// Cas behu po tridach (ns).
    pub fn run_ns(&self) -> [u64; 4] {
        self.run_ns
    }

    /// Trida, ve ktere prave bezi aktualni vlakno (`None` = necinne).
    pub fn curr_class(&self) -> Option<Class> {
        self.curr.as_ref().map(|c| match c.running {
            Running::Rt => Class::Realtime,
            Running::Fair(i) => FAIR[i],
        })
    }

    fn active(&self) -> [bool; FAIR_CLASSES] {
        [self.fair[0].active(), self.fair[1].active(), self.fair[2].active()]
    }

    /// Zuctuje beh aktualniho vlakna do `now`.
    pub fn update_curr(&mut self, now: u64) {
        let Some(cur) = &mut self.curr else { return };
        let d = now.saturating_sub(cur.last);
        cur.last = now.max(cur.last);
        if d == 0 {
            return;
        }
        match cur.running {
            Running::Rt => {
                if let Some(rt) = &mut cur.rt {
                    rt.charge(d);
                }
                self.run_ns[0] += d;
            }
            Running::Fair(c) => {
                self.fair[c].charge(d);
                let active = self.active();
                self.clock.charge(c, d, active);
                self.run_ns[c + 1] += d;
            }
        }
    }

    fn fair_insert(&mut self, c: usize, slot: Slot<T>, lag: i64) -> Placed {
        if !self.fair[c].active() {
            self.clock.activate(c);
        }
        let deadline = self.fair[c].enqueue(slot, lag);
        Placed::Fair { c, deadline }
    }

    fn insert(&mut self, item: T, ent: Ent, now: u64, wake: bool) -> Placed {
        match (ent.class, ent.rt) {
            (Class::Realtime, Some(mut rt)) => {
                if wake {
                    rt.wake(now);
                } else {
                    rt.refresh(now);
                }
                if rt.has_budget() {
                    let end = rt.period_end;
                    self.rt.push((item, rt));
                    Placed::Rt(end)
                } else {
                    // Vycerpany rozpocet: do konce periody jako Interactive.
                    self.fair_insert(0, Slot { item, rt: Some(rt) }, 0)
                }
            }
            (class, _) => {
                let c = class.fair_index().unwrap_or(0);
                // Spanim se kredit nenasetri (jen dluh zustava).
                let lag = if wake { ent.lag.min(0) } else { ent.lag };
                self.fair_insert(c, Slot { item, rt: None }, lag)
            }
        }
    }

    /// Zaradi vlakno. `wake` = probuzene ze spanku (jinak nove, presunute
    /// nebo se zmenenou tridou). Vraci, zda ma predbehnout bezici vlakno.
    pub fn enqueue(&mut self, item: T, ent: Ent, now: u64, wake: bool) -> bool {
        self.update_curr(now);
        let placed = self.insert(item, ent, now, wake);
        self.preempts(placed, now)
    }

    fn preempts(&self, placed: Placed, now: u64) -> bool {
        let Some(cur) = &self.curr else { return true };
        match (placed, cur.running) {
            (Placed::Rt(end), Running::Rt) => cur.rt.is_some_and(|r| end < r.period_end),
            (Placed::Rt(_), Running::Fair(_)) => true,
            (Placed::Fair { .. }, Running::Rt) => false,
            (Placed::Fair { c, deadline }, Running::Fair(cc)) if c != cc => {
                let _ = deadline;
                self.clock.preempts(c, cc)
            }
            (Placed::Fair { c, deadline }, Running::Fair(_)) => {
                now.saturating_sub(cur.picked_at) >= MIN_GRANULARITY_NS
                    && self.fair[c].curr_deadline().is_some_and(|d| deadline < d)
            }
        }
    }

    /// Presune Realtime vlakna s obnovenym rozpoctem z Interactive zpet.
    fn replenish(&mut self, now: u64) {
        while let Some((slot, _)) = self.fair[0].remove_where(|s| s.rt.is_some_and(|r| r.period_end <= now)) {
            if let Some(mut rt) = slot.rt {
                rt.refresh(now);
                self.rt.push((slot.item, rt));
            }
        }
    }

    /// Nejblizsi obnova rozpoctu cekajiciho Realtime vlakna (pro casovac).
    pub fn next_replenish(&self) -> Option<u64> {
        self.fair[0].iter().filter_map(|s| s.rt.map(|r| r.period_end)).min()
    }

    /// Vybere dalsi vlakno (volat bez bezicicho). Vraci vlakno a delku
    /// jeho behu do dalsiho rozhodnuti (ns).
    pub fn pick(&mut self, now: u64) -> Option<(T, u64)> {
        debug_assert!(self.curr.is_none());
        self.replenish(now);
        for (_, rt) in &mut self.rt {
            rt.refresh(now);
        }
        if let Some(i) = (0..self.rt.len()).min_by_key(|&i| self.rt[i].1.period_end) {
            let (item, rt) = self.rt.swap_remove(i);
            self.curr = Some(Curr {
                running: Running::Rt,
                rt: Some(rt),
                last: now,
                picked_at: now,
            });
            return Some((item, rt.remaining.max(1)));
        }
        let c = self.clock.pick(self.active())?;
        let (slot, left) = self.fair[c].pick()?;
        self.curr = Some(Curr {
            running: Running::Fair(c),
            rt: slot.rt,
            last: now,
            picked_at: now,
        });
        let mut run = left.max(MIN_GRANULARITY_NS);
        // Vycerpane Realtime: rozhodnout znovu nejpozdeji pri obnove.
        if let Some(rt) = slot.rt {
            run = run.min(rt.period_end.saturating_sub(now).max(1));
        }
        Some((slot.item, run))
    }

    /// Trida, se kterou bylo bezici vlakno zarazeno (Realtime i kdyz ted
    /// bezi s vycerpanym rozpoctem jako Interactive).
    pub fn curr_requested_class(&self) -> Option<Class> {
        self.curr.as_ref().map(|c| match (c.running, c.rt) {
            (Running::Rt, _) | (_, Some(_)) => Class::Realtime,
            (Running::Fair(i), None) => FAIR[i],
        })
    }

    /// Vlakno pokracuje v behu bez pruchodu frontou (probudili ho driv,
    /// nez stihlo usnout, a stav uz byl vracen pres [`Self::take_curr`]).
    /// Volat bez bezicicho.
    pub fn resume(&mut self, ent: Ent, now: u64) {
        debug_assert!(self.curr.is_none());
        let (running, rt) = match (ent.class, ent.rt) {
            (Class::Realtime, Some(mut rt)) => {
                rt.refresh(now);
                if rt.has_budget() {
                    (Running::Rt, Some(rt))
                } else {
                    (Running::Fair(0), Some(rt))
                }
            }
            (class, _) => (Running::Fair(class.fair_index().unwrap_or(0)), None),
        };
        if let Running::Fair(c) = running {
            if !self.fair[c].active() {
                self.clock.activate(c);
            }
            self.fair[c].enter_curr(if rt.is_some() { 0 } else { ent.lag });
        }
        self.curr = Some(Curr {
            running,
            rt,
            last: now,
            picked_at: now,
        });
    }

    /// Bezici vlakno dobrovolne prenechava CPU (vola se pred `put_curr`).
    pub fn yield_curr(&mut self) {
        if let Some(Curr { running: Running::Fair(c), .. }) = self.curr {
            self.fair[c].yield_curr();
        }
    }

    /// Vrati bezici vlakno do fronty (preempce, konec kvanta).
    pub fn put_curr(&mut self, item: T, now: u64) {
        self.update_curr(now);
        let Some(cur) = self.curr.take() else { return };
        match cur.running {
            Running::Rt => {
                let rt = cur.rt.expect("Realtime bez rozpoctu");
                if rt.has_budget() {
                    self.rt.push((item, rt));
                } else {
                    self.fair_insert(0, Slot { item, rt: Some(rt) }, 0);
                }
            }
            Running::Fair(c) => match cur.rt {
                Some(rt) if now >= rt.period_end => {
                    self.fair[c].take_curr();
                    self.insert(item, Ent { class: Class::Realtime, lag: 0, rt: Some(rt) }, now, false);
                }
                rt => self.fair[c].put_curr(Slot { item, rt }),
            },
        }
    }

    /// Bezici vlakno odchazi z fronty (spanek, konec, presun, zmena tridy).
    pub fn take_curr(&mut self, now: u64) -> Option<Ent> {
        self.update_curr(now);
        let cur = self.curr.take()?;
        Some(match cur.running {
            Running::Rt => Ent {
                class: Class::Realtime,
                lag: 0,
                rt: cur.rt,
            },
            Running::Fair(c) => {
                let lag = self.fair[c].take_curr();
                Ent {
                    class: if cur.rt.is_some() { Class::Realtime } else { FAIR[c] },
                    lag,
                    rt: cur.rt,
                }
            }
        })
    }

    /// Vyjme cekajici vlakno pro jine CPU: tridy od nejvyssi, jen kde
    /// `allow(trida, pocet cekajicich ve tride)` dovoli.
    pub fn steal(&mut self, mut allow: impl FnMut(Class, usize) -> bool) -> Option<(T, Ent)> {
        if !self.rt.is_empty() && allow(Class::Realtime, self.rt.len()) {
            let (item, rt) = self.rt.pop()?;
            return Some((
                item,
                Ent {
                    class: Class::Realtime,
                    lag: 0,
                    rt: Some(rt),
                },
            ));
        }
        for c in 0..FAIR_CLASSES {
            let n = self.fair[c].len();
            if n == 0 || !allow(FAIR[c], n) {
                continue;
            }
            if let Some((slot, lag)) = self.fair[c].remove_where(|_| true) {
                let class = if slot.rt.is_some() { Class::Realtime } else { FAIR[c] };
                return Some((slot.item, Ent { class, lag, rt: slot.rt }));
            }
        }
        None
    }

    /// Ma byt bezici vlakno vystridano kvuli obnovenemu Realtime rozpoctu
    /// nebo vycerpanemu vlastnimu? (Konec kvanta hlida volajici casovacem.)
    pub fn tick(&mut self, now: u64) -> bool {
        self.update_curr(now);
        let Some(cur) = &self.curr else { return !self.is_empty() };
        match cur.running {
            Running::Rt => cur.rt.is_some_and(|r| !r.has_budget()),
            Running::Fair(_) => {
                cur.rt.is_some_and(|r| now >= r.period_end)
                    || self.next_replenish().is_some_and(|t| t <= now)
            }
        }
    }

    /// Virtualni cas tridy (pro testy a ladeni).
    pub fn class_v(&self, c: usize) -> u64 {
        self.clock.v(c)
    }
}
