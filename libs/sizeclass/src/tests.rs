//! Testy trid: zarovnani, opakovane pouziti, prekryvy objektu, nahodna
//! zatez proti modelu.

use std::alloc::{alloc, dealloc};
use std::collections::BTreeMap;
use std::vec::Vec;

use super::*;

/// Kusy pro tridy z alokatoru hostitele (uvolni se na konci testu).
struct Backing(Vec<(*mut u8, Layout)>);

impl Backing {
    fn take(&mut self, l: Layout) -> *mut u8 {
        // SAFETY: layout ma nenulovou velikost.
        let p = unsafe { alloc(l) };
        self.0.push((p, l));
        p
    }
}

impl Drop for Backing {
    fn drop(&mut self) {
        for (p, l) in self.0.drain(..) {
            // SAFETY: `p` vratil `alloc` se stejnym layoutem.
            unsafe { dealloc(p, l) };
        }
    }
}

#[test]
fn class_selection() {
    let l = |s, a| Layout::from_size_align(s, a).unwrap();
    assert_eq!(class_of(l(0, 1)), Some(0));
    assert_eq!(class_of(l(1, 1)), Some(0));
    assert_eq!(class_of(l(16, 8)), Some(0));
    assert_eq!(class_of(l(17, 8)), Some(1));
    assert_eq!(class_of(l(100, 16)), Some(5));
    assert_eq!(class_of(l(4096, 16)), Some(15));
    assert_eq!(class_of(l(4097, 8)), None);
    assert_eq!(class_of(l(64, 32)), None, "vetsi zarovnani do hlavniho alokatoru");
    for (c, &s) in CLASSES.iter().enumerate() {
        assert_eq!(s % ALIGN, 0);
        assert!(chunk_size(c) >= s);
    }
}

#[test]
fn objects_are_aligned_distinct_and_reused() {
    let mut b = Backing(Vec::new());
    let mut cl = Classes::new();
    for c in 0..CLASSES.len() {
        let n = chunk_size(c) / CLASSES[c] * 2 + 3;
        let mut got = Vec::new();
        for _ in 0..n {
            // SAFETY: refill vraci pamet z alokatoru hostitele.
            let p = unsafe { cl.alloc(c, |l| b.take(l)) };
            assert!(!p.is_null());
            assert_eq!(p as usize % ALIGN, 0);
            got.push(p as usize);
        }
        let mut s = got.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), got.len(), "trida {c}: dva stejne objekty");
        for w in s.windows(2) {
            assert!(w[1] - w[0] >= CLASSES[c], "trida {c}: objekty se prekryvaji");
        }
        let chunks = cl.chunks;
        for p in &got {
            // SAFETY: objekty prave vratil `alloc` teto tridy.
            unsafe { cl.dealloc(c, *p as *mut u8) };
        }
        // Znovu stejny pocet: bez noveho kusu.
        for _ in 0..n {
            // SAFETY: jako vyse.
            let p = unsafe { cl.alloc(c, |l| b.take(l)) };
            assert!(got.contains(&(p as usize)));
        }
        assert_eq!(cl.chunks, chunks);
    }
}

#[test]
fn refill_failure_returns_null() {
    let mut cl = Classes::new();
    // SAFETY: refill vraci null (nedostatek pameti).
    let p = unsafe { cl.alloc(3, |_| core::ptr::null_mut()) };
    assert!(p.is_null());
    assert_eq!(cl.chunks, 0);
}

#[test]
fn random_load_keeps_contents() {
    let mut b = Backing(Vec::new());
    let mut cl = Classes::new();
    // Zivy objekt -> (trida, velikost, vzor).
    let mut live: BTreeMap<usize, (usize, usize, u8)> = BTreeMap::new();
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut rnd = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for step in 0..50_000u32 {
        if live.len() < 2000 && (rnd() % 3 != 0 || live.is_empty()) {
            let size = (rnd() % 4096 + 1) as usize;
            let c = class_of(Layout::from_size_align(size, 8).unwrap()).unwrap();
            // SAFETY: refill vraci pamet z alokatoru hostitele.
            let p = unsafe { cl.alloc(c, |l| b.take(l)) };
            let pat = (step % 251) as u8;
            // SAFETY: objekt ma aspon `size` bajtu.
            unsafe { core::ptr::write_bytes(p, pat, size) };
            assert!(live.insert(p as usize, (c, size, pat)).is_none());
        } else {
            let k = *live.keys().nth((rnd() as usize) % live.len()).unwrap();
            let (c, size, pat) = live.remove(&k).unwrap();
            // SAFETY: objekt je zivy a ma `size` bajtu.
            let data = unsafe { core::slice::from_raw_parts(k as *const u8, size) };
            assert!(data.iter().all(|&v| v == pat), "obsah objektu prepsan");
            // SAFETY: objekt vratil `alloc` tridy `c`.
            unsafe { cl.dealloc(c, k as *mut u8) };
        }
    }
}

#[test]
fn heap_grows_from_small_start_logarithmically() {
    use crate::{HEAP_GROW_MIN, HEAP_INITIAL, heap_grow};
    let page = 4096;
    assert_eq!(heap_grow(100, HEAP_INITIAL, page), HEAP_GROW_MIN);
    assert_eq!(heap_grow(1 << 20, HEAP_INITIAL, page), 1 << 20, "velka alokace = jeji velikost");
    assert_eq!(heap_grow(10, 64 << 20, page), 32 << 20, "velky heap o polovinu");
    assert_eq!(heap_grow(5000, 0, 65536) % 65536, 0, "zarovnani na stranku");
    // Z 256 KiB na 256 MiB staci par desitek rustu (mapovani).
    let (mut size, mut steps) = (HEAP_INITIAL, 0);
    while size < 256 << 20 {
        size += heap_grow(4096, size, page);
        steps += 1;
    }
    assert!(steps <= 20, "{steps} rustu");
}
