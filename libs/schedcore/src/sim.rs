//! Simulace planovace na hostiteli: nekolik CPU (i hybridnich, ktere
//! QEMU neumi), kazde s [`RunQueue`], umisteni pres [`place`] a kradeni
//! prace jako v jadre. Cas bezi po krocich `STEP_NS`; vlakno na jadre
//! s kapacitou `c` udela za krok `STEP_NS * c / 1024` prace.
//!
//! Zjednoduseni proti jadru: preempce pri probuzeni se projevi na zacatku
//! dalsiho kroku (latence se meri v krocich), kradeni jen pri necinnosti.

use alloc::vec::Vec;

use abi::sched::core_kind;

use crate::class::Class;
use crate::place::{self, CpuView};
use crate::rt::Rt;
use crate::runqueue::{Ent, RunQueue};

pub const STEP_NS: u64 = 50_000;

#[derive(Debug, Clone, Copy)]
pub enum Behavior {
    /// Porad pocita.
    Spin,
    /// Pocita `work` ns prace (na jadre s kapacitou 1024), pak spi `sleep`.
    Periodic { work: u64, sleep: u64 },
}

pub struct Task {
    pub class: Class,
    pub behavior: Behavior,
    ent: Option<Ent>,
    wake_at: Option<u64>,
    woke_at: Option<u64>,
    left: u64,
    last_cpu: usize,
    /// Cas na CPU (ns).
    pub ran: u64,
    /// Udelana prace (ns na jadre s kapacitou 1024).
    pub work: u64,
    /// Cas na CPU podle typu jadra (index = `core_kind`).
    pub ran_on_kind: [u64; 3],
    /// Latence od probuzeni po spusteni (ns).
    pub latencies: Vec<u64>,
}

struct Cpu {
    kind: u8,
    capacity: u32,
    rq: RunQueue<usize>,
    curr: Option<usize>,
    slice_end: u64,
    resched: bool,
}

pub struct Sim {
    cpus: Vec<Cpu>,
    pub tasks: Vec<Task>,
    pub now: u64,
    hybrid: bool,
}

impl Sim {
    /// CPU zadana jako (typ jadra, kapacita).
    pub fn new(cpus: &[(u8, u32)]) -> Self {
        Self {
            hybrid: crate::topo::is_hybrid(cpus.iter().map(|c| c.0)),
            cpus: cpus
                .iter()
                .map(|&(kind, capacity)| Cpu {
                    kind,
                    capacity,
                    rq: RunQueue::new(),
                    curr: None,
                    slice_end: 0,
                    resched: false,
                })
                .collect(),
            tasks: Vec::new(),
            now: 0,
        }
    }

    /// `n` stejnych CPU.
    pub fn homogeneous(n: usize) -> Self {
        Self::new(&alloc::vec![(core_kind::UNKNOWN, 1024); n])
    }

    fn views(&self) -> Vec<CpuView> {
        self.cpus
            .iter()
            .enumerate()
            .map(|(index, c)| CpuView {
                index,
                kind: c.kind,
                capacity: c.capacity,
                idle: c.curr.is_none() && c.rq.is_empty(),
                queued: c.rq.len(),
                load: c.rq.weighted_load(),
            })
            .collect()
    }

    /// Prida spravedlive vlakno, ktere naposled "bezelo" na `cpu`.
    pub fn add_on(&mut self, cpu: usize, class: Class, behavior: Behavior) -> usize {
        let id = self.add(class, None, behavior);
        self.tasks[id].last_cpu = cpu;
        id
    }

    /// Prida vlakno (Realtime s rozpoctem `rt`), zacne hned.
    pub fn add(&mut self, class: Class, rt: Option<(u64, u64)>, behavior: Behavior) -> usize {
        let id = self.tasks.len();
        let left = match behavior {
            Behavior::Spin => u64::MAX,
            Behavior::Periodic { work, .. } => work,
        };
        self.tasks.push(Task {
            class,
            behavior,
            ent: Some(Ent {
                class,
                lag: 0,
                rt: rt.map(|(b, p)| Rt::new(b, p)),
            }),
            wake_at: Some(self.now),
            woke_at: None,
            left,
            last_cpu: id % self.cpus.len(),
            ran: 0,
            work: 0,
            ran_on_kind: [0; 3],
            latencies: Vec::new(),
        });
        id
    }

    fn wakeups(&mut self) {
        let now = self.now;
        for id in 0..self.tasks.len() {
            if self.tasks[id].wake_at.is_none_or(|w| w > now) {
                continue;
            }
            let views = self.views();
            let t = &mut self.tasks[id];
            t.wake_at = None;
            let ent = t.ent.take().expect("spici vlakno bez stavu");
            let cpu = place::select_cpu(t.class, t.last_cpu, self.hybrid, views.iter().copied());
            let first = t.woke_at.is_none() && t.latencies.is_empty() && t.ran == 0;
            t.woke_at = Some(now);
            let c = &mut self.cpus[cpu];
            if c.rq.enqueue(id, ent, now, !first) && c.curr.is_some() {
                c.resched = true;
            }
        }
    }

    fn schedule(&mut self, i: usize) {
        let now = self.now;
        let c = &mut self.cpus[i];
        let expired = c.curr.is_some() && (now >= c.slice_end || c.resched || c.rq.tick(now));
        if expired {
            let cur = c.curr.take().unwrap();
            // Vyvazovani jako v jadre: preplnene CPU vystridane vlakno posle dal.
            let target = if c.rq.is_empty() {
                None
            } else {
                let class = self.tasks[cur].class;
                let mut views = self.views();
                views[i].idle = false;
                place::push_target(class, i, self.hybrid, views.iter().copied())
            };
            let c = &mut self.cpus[i];
            match target {
                Some(t) => {
                    let ent = c.rq.take_curr(now).expect("bezici bez stavu");
                    if self.cpus[t].rq.enqueue(cur, ent, now, false) && self.cpus[t].curr.is_some() {
                        self.cpus[t].resched = true;
                    }
                }
                None => c.rq.put_curr(cur, now),
            }
        }
        let c = &mut self.cpus[i];
        c.resched = false;
        if c.curr.is_some() {
            return;
        }
        if c.rq.is_empty() {
            self.steal(i);
        }
        let c = &mut self.cpus[i];
        if let Some((id, slice)) = c.rq.pick(now) {
            c.curr = Some(id);
            c.slice_end = now + slice;
            let t = &mut self.tasks[id];
            if let Some(w) = t.woke_at.take() {
                t.latencies.push(now - w);
            }
        }
    }

    fn steal(&mut self, i: usize) {
        let views = self.views();
        let me = views[i];
        let hybrid = self.hybrid;
        for j in (0..self.cpus.len()).filter(|&j| j != i) {
            if let Some((id, ent)) = self.cpus[j].rq.steal(|class, n| place::may_steal(&me, class, n, hybrid)) {
                let now = self.now;
                self.cpus[i].rq.enqueue(id, ent, now, false);
                return;
            }
        }
    }

    fn execute(&mut self, i: usize) {
        let c = &mut self.cpus[i];
        let Some(id) = c.curr else { return };
        let t = &mut self.tasks[id];
        let work = STEP_NS * c.capacity as u64 / 1024;
        t.ran += STEP_NS;
        t.ran_on_kind[c.kind as usize] += STEP_NS;
        t.work += work;
        t.last_cpu = i;
        if let Behavior::Periodic { work: w, sleep } = t.behavior {
            t.left = t.left.saturating_sub(work);
            if t.left == 0 {
                t.left = w;
                let end = self.now + STEP_NS;
                t.ent = c.rq.take_curr(end);
                t.wake_at = Some(end + sleep);
                c.curr = None;
            }
        }
    }

    /// Simuluje `duration` ns.
    pub fn run(&mut self, duration: u64) {
        let end = self.now + duration;
        while self.now < end {
            self.wakeups();
            for i in 0..self.cpus.len() {
                self.schedule(i);
            }
            for i in 0..self.cpus.len() {
                self.execute(i);
            }
            self.now += STEP_NS;
        }
    }

    /// Podil casu vlakna v promile z `total` ns.
    pub fn share_permille(&self, id: usize, total: u64) -> u64 {
        self.tasks[id].ran * 1000 / total
    }
}
