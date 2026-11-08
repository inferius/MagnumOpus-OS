//! Testy retezcovych funkci, ctype, malloc a strtoul (na hostiteli).

use crate::ctype::*;
use crate::stdlib::*;
use crate::string::*;

fn c(s: &str) -> alloc::vec::Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

#[test]
fn delka_a_porovnani() {
    let a = c("ahoj");
    let b = c("ahoz");
    unsafe {
        assert_eq!(strlen(a.as_ptr()), 4);
        assert_eq!(strnlen(a.as_ptr(), 2), 2);
        assert!(strcmp(a.as_ptr(), b.as_ptr()) < 0);
        assert_eq!(strncmp(a.as_ptr(), b.as_ptr(), 3), 0);
        assert_eq!(strcmp(a.as_ptr(), a.as_ptr()), 0);
    }
}

#[test]
fn kopirovani() {
    let src = c("abc");
    let mut dst = [0xAAu8; 8];
    unsafe {
        strcpy(dst.as_mut_ptr(), src.as_ptr());
        assert_eq!(&dst[..4], b"abc\0");
        let mut d2 = [0xAAu8; 6];
        strncpy(d2.as_mut_ptr(), src.as_ptr(), 6);
        assert_eq!(&d2, b"abc\0\0\0");
        let mut d3 = [0xAAu8; 2];
        strncpy(d3.as_mut_ptr(), src.as_ptr(), 2);
        assert_eq!(&d3, b"ab");
        let mut d4 = [0u8; 16];
        strcpy(d4.as_mut_ptr(), src.as_ptr());
        strncat(d4.as_mut_ptr(), c("defgh").as_ptr(), 2);
        assert_eq!(&d4[..6], b"abcde\0");
        strcat(d4.as_mut_ptr(), c("X").as_ptr());
        assert_eq!(&d4[..7], b"abcdeX\0");
    }
}

#[test]
fn hledani() {
    let s = c("a.b.c");
    unsafe {
        assert_eq!(strchr(s.as_ptr(), b'.' as i32) as usize - s.as_ptr() as usize, 1);
        assert_eq!(strrchr(s.as_ptr(), b'.' as i32) as usize - s.as_ptr() as usize, 3);
        assert!(strchr(s.as_ptr(), b'x' as i32).is_null());
        // Hledani nuly najde konec retezce.
        assert_eq!(strchr(s.as_ptr(), 0) as usize - s.as_ptr() as usize, 5);
        assert_eq!(strstr(s.as_ptr(), c("b.c").as_ptr()) as usize - s.as_ptr() as usize, 2);
        assert!(strstr(s.as_ptr(), c("bb").as_ptr()).is_null());
        assert_eq!(memchr(s.as_ptr(), b'c' as i32, 5) as usize - s.as_ptr() as usize, 4);
        assert!(memchr(s.as_ptr(), b'c' as i32, 4).is_null());
    }
}

#[test]
fn tridy_znaku() {
    assert_eq!(isdigit(b'7' as i32), 1);
    assert_eq!(isxdigit(b'f' as i32), 1);
    assert_eq!(isspace(b'\t' as i32), 1);
    assert_eq!(isprint(0x7F), 0);
    assert_eq!(isdigit(-1), 0);
    assert_eq!(toupper(b'a' as i32), b'A' as i32);
    assert_eq!(tolower(-1), -1);
}

#[test]
fn prevody_cisel() {
    unsafe {
        let mut end = core::ptr::null_mut();
        let s = c("  0x1Fz");
        assert_eq!(strtoul(s.as_ptr(), &mut end, 0), 31);
        assert_eq!(end as usize - s.as_ptr() as usize, 6);
        assert_eq!(strtoul(c("017").as_ptr(), core::ptr::null_mut(), 0), 15);
        assert_eq!(strtol(c("-42").as_ptr(), core::ptr::null_mut(), 10), -42);
        assert_eq!(strtoull(c("99999999999999999999").as_ptr(), core::ptr::null_mut(), 10), u64::MAX);
        assert_eq!(strtoll(c("-9223372036854775808").as_ptr(), core::ptr::null_mut(), 10), i64::MIN);
        // Zadne cislo: konec = zacatek.
        let x = c("xyz");
        strtoul(x.as_ptr(), &mut end, 10);
        assert_eq!(end as *const u8, x.as_ptr());
        // "0x" bez hex cislice je nula a konec za nulou.
        let z = c("0xg");
        assert_eq!(strtoul(z.as_ptr(), &mut end, 16), 0);
        assert_eq!(end as usize - z.as_ptr() as usize, 1);
    }
}

#[test]
fn malloc_realloc_free() {
    unsafe {
        let p = malloc(10);
        assert!(!p.is_null());
        assert_eq!(p as usize % 16, 0);
        for i in 0..10 {
            *p.add(i) = i as u8;
        }
        let q = realloc(p, 1000);
        for i in 0..10 {
            assert_eq!(*q.add(i), i as u8);
        }
        free(q);
        let z = calloc(4, 8);
        assert!((0..32).all(|i| *z.add(i) == 0));
        free(z);
        assert!(calloc(usize::MAX, 2).is_null());
        assert!(malloc(usize::MAX).is_null());
        free(core::ptr::null_mut());
    }
}
