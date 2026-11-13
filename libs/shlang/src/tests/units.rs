//! Jednotky z tabulky (`crate::units`): minuta `m`, slozene doby,
//! frekvence, procenta, vypis a chyby.

use super::host::{fails, ok};
use crate::error::code;
use crate::units::Dim;
use crate::value::{Quantity, Value};

fn one(src: &str) -> Value {
    let out = ok(src);
    assert_eq!(out.len(), 1, "{src}: {out:?}");
    out.into_iter().next().unwrap()
}

const S: i64 = 1_000_000_000;

#[test]
fn minutes_and_compound_durations() {
    assert_eq!(one("1m"), Value::Duration(60 * S));
    assert_eq!(one("1min"), Value::Duration(60 * S));
    assert_eq!(one("1h30m"), Value::Duration(90 * 60 * S));
    assert_eq!(one("2m30s"), Value::Duration(150 * S));
    assert_eq!(one("1h30m == 90m"), Value::Bool(true));
    assert_eq!(one("-1h30m"), Value::Duration(-90 * 60 * S));
    // Poradi od vetsi k mensi, kazda jednotka jednou.
    assert_eq!(fails("30m1h").code, code::PARSE);
    assert_eq!(fails("1h1h").code, code::PARSE);
    assert_eq!(fails("1h30").code, code::PARSE);
    assert_eq!(fails("5MB2").code, code::PARSE);
}

#[test]
fn frequency_and_percent() {
    let ghz = one("2.4GHz");
    assert_eq!(ghz, Value::Quantity(Quantity { dim: Dim::Frequency, value: 2.4e9 }));
    assert_eq!(ghz.to_string(), "2.4 GHz");
    assert_eq!(one("440Hz + 60Hz").to_string(), "500 Hz");
    assert_eq!(one("1MHz / 4").to_string(), "250 kHz");
    assert_eq!(one("50%").to_string(), "50%");
    // Procento z hodnoty.
    assert_eq!(one("50% * 200MB"), Value::Size(100_000_000));
    assert_eq!(one("10% * 1h"), Value::Duration(6 * 60 * S));
    // `%` mezi cisly je dal zbytek po deleni.
    assert_eq!(one("10%3"), Value::Int(1));
    assert_eq!(one("10 % 3"), Value::Int(1));
    // Ruzne rozmery nejde scitat ani porovnat.
    assert_eq!(fails("1Hz + 1%").code, code::TYPE);
    assert_eq!(fails("1Hz < 1s").code, code::TYPE);
    assert_eq!(one("1GHz > 900MHz"), Value::Bool(true));
}

#[test]
fn unknown_unit_lists_table() {
    let e = fails("5parsecs");
    assert_eq!(e.code, code::PARSE);
    let h = e.hint.unwrap();
    assert!(h.contains("frekvence: Hz kHz MHz GHz") && h.contains("doba: ns us ms s m min"), "{h}");
}
