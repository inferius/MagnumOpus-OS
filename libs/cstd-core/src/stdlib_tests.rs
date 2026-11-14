//! Testy prevodu textu na `double` (`strtod`).

use crate::stdlib::parse_float;

fn p(s: &str) -> (f64, usize, bool) {
    parse_float(s.as_bytes())
}

#[test]
fn decimal_forms() {
    assert_eq!(p("3.25"), (3.25, 4, false));
    assert_eq!(p("  -1.5e3xyz"), (-1500.0, 8, false));
    assert_eq!(p("+.5"), (0.5, 3, false));
    assert_eq!(p("7."), (7.0, 2, false));
    assert_eq!(p("1e"), (1.0, 1, false), "neuplny exponent se necte");
    assert_eq!(p("0.1").0, 0.1);
    assert_eq!(p("abc"), (0.0, 0, false));
    assert_eq!(p("."), (0.0, 0, false));
}

#[test]
fn special_and_range() {
    assert!(p("inf").0.is_infinite() && p("inf").1 == 3);
    assert_eq!(p("-Infinity").1, 9);
    assert!(p("nan(123)").0.is_nan() && p("nan(123)").1 == 8);
    assert!(p("NaN").0.is_nan());
    let (v, n, r) = p("1e400");
    assert!(v.is_infinite() && n == 5 && r, "preteceni = ERANGE");
    let (v, _, r) = p("1e-400");
    assert!(v == 0.0 && r, "podteceni = ERANGE");
    assert!(!p("0.000").2, "nula neni podteceni");
}

#[test]
fn hex_forms() {
    assert_eq!(p("0x1.8p3"), (12.0, 7, false));
    assert_eq!(p("0X10"), (16.0, 4, false));
    assert_eq!(p("-0x.8p-1"), (-0.25, 8, false));
    assert_eq!(p("0x"), (0.0, 1, false), "jen nula");
    assert_eq!(p("0x1p-1074").0, f64::from_bits(1));
}
