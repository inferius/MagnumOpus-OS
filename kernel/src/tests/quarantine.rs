//! Testy karanteny DMA bez skutecneho zarizeni: rozhodovani, ktera pamet
//! se zastavuje a ktera zustava v karantene. Skutecne zastaveni (bus master
//! na zarizeni edu po padu ovladace) testuje sada `ovladace` v initu.

use alloc::vec;

use crate::arch::paging::PAGE_SIZE;
use crate::object::quarantine::{driver_gone, held_count};
use crate::object::vmo::MemoryObject;

pub const TESTS: &[(&str, fn())] = &[
    ("karantena DMA: zarizeni mimo PCI zustava, klient nic nezastavi", decisions),
];

fn decisions() {
    let before = held_count();
    // Zarizeni v segmentu, ktery nema okno ECAM: nejde zastavit, pamet
    // zustava v karantene (s varovanim v logu).
    let fake = (0xFFF0, 0, 0);
    let vmo = MemoryObject::new_contiguous_dma(PAGE_SIZE, None, Some((fake, 1)), None).unwrap();
    let frame = vmo.page(0).unwrap();
    driver_gone(&[fake], vec![vmo], vec![]);
    assert_eq!(held_count(), before + 1, "pamet nezastavitelneho zarizeni zustava");
    assert!(crate::mm::phys::is_pinned(frame), "drzena pamet je stale pripnuta");
    // Klient (zarizeni neridil): pamet se pusti hned, nic se nezastavuje.
    let other = MemoryObject::new_contiguous_dma(PAGE_SIZE, None, Some(((0xFFF1, 0, 0), 1)), None).unwrap();
    let f2 = other.page(0).unwrap();
    driver_gone(&[], vec![other], vec![]);
    assert_eq!(held_count(), before + 1, "klient nic nedrzi");
    assert!(!crate::mm::phys::is_pinned(f2), "pamet klienta uvolnena");
}
