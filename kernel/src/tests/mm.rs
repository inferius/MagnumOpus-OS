//! Testy spravy pameti a vyjimek.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::arch;
use crate::mm::{self, PhysAddr, VirtAddr, phys};

pub const TESTS: &[(&str, fn())] = &[
    ("vyjimka breakpoint se vrati", breakpoint),
    ("buddy: alokace a uvolneni", buddy),
    ("heap: Box, Vec, String", heap_small),
    ("heap: velka alokace", heap_large),
    ("strankovani: identita od loaderu je pryc", no_identity_map),
    ("strankovani: prima mapa", direct_map),
];

fn breakpoint() {
    arch::breakpoint();
}

fn buddy() {
    let (free_before, _) = phys::stats();
    let a = phys::alloc_pages(0).expect("alokace 1 stranky");
    let b = phys::alloc_pages(3).expect("alokace 8 stranek");
    assert_eq!(
        b.as_u64() % (8 * 4096),
        0,
        "blok neni zarovnany na svou velikost"
    );
    assert_ne!(a, b);
    assert_eq!(phys::stats().0, free_before - 9);
    phys::free_pages(a, 0);
    phys::free_pages(b, 3);
    assert_eq!(
        phys::stats().0,
        free_before,
        "uvolneni nevratilo vsechny stranky"
    );
}

fn heap_small() {
    let boxed = Box::new(0xDEAD_BEEF_u64);
    assert_eq!(*boxed, 0xDEAD_BEEF);
    let v: Vec<u32> = (0..1000).collect();
    assert_eq!(v.iter().sum::<u32>(), 499_500);
    let mut s = String::from("Příliš ");
    s.push_str("žluťoučký kůň");
    assert_eq!(s.chars().count(), 20);
}

fn heap_large() {
    let (free_before, _) = phys::stats();
    let v = alloc::vec![0xA5u8; 1024 * 1024];
    assert!(v.iter().all(|&x| x == 0xA5));
    drop(v);
    assert_eq!(phys::stats().0, free_before, "velka alokace se neuvolnila");
}

fn no_identity_map() {
    // Kod jadra lezi fyzicky nekde v RAM; na jeho fyzicke adrese uz nesmi
    // byt nic namapovano (identita byla jen pro prechod z loaderu).
    let code = mm::translate(VirtAddr::new(no_identity_map as *const () as u64))
        .expect("kod neni namapovany");
    assert!(mm::translate(VirtAddr::new(code.as_u64())).is_none());
}

fn direct_map() {
    let frame = phys::alloc_frame_zeroed().expect("alokace stranky");
    let virt = frame.to_virt();
    assert_eq!(mm::translate(virt), Some(frame));
    // SAFETY: prave alokovana stranka, pristup pres primou mapu.
    unsafe { virt.as_mut_ptr::<u64>().write(42) };
    assert_eq!(PhysAddr::from_direct_map(virt), frame);
    phys::free_pages(frame, 0);
}
