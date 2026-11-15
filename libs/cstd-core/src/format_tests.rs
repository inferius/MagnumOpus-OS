//! Testy formatovani printf (na hostiteli, argumenty ze seznamu).

use alloc::string::String;
use alloc::vec::Vec;

use crate::format::{Args, Buf, Len, format};

#[derive(Clone, Copy)]
enum A<'a> {
    I(i64),
    S(Option<&'a str>),
    B(&'a [u8]),
    W(Option<&'a [u32]>),
    P(usize),
    D(f64),
}

struct List<'a>(Vec<A<'a>>, usize);

impl<'a> List<'a> {
    fn next(&mut self) -> A<'a> {
        let a = self.0[self.1];
        self.1 += 1;
        a
    }
}

impl Args for List<'_> {
    fn signed(&mut self, len: Len) -> i64 {
        match (self.next(), len) {
            (A::I(v), Len::Char) => v as i8 as i64,
            (A::I(v), Len::Short) => v as i16 as i64,
            (A::I(v), Len::Int) => v as i32 as i64,
            (A::I(v), Len::Long) => v,
            _ => panic!("cekal cislo"),
        }
    }
    fn unsigned(&mut self, len: Len) -> u64 {
        match (self.next(), len) {
            (A::I(v), Len::Char) => v as u8 as u64,
            (A::I(v), Len::Short) => v as u16 as u64,
            (A::I(v), Len::Int) => v as u32 as u64,
            (A::I(v), Len::Long) => v as u64,
            _ => panic!("cekal cislo"),
        }
    }
    fn pointer(&mut self) -> usize {
        match self.next() {
            A::P(p) => p,
            _ => panic!("cekal ukazatel"),
        }
    }
    fn string(&mut self, out: &mut dyn FnMut(u8) -> bool) -> bool {
        match self.next() {
            A::S(Some(s)) => {
                for b in s.bytes() {
                    if !out(b) {
                        break;
                    }
                }
                true
            }
            A::B(s) => {
                for &b in s {
                    if !out(b) {
                        break;
                    }
                }
                true
            }
            A::S(None) => false,
            _ => panic!("cekal retezec"),
        }
    }
    fn wstring(&mut self, out: &mut dyn FnMut(u32) -> bool) -> bool {
        match self.next() {
            A::W(Some(s)) => {
                for &c in s {
                    if c == 0 || !out(c) {
                        break;
                    }
                }
                true
            }
            A::W(None) => false,
            _ => panic!("cekal siroky retezec"),
        }
    }
    fn double(&mut self) -> f64 {
        match self.next() {
            A::D(v) => v,
            _ => panic!("cekal double"),
        }
    }
}

fn f(fmt: &str, args: &[A]) -> String {
    let mut buf = [0u8; 512];
    let mut out = Buf { buf: &mut buf, pos: 0, total: 0 };
    let mut list = List(args.to_vec(), 0);
    format(&mut out, fmt.as_bytes(), &mut list);
    assert_eq!(list.1, args.len(), "nespotrebovane argumenty u {fmt}");
    let n = out.pos;
    String::from_utf8(buf[..n].to_vec()).unwrap()
}

#[test]
fn cela_cisla() {
    assert_eq!(f("%d %i %u", &[A::I(-5), A::I(7), A::I(-1)]), "-5 7 4294967295");
    assert_eq!(f("%ld %llu", &[A::I(-5_000_000_000), A::I(-1)]), "-5000000000 18446744073709551615");
    assert_eq!(f("%hhd %hu", &[A::I(0x1ff), A::I(0x12345)]), "-1 9029");
    assert_eq!(f("%x %X %o", &[A::I(255), A::I(255), A::I(8)]), "ff FF 10");
    assert_eq!(f("%d", &[A::I(i32::MIN as i64)]), "-2147483648");
    assert_eq!(f("%ld", &[A::I(i64::MIN)]), "-9223372036854775808");
}

#[test]
fn sirka_presnost_priznaky() {
    assert_eq!(f("[%5d]", &[A::I(42)]), "[   42]");
    assert_eq!(f("[%-5d]", &[A::I(42)]), "[42   ]");
    assert_eq!(f("[%05d]", &[A::I(-42)]), "[-0042]");
    assert_eq!(f("[%+d %+d]", &[A::I(3), A::I(-3)]), "[+3 -3]");
    assert_eq!(f("[% d]", &[A::I(3)]), "[ 3]");
    assert_eq!(f("[%8.8X]", &[A::I(0xABC)]), "[00000ABC]");
    assert_eq!(f("[%.3d]", &[A::I(7)]), "[007]");
    assert_eq!(f("[%.0d]", &[A::I(0)]), "[]");
    assert_eq!(f("[%#x %#X %#o]", &[A::I(255), A::I(255), A::I(8)]), "[0xff 0XFF 010]");
    assert_eq!(f("[%#x]", &[A::I(0)]), "[0]");
    assert_eq!(f("[%*d]", &[A::I(4), A::I(1)]), "[   1]");
    assert_eq!(f("[%*d]", &[A::I(-4), A::I(1)]), "[1   ]");
    assert_eq!(f("[%.*s]", &[A::I(2), A::S(Some("abcd"))]), "[ab]");
}

#[test]
fn retezce_znaky_ukazatele() {
    assert_eq!(f("%s!", &[A::S(Some("ahoj"))]), "ahoj!");
    assert_eq!(f("[%6s][%-6s]", &[A::S(Some("ab")), A::S(Some("cd"))]), "[    ab][cd    ]");
    assert_eq!(f("%s", &[A::S(None)]), "(null)");
    assert_eq!(f("%4.4s", &[A::S(Some("_SB_X"))]), "_SB_");
    assert_eq!(f("%c%c", &[A::I(b'o' as i64), A::I(b'k' as i64)]), "ok");
    assert_eq!(f("%p", &[A::P(0x1000)]), "0x1000");
    assert_eq!(f("100%%", &[]), "100%");
}

#[test]
fn bezpecne_chovani() {
    // %n nic nezapise, jen spotrebuje argument.
    assert_eq!(f("a%nb", &[A::P(0x1234)]), "ab");
    // Neznamy prevod se vypise doslova a nic nespotrebuje.
    assert_eq!(f("%q %d", &[A::I(1)]), "%q 1");
    // Plovouci carka zatim neni: otaznik.
    assert_eq!(f("%Lf", &[]), "?");
    // Obrovska sirka je omezena (zadne gigabajty mezer).
    let s = f("%2000000000d", &[A::I(1)]);
    assert!(s.len() <= 512);
    // Format konci uprostred specifikace.
    assert_eq!(f("abc%", &[]), "abc%");
}

#[test]
fn oriznuti_a_celkova_delka() {
    let mut buf = [0u8; 4];
    let mut out = Buf { buf: &mut buf, pos: 0, total: 0 };
    format(&mut out, b"%d", &mut List(alloc::vec![A::I(123456)], 0));
    assert_eq!(out.total, 6);
    assert_eq!(out.pos, 4);
    assert_eq!(&buf, b"1234");
}

/// Ocekavane vysledky jsou z glibc (printf v Linuxu).
#[test]
fn floats_like_glibc() {
    let d = |v: f64| A::D(v);
    assert_eq!(f("%f", &[d(3.14159)]), "3.141590");
    assert_eq!(f("%.2f|%.0f|%#.0f", &[d(2.675), d(2.5), d(3.0)]), "2.67|2|3.");
    assert_eq!(f("%8.3f|%-8.1f|%+08.2f", &[d(-1.5), d(2.25), d(3.14159)]), "  -1.500|2.2     |+0003.14");
    assert_eq!(f("%e|%.2E|%.0e", &[d(12345.678), d(0.000123), d(5e300)]), "1.234568e+04|1.23E-04|5e+300");
    assert_eq!(f("%g|%g|%g|%g", &[d(100000.0), d(1000000.0), d(0.0001), d(0.00001)]), "100000|1e+06|0.0001|1e-05");
    assert_eq!(f("%g|%.3g|%#g|%G", &[d(3.14159), d(1234.5), d(1.0), d(1.5e-10)]), "3.14159|1.23e+03|1.00000|1.5E-10");
    assert_eq!(f("%.10g|%g", &[d(0.1), d(0.0)]), "0.1|0");
    assert_eq!(f("%a|%A|%.1a|%a", &[d(1.0), d(-0.5), d(1.5), d(0.0)]), "0x1p+0|-0X1P-1|0x1.8p+0|0x0p+0");
    assert_eq!(f("%f|%F|%5.1f|%e", &[d(f64::INFINITY), d(f64::NAN), d(f64::NEG_INFINITY), d(-0.0)]), "inf|NAN| -inf|-0.000000e+00");
    assert_eq!(f("%010.3e", &[d(-1.0)]), "-1.000e+00");
    assert_eq!(f("%.3f %d", &[d(1e-10), A::I(7)]), "0.000 7");
    // Velke cislo: vsechny cislice celeho cisla.
    assert_eq!(f("%.0f", &[d(1e22)]), "10000000000000000000000");
}

#[path = "format_wide_tests.rs"]
mod wide;
