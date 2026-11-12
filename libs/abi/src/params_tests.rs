//! Testy bloku parametru procesu.

use crate::params::{Params, encode, encoded_len};

fn build(cwd: &[u8], args: &[&[u8]], env: &[&[u8]]) -> Vec<u8> {
    let mut out = vec![0; encoded_len(cwd, args.iter().copied(), env.iter().copied())];
    encode(&mut out, cwd, args.iter().copied(), env.iter().copied()).unwrap();
    out
}

#[test]
fn roundtrip() {
    let data = build(b"/home/anna", &[b"hello-std", b"--x", b""], &[b"A=1", b"PATH=/bin"]);
    let p = Params::parse(&data).unwrap();
    assert_eq!(p.cwd, b"/home/anna");
    assert_eq!(p.args().collect::<Vec<_>>(), [&b"hello-std"[..], b"--x", b""]);
    assert_eq!(p.env().collect::<Vec<_>>(), [&b"A=1"[..], b"PATH=/bin"]);
}

#[test]
fn empty() {
    let data = build(b"", &[], &[]);
    let p = Params::parse(&data).unwrap();
    assert_eq!(p.cwd, b"");
    assert_eq!(p.args().count(), 0);
    assert_eq!(p.env().count(), 0);
}

#[test]
fn corrupted_rejected() {
    let data = build(b"/", &[b"a", b"bb"], &[b"C=3"]);
    // Kazde zkraceni bloku = neplatny blok, zadna panika.
    for n in 0..data.len() {
        assert!(Params::parse(&data[..n]).is_none(), "delka {n}");
    }
    // Pocet polozek vetsi nez data.
    let mut bad = data.clone();
    bad[12] = 200;
    assert!(Params::parse(&bad).is_none());
    // Delka retezce za koncem.
    let mut bad = data.clone();
    bad[20] = 0xFF;
    assert!(Params::parse(&bad).is_none());
    let mut bad = data;
    bad[0] = b'X';
    assert!(Params::parse(&bad).is_none());
}

#[test]
fn trailing_data_ignored() {
    let mut data = build(b"/", &[b"a"], &[]);
    let size = data.len();
    data.resize(4096, 0xAA);
    assert_eq!(Params::size(&data), Some(size));
    assert_eq!(Params::parse(&data).unwrap().args().collect::<Vec<_>>(), [&b"a"[..]]);
}

#[test]
fn wrong_output_size() {
    let mut out = [0u8; 10];
    assert!(encode(&mut out, b"/", [&b"a"[..]], []).is_none());
}
