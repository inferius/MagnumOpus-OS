//! Hlavicka `include/magnumopus/sys.h` musi sedet s jadrem (kody chyb,
//! signaly) a s `consts`.

use abi::{Error, Signals};

use crate::consts::*;
use crate::deadline;

fn header() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/include/magnumopus/sys.h")).unwrap()
}

/// Hodnota `#define NAME hodnota` (bez zavorek a pripony `u`).
fn define(h: &str, name: &str) -> i64 {
    let line = h.lines().find(|l| l.starts_with(&format!("#define {name} "))).unwrap_or_else(|| panic!("{name} chybi"));
    let v = line.split_whitespace().nth(2).unwrap().trim_matches(|c| c == '(' || c == ')').trim_end_matches('u');
    match v.strip_prefix("0x") {
        Some(hex) => i64::from_str_radix(hex, 16).unwrap(),
        None => v.parse().unwrap(),
    }
}

#[test]
fn header_matches_kernel() {
    let h = header();
    for (name, e) in [
        ("MO_ERR_TIMED_OUT", Error::TimedOut),
        ("MO_ERR_NO_MEMORY", Error::NoMemory),
        ("MO_ERR_INVALID_ARGS", Error::InvalidArgs),
        ("MO_ERR_SHOULD_WAIT", Error::ShouldWait),
        ("MO_ERR_PEER_CLOSED", Error::PeerClosed),
        ("MO_ERR_BUFFER_TOO_SMALL", Error::BufferTooSmall),
    ] {
        assert_eq!(define(&h, name), e.code() as i64, "{name}");
    }
    for (name, s, c) in [
        ("MO_SIG_SIGNALED", Signals::SIGNALED, SIG_SIGNALED),
        ("MO_SIG_TERMINATED", Signals::TERMINATED, SIG_TERMINATED),
        ("MO_SIG_READABLE", Signals::READABLE, SIG_READABLE),
        ("MO_SIG_PEER_CLOSED", Signals::PEER_CLOSED, SIG_PEER_CLOSED),
    ] {
        assert_eq!(define(&h, name), s.0 as i64, "{name}");
        assert_eq!(c, s.0, "{name}");
    }
    assert_eq!(define(&h, "MO_DEADLINE_INFINITE"), DEADLINE_INFINITE);
    assert_eq!(define(&h, "MO_DEADLINE_NOW"), DEADLINE_NOW);
}

#[test]
fn deadlines() {
    assert_eq!(deadline(-1), abi::syscall::DEADLINE_INFINITE);
    assert_eq!(deadline(0), 0);
    assert_eq!(deadline(5_000), 5_000);
}
