//! Testy umisteni na CPU (simulovana topologie).

use abi::sched::core_kind::{EFFICIENCY as E, PERFORMANCE as P, UNKNOWN as U};

use crate::class::Class;
use crate::place::{CpuView, may_steal, push_target, select_cpu};

fn cpus(spec: &[(u8, u32, bool)]) -> Vec<CpuView> {
    spec.iter()
        .enumerate()
        .map(|(index, &(kind, capacity, idle))| CpuView {
            index,
            kind,
            capacity,
            idle,
            queued: if idle { 0 } else { 1 },
            load: if idle { 0 } else { 8 },
        })
        .collect()
}

#[test]
fn interactive_goes_to_idle_performance_core() {
    let c = cpus(&[(P, 1024, false), (P, 1024, true), (E, 512, true), (E, 512, true)]);
    assert_eq!(select_cpu(Class::Interactive, 2, true, c.iter().copied()), 1);
    assert_eq!(select_cpu(Class::Realtime, 3, true, c.iter().copied()), 1);
}

#[test]
fn background_goes_to_efficiency_core() {
    let c = cpus(&[(P, 1024, true), (P, 1024, true), (E, 512, false), (E, 512, true)]);
    assert_eq!(select_cpu(Class::Background, 0, true, c.iter().copied()), 3);
    // Zadne usporne volne: zustane na uspornem, vykonne nevzbudi.
    let c = cpus(&[(P, 1024, true), (E, 512, false), (E, 512, false)]);
    assert_eq!(select_cpu(Class::Background, 2, true, c.iter().copied()), 2);
}

#[test]
fn interactive_leaves_busy_efficiency_core() {
    let c = cpus(&[(P, 1024, false), (P, 1024, false), (E, 512, false)]);
    let target = select_cpu(Class::Interactive, 2, true, c.iter().copied());
    assert!(target < 2, "Interactive ma jit na vykonne jadro, ne {target}");
}

#[test]
fn homogeneous_prefers_last_then_any_idle() {
    let c = cpus(&[(U, 1024, false), (U, 1024, true), (U, 1024, true)]);
    assert_eq!(select_cpu(Class::Normal, 2, false, c.iter().copied()), 2);
    assert_eq!(select_cpu(Class::Background, 0, false, c.iter().copied()), 1);
    let busy = cpus(&[(U, 1024, false), (U, 1024, false)]);
    assert_eq!(select_cpu(Class::Interactive, 1, false, busy.iter().copied()), 1);
}

#[test]
fn efficiency_core_steals_interactive_only_from_overloaded() {
    let e = cpus(&[(E, 512, true)])[0];
    assert!(!may_steal(&e, Class::Interactive, 1, true));
    assert!(may_steal(&e, Class::Interactive, 2, true));
    assert!(may_steal(&e, Class::Background, 1, true));
    assert!(may_steal(&e, Class::Interactive, 1, false));
}

fn loaded(spec: &[(u8, u32, usize)]) -> Vec<CpuView> {
    spec.iter()
        .enumerate()
        .map(|(index, &(kind, capacity, queued))| CpuView {
            index,
            kind,
            capacity,
            idle: false,
            queued,
            // Vsechno Normal (vaha 4), vcetne bezicicho.
            load: (queued as u64 + 1) * 4,
        })
        .collect()
}

#[test]
fn push_balances_busy_cpus() {
    // CPU 0: bezici + 3 cekajici, ostatni po jednom bezicim.
    let c = loaded(&[(U, 1024, 3), (U, 1024, 0), (U, 1024, 1)]);
    assert_eq!(push_target(Class::Background, 0, false, c.iter().copied()), Some(1));
    // Rozdil o jedno: nic (zadne prehazovani).
    let c = loaded(&[(U, 1024, 1), (U, 1024, 0)]);
    assert_eq!(push_target(Class::Normal, 0, false, c.iter().copied()), None);
}

#[test]
fn push_respects_core_types() {
    // CPU 0: 6 vlaken; usporne jadro po presunu 2 * 1024 / 512 = zatez 4.
    let c = loaded(&[(P, 1024, 5), (P, 1024, 0), (E, 512, 0)]);
    // Background jen na usporne jadro, i kdyz je vykonne volnejsi.
    assert_eq!(push_target(Class::Background, 0, true, c.iter().copied()), Some(2));
    assert_eq!(push_target(Class::Interactive, 0, true, c.iter().copied()), Some(1));
}

#[test]
fn push_weighs_classes() {
    // CPU 0: dve Interactive (30), CPU 1: dve Background (2) - stejny
    // pocet vlaken, ale Interactive ma jit k Background.
    let mut c = loaded(&[(U, 1024, 1), (U, 1024, 1)]);
    c[0].load = 30;
    c[1].load = 2;
    assert_eq!(push_target(Class::Interactive, 0, false, c.iter().copied()), Some(1));
    assert_eq!(push_target(Class::Background, 1, false, c.iter().copied()), None);
}
