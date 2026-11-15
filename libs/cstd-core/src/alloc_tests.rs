//! Testy alokace: zarovnane bloky, `free` a `realloc` obou druhu bloku.

use crate::stdlib::{aligned_alloc, free, malloc, memalign, posix_memalign, realloc};

#[test]
fn aligned_blocks() {
    for align in [16usize, 32, 64, 4096] {
        let mut p: *mut u8 = core::ptr::null_mut();
        // SAFETY: platny vystupni ukazatel; blok se zapise a uvolni.
        unsafe {
            assert_eq!(posix_memalign(&mut p, align, 100), 0);
            assert!(!p.is_null() && p as usize % align == 0, "zarovnani {align}");
            core::ptr::write_bytes(p, 0xab, 100);
            free(p);
            let q = aligned_alloc(align, 33);
            assert_eq!(q as usize % align, 0);
            free(q);
            let r = memalign(align, 1);
            assert_eq!(r as usize % align, 0);
            free(r);
        }
    }
}

#[test]
fn bad_alignment() {
    let mut p: *mut u8 = core::ptr::null_mut();
    // SAFETY: platny vystupni ukazatel.
    unsafe {
        assert_eq!(posix_memalign(&mut p, 24, 8), 22, "neni mocnina 2");
        assert_eq!(posix_memalign(&mut p, 4, 8), 22, "mensi nez ukazatel");
        assert!(aligned_alloc(3, 8).is_null());
    }
}

#[test]
fn realloc_keeps_data() {
    // SAFETY: bloky z alokatoru teto knihovny, delky v mezich.
    unsafe {
        let a = aligned_alloc(64, 16);
        for i in 0..16 {
            a.add(i).write(i as u8);
        }
        let b = realloc(a, 1000);
        assert!(!b.is_null());
        for i in 0..16 {
            assert_eq!(b.add(i).read(), i as u8);
        }
        let c = realloc(b, 8);
        assert_eq!(c.add(7).read(), 7);
        free(c);
        let m = malloc(10);
        let n = realloc(m, 20);
        free(n);
    }
}
