//! Zamky POSIX (`posix_lock`): konflikty, deleni, slucovani, zmena
//! druhu, uvolneni vlastnika a odhaleni cyklu cekani.

use crate::posix_lock::{Kind, Lock, PosixLocks, SetResult, WAIT_TTL, range};

const F: (u64, u64, u8) = (1, 42, 0);
const MAX: u64 = u64::MAX;

#[test]
fn shared_and_exclusive_conflicts() {
    let mut t = PosixLocks::new();
    assert_eq!(t.set(F, 1, 0, 100, Kind::Read, false, 0), SetResult::Locked);
    assert_eq!(t.set(F, 2, 50, 150, Kind::Read, false, 0), SetResult::Locked, "ctenari se nebiji");
    let c = t.set(F, 3, 90, 95, Kind::Write, false, 0);
    assert!(matches!(c, SetResult::Conflict(Lock { owner: 1 | 2, .. })));
    assert_eq!(t.set(F, 3, 150, 200, Kind::Write, false, 0), SetResult::Locked, "mimo cizi rozsahy");
    assert_eq!(t.test(F, 1, 160, 170, Kind::Read), Some(Lock { owner: 3, start: 150, end: 200, kind: Kind::Write }));
    // Jiny soubor nebo trida flock se neovlivnuji.
    assert_eq!(t.set((1, 42, 1), 9, 0, MAX, Kind::Write, false, 0), SetResult::Locked);
    assert_eq!(t.set((1, 43, 0), 9, 0, MAX, Kind::Write, false, 0), SetResult::Locked);
}

#[test]
fn own_locks_split_merge_and_convert() {
    let mut t = PosixLocks::new();
    t.set(F, 1, 0, 100, Kind::Write, false, 0);
    // Odemceni stredu rozdeli zamek.
    t.unlock(F, 1, 40, 60);
    let mut l = t.locks(F).to_vec();
    l.sort_by_key(|x| x.start);
    assert_eq!(l.iter().map(|x| (x.start, x.end)).collect::<Vec<_>>(), vec![(0, 40), (60, 100)]);
    // Znovu zamknout stred: slouci se do jednoho.
    t.set(F, 1, 40, 60, Kind::Write, false, 0);
    assert_eq!(t.locks(F), &[Lock { owner: 1, start: 0, end: 100, kind: Kind::Write }]);
    // Zmena druhu casti (downgrade): tri useky.
    t.set(F, 1, 10, 20, Kind::Read, false, 0);
    assert_eq!(t.locks(F).len(), 3);
    assert_eq!(t.test(F, 2, 12, 13, Kind::Read), None, "cteni povoleno v ctenem useku");
    assert!(t.test(F, 2, 30, 31, Kind::Read).is_some());
    // Do konce souboru.
    assert_eq!(range(5, 0), (5, MAX));
    t.set(F, 1, 1000, MAX, Kind::Write, false, 0);
    assert!(t.test(F, 2, u64::MAX - 10, MAX, Kind::Read).is_some());
}

#[test]
fn release_file_and_owner() {
    let mut t = PosixLocks::new();
    t.set(F, 1, 0, 10, Kind::Write, false, 0);
    t.set((1, 7, 0), 1, 0, 10, Kind::Write, false, 0);
    t.set(F, 2, 20, 30, Kind::Write, false, 0);
    t.release_file(F, 1);
    assert!(!t.holds(F, 1) && t.holds((1, 7, 0), 1) && t.holds(F, 2));
    t.release_owner(2);
    assert!(t.locks(F).is_empty());
    t.release_owner(1);
    assert!(t.locks((1, 7, 0)).is_empty());
}

#[test]
fn wait_cycle_is_deadlock() {
    let mut t = PosixLocks::new();
    let g = (1, 43, 0);
    t.set(F, 1, 0, 10, Kind::Write, false, 0);
    t.set(g, 2, 0, 10, Kind::Write, false, 0);
    // 1 ceka na 2 (soubor g), pak 2 chce F drzeny 1: cyklus.
    assert!(matches!(t.set(g, 1, 0, 10, Kind::Write, true, 100), SetResult::Conflict(_)));
    assert_eq!(t.set(F, 2, 0, 10, Kind::Write, true, 200), SetResult::Deadlock);
    // Neobnovene cekani vyprsi: uz to neni cyklus.
    assert!(matches!(t.set(F, 2, 0, 10, Kind::Write, true, 100 + WAIT_TTL + 1), SetResult::Conflict(_)));
    // Ziskany zamek zaznam cekani smaze.
    t.release_owner(2);
    assert_eq!(t.set(g, 1, 0, 10, Kind::Write, true, 300), SetResult::Locked);
}
