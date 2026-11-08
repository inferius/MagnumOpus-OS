//! Rozlozeni vlaken mezi CPU: vyber CPU podle tridy a kapacity jadra,
//! presun vystridaneho vlakna na necinne CPU a kradeni prace.
//!
//! Pravidla jsou v `schedcore::place` (testovana i nad simulovanou
//! hybridni topologii); tady se jen sestavi pohled na CPU z atomickych
//! kopii stavu (bez zamku front).

use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use schedcore::place::{self, CpuView};
use schedcore::{Class, Ent};

use super::percpu::{self, PerCpu};
use super::{Thread, topo};

fn view(c: &PerCpu) -> CpuView {
    let queued = c.queued.load(Ordering::Relaxed);
    CpuView {
        index: c.index,
        kind: c.kind.load(Ordering::Relaxed),
        capacity: c.capacity.load(Ordering::Relaxed),
        idle: c.is_idle.load(Ordering::Relaxed) && queued == 0,
        queued,
        load: c.load.load(Ordering::Relaxed),
    }
}

fn views() -> impl Iterator<Item = CpuView> + Clone {
    (0..percpu::cpu_count()).filter_map(percpu::cpu).map(view)
}

/// CPU pro vlakno tridy `class`, ktere naposled bezelo na `last`.
pub(super) fn select_cpu(class: Class, last: usize) -> &'static PerCpu {
    let i = place::select_cpu(class, last, topo::hybrid(), views());
    percpu::cpu(i).unwrap_or_else(percpu::this_cpu)
}

/// Necinne CPU, kam presunout vystridane vlakno z `cpu` (`None` = zustat).
/// Jen kdyz pravidla umisteni vyberou jine CPU, ktere je necinne.
pub(super) fn idle_target(cpu: &PerCpu, class: Class) -> Option<&'static PerCpu> {
    let i = place::select_cpu(class, cpu.index, topo::hybrid(), views());
    let target = percpu::cpu(i)?;
    (i != cpu.index && view(target).idle).then_some(target)
}

/// Mene zatizene CPU pro vystridane vlakno z preplneneho `cpu` (zatez
/// vazena kapacitou), viz `place::push_target`.
pub(super) fn push_target(cpu: &PerCpu, class: Class) -> Option<&'static PerCpu> {
    let me = cpu.index;
    // Toto CPU prave vlakno stridalo: pocita se jako bezici.
    let views = views().map(move |mut v| {
        if v.index == me {
            v.idle = false;
        }
        v
    });
    let i = place::push_target(class, me, topo::hybrid(), views)?;
    percpu::cpu(i)
}

/// Je vlakno tridy `class` na `cpu` na spatnem typu jadra (Interactive na
/// uspornem, kdyz je vykonne necinne)?
pub(super) fn misfit(cpu: &PerCpu, class: Class) -> bool {
    topo::hybrid()
        && matches!(class, Class::Realtime | Class::Interactive)
        && cpu.capacity.load(Ordering::Relaxed) < abi::sched::CAPACITY_MAX
}

/// Vezme cekajici vlakno z jine fronty pro necinne `cpu`. `try_lock`, aby
/// se dve kradouci CPU nezablokovala navzajem.
pub(super) fn steal(cpu: &PerCpu) -> Option<(Arc<Thread>, Ent)> {
    let me = view(cpu);
    let hybrid = topo::hybrid();
    for other in percpu::all_cpus().filter(|c| c.index != cpu.index) {
        if other.queued.load(Ordering::Relaxed) == 0 {
            continue;
        }
        let Some(mut rq) = other.rq.try_lock() else { continue };
        let got = rq.steal(|class, n| place::may_steal(&me, class, n, hybrid));
        other.sync(&rq);
        if got.is_some() {
            return got;
        }
    }
    None
}

/// Ma nekde prace, kterou by `cpu` smelo vzit?
pub(super) fn stealable(cpu: &PerCpu) -> bool {
    let me = view(cpu);
    let hybrid = topo::hybrid();
    percpu::all_cpus().filter(|c| c.index != cpu.index).any(|c| {
        c.queued.load(Ordering::Relaxed) > 0
            && c.rq.try_lock().is_some_and(|rq| {
                [Class::Realtime, Class::Interactive, Class::Normal, Class::Background]
                    .into_iter()
                    .any(|cl| {
                        let n = rq.class_len(cl);
                        n > 0 && place::may_steal(&me, cl, n, hybrid)
                    })
            })
    })
}
