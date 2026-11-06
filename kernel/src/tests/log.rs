//! Testy logovani.

use alloc::string::String;

use crate::log::{self, Level};

pub const TESTS: &[(&str, fn())] = &[
    (
        "zaznam se ulozi do bufferu s cilem a urovni",
        record_in_ring,
    ),
    ("uroven pro modul potlaci zaznam", target_filter),
    ("prilis dlouhy radek se zkrati na hranici znaku", long_line),
];

/// Obsah bufferu logu jako text (posledni zaznamy).
fn ring_text() -> String {
    let mut text = String::new();
    log::snapshot(|chunk| text.push_str(&String::from_utf8_lossy(chunk)));
    text
}

fn record_in_ring() {
    log::info!("znacka-testu-logu-1");
    let text = ring_text();
    let line = text
        .lines()
        .rev()
        .find(|l| l.contains("znacka-testu-logu-1"))
        .expect("zaznam neni v bufferu logu");
    assert!(line.contains(" I tests::log: "), "spatny format: {line}");
    assert!(line.starts_with('['), "chybi cas: {line}");
}

fn target_filter() {
    log::set_target_level("tests::log", Level::Warn);
    log::info!("znacka-testu-logu-2");
    log::warn!("znacka-testu-logu-3");
    log::clear_target_level("tests::log");
    let text = ring_text();
    assert!(
        !text.contains("znacka-testu-logu-2"),
        "info prosel pres filtr Warn"
    );
    let warn = text
        .lines()
        .find(|l| l.contains("znacka-testu-logu-3"))
        .expect("warn se nezalogoval");
    assert!(
        warn.contains("tests/log.rs:"),
        "varovani bez mista ve zdrojaku: {warn}"
    );
}

fn long_line() {
    let long: String = core::iter::repeat_n('ž', 400).collect();
    log::info!("znacka-testu-logu-4 {long}");
    let text = ring_text();
    let line = text
        .lines()
        .find(|l| l.contains("znacka-testu-logu-4"))
        .expect("dlouhy zaznam chybi");
    assert!(
        line.ends_with("..."),
        "dlouhy radek neni oznaceny jako zkraceny"
    );
}
