//! Testy locale na hostiteli (jmena, masky, locale vlakna).

use crate::locale::*;

/// Locale vlakna na dobu `f` (globalni locale testy nemeni - bezi soubezne).
fn with_utf8<R>(f: impl FnOnce() -> R) -> R {
    // SAFETY: jmeno je retezec C; locale se uvolni na konci.
    unsafe {
        let l = newlocale(LC_ALL_MASK, c"C.UTF-8".as_ptr().cast(), core::ptr::null_mut());
        assert!(!l.is_null());
        let old = uselocale(l);
        let r = f();
        uselocale(old);
        freelocale(l);
        r
    }
}

pub(crate) fn utf8<R>(f: impl FnOnce() -> R) -> R {
    with_utf8(f)
}

#[test]
fn names_and_codesets() {
    assert_eq!(parse_name(b"C"), Some(false));
    assert_eq!(parse_name(b"POSIX"), Some(false));
    for n in [&b"C.UTF-8"[..], b"cs_CZ.UTF-8", b"en_US.utf8", b"en_US", b"cs_CZ.UTF-8@euro", b"C.utf-8"] {
        assert_eq!(parse_name(n), Some(true), "{}", String::from_utf8_lossy(n));
    }
    assert_eq!(parse_name(b"cs_CZ.ISO-8859-2"), None, "cizi kodovani se odmita");
}

#[test]
fn global_masks() {
    assert_eq!(set_mask(0, LC_ALL, b"C.UTF-8"), Some(0x3F));
    assert_eq!(set_mask(0, LC_CTYPE, b"cs_CZ.UTF-8"), Some(1));
    assert_eq!(set_mask(0x3F, LC_TIME, b"C"), Some(0x3F & !(1 << LC_TIME)));
    assert_eq!(set_mask(0, LC_ALL, b"C.UTF-8;C;C;C;C;C"), Some(1), "slozene jmeno z dotazu");
    assert_eq!(set_mask(0, LC_ALL, b"C;C"), None);
    assert_eq!(set_mask(0, 7, b"C"), None);
    assert_eq!(set_mask(0, LC_ALL, b"x.KOI8-R"), None);
    assert_eq!(new_mask(LC_CTYPE_MASK_T, b"C.UTF-8", 0), Some(1));
}

const LC_CTYPE_MASK_T: core::ffi::c_int = 1 << LC_CTYPE;

#[test]
fn thread_locale_objects() {
    // SAFETY: platne retezce C a locale z newlocale.
    unsafe {
        assert_eq!(__ctype_get_mb_cur_max(), 1, "proces zacina v locale C");
        let c = newlocale(LC_ALL_MASK, c"C".as_ptr().cast(), core::ptr::null_mut());
        let c2 = newlocale(LC_ALL_MASK, c"POSIX".as_ptr().cast(), core::ptr::null_mut());
        assert_eq!(c, c2, "jednotne locale je sdilene (bez alokace)");
        let mixed = newlocale(LC_CTYPE_MASK_T, c"C.UTF-8".as_ptr().cast(), core::ptr::null_mut());
        assert!(!mixed.is_null() && mixed != c);
        assert_eq!(uselocale(mixed), LC_GLOBAL_LOCALE);
        assert_eq!(__ctype_get_mb_cur_max(), 4);
        assert_eq!(crate::c_bytes(nl_langinfo(nl::CODESET)), b"UTF-8");
        assert_eq!(crate::c_bytes(nl_langinfo_l(nl::CODESET, c)), b"ANSI_X3.4-1968");
        let dup = duplocale(mixed);
        assert_eq!(uselocale(LC_GLOBAL_LOCALE), mixed);
        assert_eq!(__ctype_get_mb_cur_max(), 1);
        assert_eq!(get(dup).ctype(), cstd_core::mb::Enc::Utf8);
        // Zaklad z alokace se pouzije znovu.
        let back = newlocale(LC_CTYPE_MASK_T, c"C".as_ptr().cast(), dup);
        assert_eq!(back, dup);
        assert_eq!(get(back).ctype(), cstd_core::mb::Enc::C);
        freelocale(back);
        freelocale(mixed);
        freelocale(c);
        assert!(newlocale(LC_ALL_MASK, c"cs_CZ.ISO-8859-2".as_ptr().cast(), core::ptr::null_mut()).is_null());
        assert_eq!(crate::errno::get(), crate::errno::ENOENT);
        assert!(newlocale(LC_ALL_MASK, core::ptr::null(), core::ptr::null_mut()).is_null());
    }
}

#[test]
fn locale_header_matches() {
    let h = include_str!("../include/locale.h");
    for (name, v) in [("LC_CTYPE", LC_CTYPE), ("LC_NUMERIC", LC_NUMERIC), ("LC_TIME", LC_TIME), ("LC_COLLATE", LC_COLLATE), ("LC_MONETARY", LC_MONETARY), ("LC_MESSAGES", LC_MESSAGES), ("LC_ALL", LC_ALL)] {
        assert!(h.contains(&format!("#define {name} {v}\n")), "{name}");
    }
    assert!(h.contains(&format!("#define LC_ALL_MASK 0x{:x}\n", LC_ALL_MASK)));
    assert_eq!(size_of::<lconv>(), 10 * 8 + 14 + 2, "struct lconv: 10 ukazatelu, 14 znaku, zarovnani");
}
