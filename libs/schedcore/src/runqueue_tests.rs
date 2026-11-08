//! Testy fronty jednoho CPU.

use crate::class::{Class, MIN_GRANULARITY_NS};
use crate::rt::Rt;
use crate::runqueue::{Ent, RunQueue};

const MS: u64 = 1_000_000;

fn rt_ent(budget: u64, period: u64) -> Ent {
    Ent {
        class: Class::Realtime,
        lag: 0,
        rt: Some(Rt::new(budget, period)),
    }
}

#[test]
fn realtime_first_while_budget_lasts() {
    let mut rq = RunQueue::new();
    rq.enqueue("n", Ent::new(Class::Normal), 0, false);
    rq.enqueue("rt", rt_ent(MS, 10 * MS), 0, false);
    let (t, slice) = rq.pick(0).unwrap();
    assert_eq!(t, "rt");
    assert_eq!(slice, MS);
    // Vycerpa rozpocet: bezi dal jako Interactive.
    rq.put_curr(t, MS);
    assert_eq!(rq.class_len(Class::Realtime), 0);
    assert_eq!(rq.class_len(Class::Interactive), 1);
    assert_eq!(rq.next_replenish(), Some(10 * MS));
    // Po konci periody se vrati mezi Realtime.
    let (t, _) = rq.pick(MS).unwrap();
    rq.put_curr(t, 2 * MS);
    let (t, _) = rq.pick(10 * MS).unwrap();
    assert_eq!(t, "rt");
    assert_eq!(rq.curr_class(), Some(Class::Realtime));
}

#[test]
fn interactive_wake_preempts_background() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Background), 0, false);
    let (cur, _) = rq.pick(0).unwrap();
    assert_eq!(cur, 1);
    assert!(rq.enqueue(2, Ent::new(Class::Interactive), 5 * MS, true));
    rq.put_curr(cur, 5 * MS);
    assert_eq!(rq.pick(5 * MS).unwrap().0, 2);
}

#[test]
fn background_does_not_preempt_interactive() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Interactive), 0, false);
    rq.pick(0).unwrap();
    assert!(!rq.enqueue(2, Ent::new(Class::Background), MS, true));
}

#[test]
fn realtime_wake_preempts_interactive() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Interactive), 0, false);
    rq.pick(0).unwrap();
    assert!(rq.enqueue(2, rt_ent(MS, 5 * MS), MS, true));
}

#[test]
fn same_class_wake_respects_granularity() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Normal), 0, false);
    rq.enqueue(9, Ent::new(Class::Normal), 0, false);
    rq.pick(0).unwrap();
    // Hned po vyberu se nepredbiha.
    assert!(!rq.enqueue(2, Ent::new(Class::Normal), MIN_GRANULARITY_NS / 2, true));
}

#[test]
fn take_curr_returns_class_and_lag() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Normal), 0, false);
    rq.enqueue(2, Ent::new(Class::Normal), 0, false);
    rq.pick(0).unwrap();
    let ent = rq.take_curr(4 * MS).unwrap();
    assert_eq!(ent.class, Class::Normal);
    assert!(ent.lag < 0);
    assert_eq!(rq.len(), 1);
    assert_eq!(rq.curr_class(), None);
}

#[test]
fn steal_respects_filter_and_order() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Background), 0, false);
    rq.enqueue(2, Ent::new(Class::Interactive), 0, false);
    let (t, ent) = rq.steal(|c, _| c != Class::Interactive).unwrap();
    assert_eq!((t, ent.class), (1, Class::Background));
    assert!(rq.steal(|c, _| c != Class::Interactive).is_none());
    assert_eq!(rq.steal(|_, _| true).unwrap().0, 2);
}

#[test]
fn run_time_is_accounted_per_class() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Background), 0, false);
    let (t, _) = rq.pick(0).unwrap();
    rq.put_curr(t, 3 * MS);
    assert_eq!(rq.run_ns()[Class::Background as usize], 3 * MS);
}

#[test]
fn resume_restores_current() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Normal), 0, false);
    rq.pick(0).unwrap();
    let ent = rq.take_curr(MS).unwrap();
    rq.resume(ent, MS);
    assert_eq!(rq.curr_class(), Some(Class::Normal));
    assert_eq!(rq.curr_requested_class(), Some(Class::Normal));
    rq.put_curr(1, 2 * MS);
    assert_eq!(rq.len(), 1);
    assert_eq!(rq.run_ns()[Class::Normal as usize], 2 * MS);
}

#[test]
fn yield_lets_others_run() {
    let mut rq = RunQueue::new();
    rq.enqueue(1, Ent::new(Class::Normal), 0, false);
    rq.enqueue(2, Ent::new(Class::Normal), 0, false);
    let (t, _) = rq.pick(0).unwrap();
    rq.yield_curr();
    rq.put_curr(t, 10_000);
    assert_ne!(rq.pick(10_000).unwrap().0, t);
}
