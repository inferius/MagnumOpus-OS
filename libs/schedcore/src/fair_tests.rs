//! Testy EEVDF fronty a vazeneho casu trid.

use crate::class::{FAIR_CLASSES, WEIGHTS};
use crate::fair::{ClassClock, FairQueue};

#[test]
fn equal_threads_round_robin() {
    let mut q = FairQueue::new(1000);
    for i in 0..3 {
        q.enqueue(i, 0);
    }
    let mut ran = [0u64; 3];
    for _ in 0..300 {
        let (t, slice) = q.pick().unwrap();
        q.charge(slice);
        ran[t] += slice;
        q.put_curr(t);
    }
    let (min, max) = (ran.iter().min().unwrap(), ran.iter().max().unwrap());
    assert!(max - min <= 1000, "nerovnomerne: {ran:?}");
}

#[test]
fn lag_is_clamped_and_kept() {
    let mut q = FairQueue::new(1000);
    q.enqueue(0, 0);
    q.enqueue(1, 0);
    let (t, _) = q.pick().unwrap();
    q.charge(5000);
    let lag = q.take_curr();
    assert!(lag < 0, "vlakno, ktere bezelo, ma zaporny lag: {lag}");
    // Navrat s velkym dluhem se orizne na kvantum.
    q.enqueue(t, -1_000_000);
    let (next, _) = q.pick().unwrap();
    assert_ne!(next, t, "dluznik nesmi byt vybran prvni");
}

#[test]
fn eligible_earliest_deadline_wins() {
    let mut q = FairQueue::new(10_000);
    q.enqueue('a', 0);
    let (a, _) = q.pick().unwrap();
    q.charge(8_000);
    q.put_curr(a);
    // 'b' prisel s kreditem: 'a' je nad prumerem (nezpusobile), 'b' vyhraje
    // i pres pozdejsi termin.
    q.enqueue('b', 5_000);
    assert_eq!(q.pick().unwrap().0, 'b');
}

#[test]
fn remove_where_returns_lag() {
    let mut q = FairQueue::new(1000);
    q.enqueue(1, 0);
    q.enqueue(2, 0);
    let (item, lag) = q.remove_where(|&x| x == 2).unwrap();
    assert_eq!(item, 2);
    assert_eq!(lag, 0);
    assert_eq!(q.len(), 1);
}

#[test]
fn class_clock_shares_follow_weights() {
    let mut clock = ClassClock::new();
    let active = [true; FAIR_CLASSES];
    let mut ran = [0u64; FAIR_CLASSES];
    for c in 0..FAIR_CLASSES {
        clock.activate(c);
    }
    for _ in 0..20_000 {
        let c = clock.pick(active).unwrap();
        clock.charge(c, 1000, active);
        ran[c] += 1000;
    }
    let total: u64 = ran.iter().sum();
    let wsum: u64 = WEIGHTS.iter().sum();
    for c in 0..FAIR_CLASSES {
        let expect = total * WEIGHTS[c] / wsum;
        let diff = ran[c].abs_diff(expect);
        assert!(diff * 100 <= expect + 100_000, "trida {c}: {} proti {expect}", ran[c]);
    }
}

#[test]
fn sleeping_class_banks_no_credit() {
    let mut clock = ClassClock::new();
    // Jen Background bezi dlouho.
    clock.activate(2);
    let only_bg = [false, false, true];
    for _ in 0..1000 {
        clock.charge(2, 1_000_000, only_bg);
    }
    // Interactive se probudi: na rade je hned, ale ne na dlouho.
    clock.activate(0);
    let both = [true, false, true];
    assert!(clock.preempts(0, 2));
    let mut i_ran = 0;
    while clock.pick(both) == Some(0) {
        clock.charge(0, 1_000_000, both);
        i_ran += 1;
    }
    assert!(i_ran <= 2, "Interactive si nasporil kredit: {i_ran} ms");
}
