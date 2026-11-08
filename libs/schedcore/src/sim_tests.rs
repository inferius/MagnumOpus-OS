//! Simulace: hladoveni, podily, latence, hybridni CPU.
//!
//! Hybridni topologie (vykonna + usporna jadra) je overena **jen** zde -
//! QEMU ji neumi.

use abi::sched::core_kind::{EFFICIENCY as E, PERFORMANCE as P};

use crate::class::Class;
use crate::sim::{Behavior, STEP_NS, Sim};

const MS: u64 = 1_000_000;

#[test]
fn background_gets_floor_under_interactive_load() {
    let mut sim = Sim::homogeneous(1);
    let i = sim.add(Class::Interactive, None, Behavior::Spin);
    let b = sim.add(Class::Background, None, Behavior::Spin);
    sim.run(2000 * MS);
    let bg = sim.share_permille(b, 2000 * MS);
    // Interactive : Background = 15 : 1 -> 62 promile.
    assert!(bg >= 50, "Background dostal jen {bg} promile");
    assert!(sim.share_permille(i, 2000 * MS) >= 900);
}

#[test]
fn three_classes_split_by_weight() {
    let mut sim = Sim::homogeneous(1);
    let ids = [Class::Interactive, Class::Normal, Class::Background].map(|c| sim.add(c, None, Behavior::Spin));
    sim.run(3000 * MS);
    let s = ids.map(|id| sim.share_permille(id, 3000 * MS));
    assert!((720..=780).contains(&s[0]), "{s:?}");
    assert!((170..=230).contains(&s[1]), "{s:?}");
    assert!((40..=70).contains(&s[2]), "{s:?}");
}

#[test]
fn many_background_threads_do_not_beat_interactive() {
    let mut sim = Sim::homogeneous(1);
    let i = sim.add(Class::Interactive, None, Behavior::Spin);
    for _ in 0..20 {
        sim.add(Class::Background, None, Behavior::Spin);
    }
    sim.run(2000 * MS);
    assert!(sim.share_permille(i, 2000 * MS) >= 900);
}

#[test]
fn interactive_wake_latency_under_full_load() {
    let mut sim = Sim::homogeneous(4);
    for _ in 0..8 {
        sim.add(Class::Background, None, Behavior::Spin);
        sim.add(Class::Normal, None, Behavior::Spin);
    }
    let ui = sim.add(Class::Interactive, None, Behavior::Periodic { work: MS, sleep: 16 * MS });
    sim.run(1000 * MS);
    let lat = &sim.tasks[ui].latencies;
    assert!(lat.len() >= 50, "malo probuzeni: {}", lat.len());
    let max = *lat.iter().max().unwrap();
    assert!(max <= 2 * STEP_NS, "latence probuzeni {max} ns");
}

#[test]
fn realtime_budget_bounds_cpu() {
    let mut sim = Sim::homogeneous(1);
    // Realtime smycka s rozpoctem 1 ms / 5 ms nesmi zahltit CPU.
    let rt = sim.add(Class::Realtime, Some((MS, 5 * MS)), Behavior::Spin);
    let n = sim.add(Class::Normal, None, Behavior::Spin);
    sim.run(1000 * MS);
    // 20 % jako Realtime, zbytek 15:4 jako Interactive proti Normal.
    let normal = sim.share_permille(n, 1000 * MS);
    assert!(normal >= 150, "Normal dostal jen {normal} promile");
    assert!(sim.share_permille(rt, 1000 * MS) <= 850);
}

#[test]
fn periodic_realtime_runs_immediately() {
    let mut sim = Sim::homogeneous(1);
    sim.add(Class::Interactive, None, Behavior::Spin);
    let audio = sim.add(Class::Realtime, Some((MS, 5 * MS)), Behavior::Periodic { work: MS / 2, sleep: 5 * MS });
    sim.run(500 * MS);
    let lat = &sim.tasks[audio].latencies;
    assert!(lat.len() >= 50);
    assert!(lat.iter().all(|&l| l <= STEP_NS), "{lat:?}");
}

#[test]
fn hybrid_places_by_class() {
    let mut sim = Sim::new(&[(P, 1024), (P, 1024), (E, 512), (E, 512)]);
    let ui = [
        sim.add(Class::Interactive, None, Behavior::Spin),
        sim.add(Class::Interactive, None, Behavior::Spin),
    ];
    let bg = [
        sim.add(Class::Background, None, Behavior::Spin),
        sim.add(Class::Background, None, Behavior::Spin),
    ];
    sim.run(500 * MS);
    for id in ui {
        let t = &sim.tasks[id];
        assert!(t.ran_on_kind[P as usize] * 10 >= t.ran * 9, "Interactive mimo vykonna jadra: {:?}", t.ran_on_kind);
    }
    for id in bg {
        let t = &sim.tasks[id];
        assert!(t.ran_on_kind[E as usize] * 10 >= t.ran * 9, "Background mimo usporna jadra: {:?}", t.ran_on_kind);
    }
}

#[test]
fn hybrid_interactive_wake_goes_to_performance_core() {
    let mut sim = Sim::new(&[(P, 1024), (E, 512), (E, 512)]);
    let ui = sim.add(Class::Interactive, None, Behavior::Periodic { work: MS, sleep: 8 * MS });
    sim.add(Class::Background, None, Behavior::Spin);
    sim.run(300 * MS);
    let t = &sim.tasks[ui];
    assert!(t.ran_on_kind[P as usize] * 10 >= t.ran * 9, "{:?}", t.ran_on_kind);
}

#[test]
fn hybrid_overload_uses_all_cores_without_starvation() {
    let mut sim = Sim::new(&[(P, 1024), (P, 1024), (E, 512), (E, 512)]);
    let ids: Vec<_> = (0..6).map(|_| sim.add(Class::Normal, None, Behavior::Spin)).collect();
    sim.run(600 * MS);
    for id in ids {
        assert!(sim.tasks[id].ran >= 100 * MS, "vlakno {id} hladovi: {}", sim.tasks[id].ran);
    }
}

#[test]
fn homogeneous_spreads_load() {
    let mut sim = Sim::homogeneous(4);
    let ids: Vec<_> = (0..4).map(|_| sim.add(Class::Normal, None, Behavior::Spin)).collect();
    sim.run(100 * MS);
    for id in ids {
        assert!(sim.share_permille(id, 100 * MS) >= 950);
    }
}

#[test]
fn busy_cpus_rebalance() {
    // Vsechna vlakna vzniknou na CPU 0 (jako deti jednoho rodice).
    let mut sim = Sim::homogeneous(4);
    let ids: Vec<_> = (0..8).map(|_| sim.add_on(0, Class::Background, Behavior::Spin)).collect();
    sim.run(500 * MS);
    for id in ids {
        assert!(sim.share_permille(id, 500 * MS) >= 300, "vlakno {id}: {}", sim.share_permille(id, 500 * MS));
    }
}

#[test]
fn rebalance_by_class_weight() {
    // Na kazde CPU pripada jedna Interactive a jedna Background smycka,
    // ale vsechny vzniknou na CPU 0: Background musi dostat svuj podil.
    let mut sim = Sim::homogeneous(4);
    let bg: Vec<_> = (0..4)
        .map(|_| {
            sim.add_on(0, Class::Interactive, Behavior::Spin);
            sim.add_on(0, Class::Background, Behavior::Spin)
        })
        .collect();
    sim.run(1000 * MS);
    let total: u64 = bg.iter().map(|&id| sim.tasks[id].ran).sum();
    let permille = total * 1000 / (4 * 1000 * MS);
    assert!((40..=150).contains(&permille), "Background celkem {permille} promile");
}
