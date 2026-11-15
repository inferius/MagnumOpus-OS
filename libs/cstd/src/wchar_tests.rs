//! Testy sirokych znaku na hostiteli (prevody, retezce, cisla, tridy,
//! siroky scanf a proud). Kodovani UTF-8 jen pres locale vlakna
//! (`uselocale`), globalni locale testy nemeni.

use core::ptr;

use crate::locale_tests::utf8;
use crate::wchar::conv::*;
use crate::wchar::num::*;
use crate::wchar::wcs::*;
use crate::wchar::wctype::*;
use crate::wchar::{WEOF, mbstate_t, wchar_t};

fn w(s: &str) -> Vec<wchar_t> {
    s.chars().map(|c| c as wchar_t).chain([0]).collect()
}

fn ws(v: &[wchar_t]) -> String {
    v.iter().take_while(|&&c| c != 0).map(|&c| char::from_u32(c as u32).unwrap()).collect()
}

#[test]
fn mbrtowc_and_wcrtomb_utf8() {
    utf8(|| unsafe {
        let mut st = mbstate_t::default();
        let mut wc: wchar_t = 0;
        let s = "č€".as_bytes();
        assert_eq!(mbrtowc(&mut wc, s.as_ptr(), 1, &mut st), usize::MAX - 1, "neuplny znak");
        assert_eq!(mbsinit(&st), 0);
        assert_eq!(mbrtowc(&mut wc, s[1..].as_ptr(), 4, &mut st), 1);
        assert_eq!(wc, 0x10D);
        assert_eq!(mbrtowc(&mut wc, s[2..].as_ptr(), 3, &mut st), 3);
        assert_eq!(wc, 0x20AC);
        assert_eq!(mbrtowc(&mut wc, b"\xff".as_ptr(), 1, &mut st), usize::MAX);
        assert_eq!(crate::errno::get(), crate::errno::EILSEQ);
        assert_eq!(mbrlen("ž".as_ptr(), 2, ptr::null_mut()), 2);
        let mut buf = [0u8; 4];
        assert_eq!(wcrtomb(buf.as_mut_ptr(), 0x1F600, &mut st), 4);
        assert_eq!(&buf, "😀".as_bytes());
        assert_eq!(wcrtomb(buf.as_mut_ptr(), 0xD800, &mut st), usize::MAX);
        assert_eq!(mbtowc(&mut wc, "Ř".as_ptr(), 4), 2);
        assert_eq!(wc, 0x158);
        assert_eq!(mblen(b"\xc4".as_ptr(), 1), -1);
        assert_eq!(wctomb(buf.as_mut_ptr(), 0xE9), 2);
        assert_eq!(btowc(0xE9), WEOF);
        assert_eq!(wctob(0x41), 0x41);
        assert_eq!(wctob(0x10D), -1);
    });
}

#[test]
fn c_locale_is_single_byte() {
    // SAFETY: platne ukazatele.
    unsafe {
        let mut wc: wchar_t = 0;
        assert_eq!(mbrtowc(&mut wc, b"\xe9".as_ptr(), 1, ptr::null_mut()), 1);
        assert_eq!(wc as u32, 0xDFE9);
        assert_eq!(btowc(0xE9), 0xDFE9);
        assert_eq!(wctob(0xDFE9), 0xE9);
        let mut buf = [0u8; 4];
        assert_eq!(wcrtomb(buf.as_mut_ptr(), 0x10D, ptr::null_mut()), usize::MAX, "c s hackem v locale C nejde");
    }
}

#[test]
fn string_conversions() {
    utf8(|| unsafe {
        // mbsrtowcs: cely retezec, *src = NULL na konci.
        let src = "žluťoučký\0";
        let mut p = src.as_ptr();
        let mut out = [0 as wchar_t; 16];
        assert_eq!(mbsrtowcs(out.as_mut_ptr(), &mut p, 16, ptr::null_mut()), 9);
        assert!(p.is_null());
        assert_eq!(ws(&out), "žluťoučký");
        // Jen pocitani.
        let mut p = src.as_ptr();
        assert_eq!(mbsrtowcs(ptr::null_mut(), &mut p, 0, ptr::null_mut()), 9);
        // Omezeny pocet znaku: *src za posledni prevedeny.
        let mut p = src.as_ptr();
        assert_eq!(mbsrtowcs(out.as_mut_ptr(), &mut p, 2, ptr::null_mut()), 2);
        assert_eq!(p, src.as_ptr().add(3), "z (2 B) + l");
        // mbsnrtowcs: zdroj konci uprostred znaku -> stav si ho pamatuje.
        let mut st = mbstate_t::default();
        let mut p = src.as_ptr();
        assert_eq!(mbsnrtowcs(out.as_mut_ptr(), &mut p, 1, 16, &mut st), 0);
        assert_eq!(mbsinit(&st), 0);
        assert_eq!(mbsnrtowcs(out.as_mut_ptr(), &mut p, 1, 16, &mut st), 1);
        assert_eq!(out[0] as u32, 0x17E);
        // wcsrtombs: znak, ktery se nevejde, se nezapise.
        let wide = w("ča");
        let mut q = wide.as_ptr();
        let mut b = [0u8; 8];
        assert_eq!(wcsrtombs(b.as_mut_ptr(), &mut q, 1, ptr::null_mut()), 0);
        assert_eq!(q, wide.as_ptr());
        assert_eq!(wcsrtombs(b.as_mut_ptr(), &mut q, 8, ptr::null_mut()), 3);
        assert!(q.is_null());
        assert_eq!(&b[..4], "ča\0".as_bytes());
        assert_eq!(wcstombs(ptr::null_mut(), wide.as_ptr(), 0), 3);
        let mut q = wide.as_ptr();
        assert_eq!(wcsnrtombs(b.as_mut_ptr(), &mut q, 1, 8, ptr::null_mut()), 2);
        assert_eq!(q, wide.as_ptr().add(1));
        assert_eq!(mbstowcs(out.as_mut_ptr(), b"a\xffb\0".as_ptr(), 16), usize::MAX);
    });
}

#[test]
fn wide_strings() {
    let a = w("Příliš žluťoučký kůň");
    let b = w("PŘÍLIŠ ŽLUŤOUČKÝ KŮŇ");
    let kun = w("kůň");
    let delim = w(" ");
    // SAFETY: retezce zakoncene nulou.
    unsafe {
        assert_eq!(wcslen(a.as_ptr()), 20);
        assert_eq!(wcsnlen(a.as_ptr(), 5), 5);
        assert_ne!(wcscmp(a.as_ptr(), b.as_ptr()), 0);
        assert_eq!(wcscasecmp(a.as_ptr(), b.as_ptr()), 0);
        assert_eq!(wcsncmp(a.as_ptr(), b.as_ptr(), 1), 0);
        let found = wcsstr(a.as_ptr(), kun.as_ptr());
        assert_eq!(found as *const wchar_t, a.as_ptr().add(17));
        assert_eq!(wcschr(a.as_ptr(), 'ž' as wchar_t) as *const wchar_t, a.as_ptr().add(7));
        assert_eq!(wcsrchr(a.as_ptr(), ' ' as wchar_t) as *const wchar_t, a.as_ptr().add(16));
        assert_eq!(wcsrchr(a.as_ptr(), 0) as *const wchar_t, a.as_ptr().add(20));
        assert!(wcschr(a.as_ptr(), 'x' as wchar_t).is_null());
        assert_eq!(wcsspn(a.as_ptr(), w("Pří").as_ptr()), 3);
        assert_eq!(wcscspn(a.as_ptr(), delim.as_ptr()), 6);
        let mut t = a.clone();
        let mut save = ptr::null_mut();
        let mut words = Vec::new();
        let mut p = wcstok(t.as_mut_ptr(), delim.as_ptr(), &mut save);
        while !p.is_null() {
            words.push(ws(core::slice::from_raw_parts(p, wcslen(p) + 1)));
            p = wcstok(ptr::null_mut(), delim.as_ptr(), &mut save);
        }
        assert_eq!(words, ["Příliš", "žluťoučký", "kůň"]);
        let mut d = [0 as wchar_t; 8];
        assert_eq!(wcslcpy(d.as_mut_ptr(), a.as_ptr(), 8), 20);
        assert_eq!(ws(&d), "Příliš ");
        let mut d = [7 as wchar_t; 6];
        wcsncpy(d.as_mut_ptr(), kun.as_ptr(), 6);
        assert_eq!(&d[3..], &[0, 0, 0]);
        assert!(wmemcmp(a.as_ptr(), b.as_ptr(), 1) == 0 && wmemcmp(a.as_ptr(), b.as_ptr(), 2) > 0);
        wmemset(d.as_mut_ptr(), 'x' as wchar_t, 2);
        assert_eq!(ws(&d[..3]), "xxň");
        let mut x = [0 as wchar_t; 4];
        assert_eq!(wcsxfrm(x.as_mut_ptr(), kun.as_ptr(), 4), 3);
        assert_eq!(wcscoll(a.as_ptr(), a.as_ptr()), 0);
    }
}

#[test]
fn wide_numbers_and_time() {
    // SAFETY: retezce zakoncene nulou.
    unsafe {
        let s = w("\u{3000} -0x1Fzz");
        let mut end = ptr::null_mut();
        assert_eq!(wcstol(s.as_ptr(), &mut end, 0), -31);
        assert_eq!(end as *const wchar_t, s.as_ptr().add(7));
        let s = w("nic");
        assert_eq!(wcstoul(s.as_ptr(), &mut end, 10), 0);
        assert_eq!(end as *const wchar_t, s.as_ptr(), "zadne cislo = zacatek");
        let s = w("99999999999999999999");
        assert_eq!(wcstoll(s.as_ptr(), ptr::null_mut(), 10), i64::MAX);
        assert_eq!(crate::errno::get(), crate::errno::ERANGE);
        let s = w("  3.5e2ž");
        assert_eq!(wcstod(s.as_ptr(), &mut end), 350.0);
        assert_eq!(end as *const wchar_t, s.as_ptr().add(7));
        assert_eq!(wcstof(w("0.25").as_ptr(), ptr::null_mut()), 0.25);
    }
    let t = crate::time::tm { tm_year: 126, tm_mon: 10, tm_mday: 15, tm_hour: 9, tm_min: 5, ..Default::default() };
    let f: Vec<u32> = "%Y-%m-%d ž %H:%M %%".chars().map(|c| c as u32).collect();
    let out: String = format_tm_wide(&f, &t).iter().map(|&c| char::from_u32(c).unwrap()).collect();
    assert_eq!(out, "2026-11-15 ž 09:05 %");
}

#[test]
fn classes_and_case() {
    let c = |ch: char| ch as u32;
    assert_eq!(iswalpha(c('č')), 1);
    assert_eq!(iswupper(c('Č')), 1);
    assert_eq!(towupper(c('ř')), c('Ř'));
    assert_eq!(towlower(c('Ů')), c('ů'));
    assert_eq!(towupper(WEOF), WEOF);
    assert_eq!(iswdigit(0x0663), 0);
    assert_eq!(iswspace(0x3000), 1);
    assert_eq!(iswspace(0xA0), 0);
    assert_eq!(iswpunct(c('€')), 1);
    assert_eq!(iswprint(0x0378), 0);
    // SAFETY: retezce C.
    unsafe {
        let alpha = wctype(c"alpha".as_ptr().cast());
        assert_ne!(alpha, 0);
        assert_eq!(iswctype(c('ž'), alpha), 1);
        assert_eq!(wctype(c"nic".as_ptr().cast()), 0);
        assert_eq!(iswctype(c('a'), 0), 0);
        let up = wctrans(c"toupper".as_ptr().cast());
        assert_eq!(towctrans(c('ď'), up), c('Ď'));
        assert!(wctrans(c"x".as_ptr().cast()).is_null());
        let s = w("a字\u{301}");
        assert_eq!(wcswidth(s.as_ptr(), 10), 3);
    }
    assert_eq!(wcwidth('字' as wchar_t), 2);
    assert_eq!(wcwidth(0x07), -1);
}

#[derive(Default)]
struct Rec {
    ints: Vec<i64>,
    strs: Vec<Vec<u8>>,
    wide: Vec<String>,
}

impl crate::scanf::Sink for Rec {
    fn int(&mut self, _len: u8, v: i64) {
        self.ints.push(v);
    }
    fn float(&mut self, _len: u8, _v: f64) {}
    fn bytes(&mut self, b: &[u8], _nul: bool) {
        self.strs.push(b.to_vec());
    }
    fn wide(&mut self, w: &[u32], _nul: bool) {
        self.wide.push(w.iter().map(|&c| char::from_u32(c).unwrap()).collect());
    }
    fn ptr(&mut self, _v: usize) {}
}

#[test]
fn wide_scanf() {
    let u = |s: &str| -> Vec<u32> { s.chars().map(|c| c as u32).collect() };
    utf8(|| {
        let mut r = Rec::default();
        let n = crate::wchar::wscanf::scan_wide(&u("\u{3000}čaj 42 kůň-x"), &u("%ls %d %[^-]-%c"), &mut r);
        assert_eq!(n, 4);
        assert_eq!(r.wide, ["čaj"]);
        assert_eq!(r.ints, [42]);
        assert_eq!(r.strs, ["kůň".as_bytes().to_vec(), b"x".to_vec()], "%[ a %c v wscanf jsou vicebajtove");
        // Bajtovy scanf: %ls prevede z UTF-8.
        let mut r = Rec::default();
        assert_eq!(crate::scanf::scan_bytes("žába 7".as_bytes(), b"%ls %d", &mut r), 2);
        assert_eq!(r.wide, ["žába"]);
        assert_eq!(r.ints, [7]);
        let mut r = Rec::default();
        assert_eq!(crate::scanf::scan_bytes(b"a\xffb", b"%ls", &mut r), 0, "neplatne UTF-8");
    });
}

#[test]
fn wide_stream_roundtrip() {
    use crate::stdio::*;
    use crate::wchar::wio::*;
    let dir = std::env::temp_dir().join(format!("cstd-wide-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("w.txt");
    let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
    utf8(|| unsafe {
        let f = fopen(cpath.as_ptr().cast(), c"w+".as_ptr().cast());
        assert!(!f.is_null());
        assert_eq!(fwide(f, 0), 0);
        assert_eq!(fputws(w("čaj\nkůň\n").as_ptr(), f), 0);
        assert!(fwide(f, 0) > 0, "siroka operace nastavi orientaci");
        assert_eq!(fwide(f, -1), 1, "orientace se uz nemeni");
        assert_eq!(fputwc('😀' as wchar_t, f), 0x1F600);
        rewind(f);
        assert_eq!(fgetwc(f), 'č' as u32);
        assert_eq!(ungetwc('Č' as u32, f), 'Č' as u32);
        let mut line = [0 as wchar_t; 16];
        assert!(!fgetws(line.as_mut_ptr(), 16, f).is_null());
        assert_eq!(ws(&line), "Čaj\n");
        assert!(!fgetws(line.as_mut_ptr(), 3, f).is_null());
        assert_eq!(ws(&line), "ků", "nejvys n - 1 znaku");
        assert_eq!(fgetwc(f), 'ň' as u32);
        assert_eq!(fgetwc(f), '\n' as u32);
        assert_eq!(fgetwc(f), 0x1F600);
        assert_eq!(fgetwc(f), WEOF);
        assert_eq!(fclose(f), 0);
        let data = std::fs::read(&path).unwrap();
        assert_eq!(data, "čaj\nkůň\n😀".as_bytes());
    });
    let _ = std::fs::remove_dir_all(&dir);
}
