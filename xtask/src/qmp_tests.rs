//! Hledani znacek v logu seriove linky (`--repeat`: kazde kolo znovu).

use super::*;

fn found(s: Scan) -> (usize, usize, String) {
    match s {
        Scan::Found { i, at, rest } => (i, at, rest),
        other => panic!("ocekavana znacka, je {other:?}"),
    }
}

#[test]
fn scan_finds_earliest_marker_with_whole_line() {
    let log = b"x\nB 1 2\nA 3 4\n";
    let (i, at, rest) = found(scan(log, 0, &["A", "B"], &[]));
    assert_eq!((i, at, rest.as_str()), (1, 2, " 1 2"));
    // Radek bez konce: cisla jeste nemusi byt cela.
    assert_eq!(scan(b"B 1", 0, &["B"], &[]), Scan::Pending);
    // CRLF se z rest odstrani.
    assert_eq!(found(scan(b"B 5 6\r\n", 0, &["B"], &[])).2, " 5 6");
}

#[test]
fn scan_aborts_on_earlier_abort_marker() {
    assert_eq!(scan(b"KONEC\nA\n", 0, &["A"], &["KONEC"]), Scan::Aborted);
    // Abort az za hledanou znackou nevadi.
    assert_eq!(found(scan(b"A\nKONEC\n", 0, &["A"], &["KONEC"])).0, 0);
    assert_eq!(scan(b"nic\n", 0, &["A"], &["KONEC"]), Scan::Pending);
}

/// Druhe kolo `--repeat`: znacky z prvniho kola za pozici uz neplati.
#[test]
fn scan_ignores_markers_before_pos() {
    let log = b"--- kolo 1/2 ---\nSTART 1\nKROK\n--- kolo 2/2 ---\n";
    let (_, at, _) = found(scan(log, 0, &["START"], &[]));
    let pos = at + "START".len();
    let (_, k, _) = found(scan(log, pos, &["KROK"], &[]));
    let pos = k + "KROK".len();
    assert_eq!(scan(log, pos, &["START"], &[]), Scan::Pending);
    let mut log = log.to_vec();
    log.extend_from_slice(b"START 2\n");
    assert_eq!(found(scan(&log, pos, &["START"], &[])).2, " 2");
    // Pozice za koncem (log smazan po opakovanem startu QEMU) nepada.
    assert_eq!(scan(b"", 10, &["A"], &[]), Scan::Pending);
}

/// Useky skriptu: kazdy zacina svou znackou, znacky zacatku se v logu
/// navzajem nezamenite (zadna neni podretezcem jine).
#[test]
fn segment_starts_are_distinct() {
    let segs = segments(true);
    assert_eq!(segs.len(), STARTS.len());
    for a in STARTS {
        for b in STARTS {
            assert!(a == b || !b.contains(a), "'{a}' je v '{b}'");
        }
    }
}
