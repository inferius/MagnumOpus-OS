//! Testy bloku parametru procesu.

use crate::params::{Fd, MAX_FDS, Params, encode, encoded_len, fd_kind};

fn build(cwd: &[u8], args: &[&[u8]], env: &[&[u8]]) -> Vec<u8> {
    build_fds(cwd, args, env, &[])
}

fn build_fds(cwd: &[u8], args: &[&[u8]], env: &[&[u8]], fds: &[Fd<'_>]) -> Vec<u8> {
    let mut out = vec![0; encoded_len(cwd, args.iter().copied(), env.iter().copied(), fds)];
    encode(&mut out, 0, cwd, args.iter().copied(), env.iter().copied(), fds).unwrap();
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
    bad[16] = 200;
    assert!(Params::parse(&bad).is_none());
    // Delka retezce za koncem.
    let mut bad = data.clone();
    bad[28] = 0xFF;
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
    assert!(encode(&mut out, 0, b"/", [&b"a"[..]], [], &[]).is_none());
}

#[test]
fn flags_roundtrip() {
    let cwd = b"/";
    let mut out = vec![0; encoded_len(cwd, [], [], &[])];
    encode(&mut out, crate::params::flags::STDOUT_NULL, cwd, [], [], &[]).unwrap();
    assert_eq!(Params::parse(&out).unwrap().flags, crate::params::flags::STDOUT_NULL);
}

#[test]
fn old_version_rejected() {
    let mut data = build(b"/", &[], &[]);
    data[4] = 2;
    assert!(Params::parse(&data).is_none());
}

#[test]
fn fds_roundtrip() {
    let fds = [
        Fd { fd: 5, kind: fd_kind::PIPE, status: 1, data: b"" },
        Fd { fd: 9, kind: fd_kind::FILE, status: 2, data: b"\x01\x02/data/a.txt" },
    ];
    let data = build_fds(b"/", &[b"x"], &[b"A=1"], &fds);
    let p = Params::parse(&data).unwrap();
    assert_eq!(p.args().collect::<Vec<_>>(), [&b"x"[..]]);
    assert_eq!(p.env().collect::<Vec<_>>(), [&b"A=1"[..]]);
    assert_eq!(p.fds().collect::<Vec<_>>(), fds);
    assert!(fd_kind::has_handle(fd_kind::PIPE) && fd_kind::has_handle(fd_kind::MEMORY));
    assert!(!fd_kind::has_handle(fd_kind::FILE));
    // Kazde zkraceni = neplatny blok.
    for n in 0..data.len() {
        assert!(Params::parse(&data[..n]).is_none(), "delka {n}");
    }
}

#[test]
fn too_many_fds_rejected() {
    let many = vec![Fd { fd: 3, kind: fd_kind::PIPE, status: 0, data: b"" }; MAX_FDS + 1];
    let mut out = vec![0; encoded_len(b"/", [], [], &many)];
    assert!(encode(&mut out, 0, b"/", [], [], &many).is_none());
    let mut data = build(b"/", &[], &[]);
    data[24] = (MAX_FDS + 1) as u8;
    assert!(Params::parse(&data).is_none());
}
