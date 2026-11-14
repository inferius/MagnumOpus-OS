//! Testy alokatoru IOVA na hostiteli: `cargo test -p iova`.

extern crate std;

use std::vec::Vec;

use crate::{IovaError, IovaSpace, PAGE};

const BASE: u64 = 0x10_0000_0000;

fn space(pages: u64) -> IovaSpace {
    IovaSpace::new(BASE, BASE + pages * PAGE)
}

#[test]
fn first_fit_keeps_guard_page() {
    let mut s = space(16);
    let a = s.alloc(2 * PAGE, PAGE).unwrap();
    let b = s.alloc(PAGE, PAGE).unwrap();
    assert_eq!(a, BASE);
    // Mezi a (2 stranky) a b je volna ochranna stranka.
    assert_eq!(b, BASE + 3 * PAGE);
    assert!(s.contains(a + PAGE));
    assert!(!s.contains(a + 2 * PAGE), "ochranna stranka neni obsazena");
}

#[test]
fn freed_range_is_reused() {
    let mut s = space(16);
    let a = s.alloc(4 * PAGE, PAGE).unwrap();
    let b = s.alloc(PAGE, PAGE).unwrap();
    assert_eq!(s.free(a), Some(4 * PAGE));
    assert_eq!(s.free(a), None, "dvakrat uvolnit nejde");
    // Dira pred b (4 stranky + ochranna) pojme 3 stranky s ochrannou.
    assert_eq!(s.alloc(3 * PAGE, PAGE).unwrap(), a);
    assert!(s.contains(b));
}

#[test]
fn alignment_is_respected() {
    let mut s = space(64);
    s.alloc(PAGE, PAGE).unwrap();
    let a = s.alloc(PAGE, 16 * PAGE).unwrap();
    assert!(a.is_multiple_of(16 * PAGE));
    assert_eq!(s.alloc(PAGE, 3 * PAGE), Err(IovaError::Invalid), "zarovnani neni mocnina dvou");
}

#[test]
fn window_end_and_full() {
    let mut s = space(4);
    // 3 stranky + ochranna = cele okno.
    assert_eq!(s.alloc(3 * PAGE, PAGE).unwrap(), BASE);
    assert_eq!(s.alloc(PAGE, PAGE), Err(IovaError::Full));
    assert_eq!(s.alloc(5 * PAGE, PAGE), Err(IovaError::Invalid), "vetsi nez okno");
    assert_eq!(s.alloc(0, PAGE), Err(IovaError::Invalid));
    assert_eq!(s.alloc(100, PAGE), Err(IovaError::Invalid), "nezarovnana delka");
}

#[test]
fn hint_checks_overlap() {
    let mut s = space(32);
    s.reserve(BASE + 4 * PAGE, 2 * PAGE).unwrap();
    // Prekryv zleva, zprava, uvnitr i presne.
    assert_eq!(s.reserve(BASE + 3 * PAGE, 2 * PAGE), Err(IovaError::Overlap));
    assert_eq!(s.reserve(BASE + 5 * PAGE, 2 * PAGE), Err(IovaError::Overlap));
    assert_eq!(s.reserve(BASE + 4 * PAGE, PAGE), Err(IovaError::Overlap));
    // Tesne vedle jde (ochranna stranka se u napovedy nevynucuje).
    s.reserve(BASE + 6 * PAGE, PAGE).unwrap();
    s.reserve(BASE + 3 * PAGE, PAGE).unwrap();
    // Mimo okno, nezarovnane, preteceni.
    assert_eq!(s.reserve(BASE - PAGE, PAGE), Err(IovaError::Invalid));
    assert_eq!(s.reserve(BASE + 31 * PAGE, 2 * PAGE), Err(IovaError::Invalid));
    assert_eq!(s.reserve(BASE + 1, PAGE), Err(IovaError::Invalid));
    assert_eq!(s.reserve(u64::MAX - PAGE + 1, PAGE), Err(IovaError::Invalid));
    // Alokator respektuje napovedy i ochranne stranky za nimi.
    let a = s.alloc(2 * PAGE, PAGE).unwrap();
    assert_eq!(a, BASE);
    let b = s.alloc(PAGE, PAGE).unwrap();
    assert_eq!(b, BASE + 8 * PAGE, "za napovedou na 6 je ochranna stranka 7");
}

#[test]
fn random_model() {
    // Nahodne operace proti jednoduchemu modelu (pole stranek).
    let pages = 256u64;
    let mut s = space(pages);
    let mut model = [false; 256];
    let mut live: Vec<(u64, u64)> = Vec::new();
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for _ in 0..5000 {
        let r = next();
        if r % 3 == 0 && !live.is_empty() {
            let i = (next() % live.len() as u64) as usize;
            let (a, l) = live.swap_remove(i);
            assert_eq!(s.free(a), Some(l));
            for p in 0..l / PAGE {
                model[((a - BASE) / PAGE + p) as usize] = false;
            }
        } else {
            let len = (1 + next() % 8) * PAGE;
            if let Ok(a) = s.alloc(len, PAGE) {
                assert!(a >= BASE && a + len + PAGE <= BASE + pages * PAGE);
                for p in 0..len / PAGE {
                    let idx = ((a - BASE) / PAGE + p) as usize;
                    assert!(!model[idx], "prekryv na strance {idx}");
                    model[idx] = true;
                }
                live.push((a, len));
            }
        }
        for (i, &m) in model.iter().enumerate() {
            assert_eq!(s.contains(BASE + i as u64 * PAGE), m);
        }
    }
}
