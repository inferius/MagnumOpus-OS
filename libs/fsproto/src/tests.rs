//! Testy kodeku `Stat` a `StatEntry` (masky dostupnosti).

use alloc::vec::Vec;

use libos::Error;
use libos::time::WallTime;

use crate::{Reader, Stat, StatEntry, Writer, avail, decode_list_stat_page, encode_list_stat_page, kind};

fn stat(known: u16, hidden: u16) -> Stat {
    Stat {
        oid: 7,
        kind: kind::DIR,
        size: 0,
        mtime: WallTime { secs: 1_700_000_000, nanos: 5 },
        ctime: WallTime { secs: 1_700_000_001, nanos: 6 },
        btime: WallTime { secs: 1_600_000_000, nanos: 0 },
        owner: 1000,
        rights: 0x1f,
        tree_bytes: 0,
        tree_files: 0,
        tree_dirs: 0,
        known,
        hidden,
    }
}

fn entry(known: u16, hidden: u16) -> StatEntry {
    StatEntry {
        name: "slozka".into(),
        kind: kind::DIR,
        oid: 9,
        size: 0,
        mtime: WallTime { secs: 1_700_000_000, nanos: 0 },
        rights: 0x03,
        tree_bytes: 0,
        tree_files: 0,
        tree_dirs: 0,
        known,
        hidden,
    }
}

fn encode_stat(s: &Stat) -> Vec<u8> {
    let mut w = Writer::new();
    s.put(&mut w);
    w.finish()
}

#[test]
fn stat_roundtrip_keeps_masks() {
    // Prazdna slozka s pravem: nuly jsou zname hodnoty.
    let known = stat(avail::ALL, 0);
    // Bez MANAGE: souhrny skryte, casy zname.
    let hidden = stat(avail::ALL & !(avail::SIZE | avail::TREE), avail::SIZE | avail::TREE);
    // Hranice datasetu: casy a souhrny nedostupne.
    let unavailable = stat(avail::RIGHTS, 0);
    for s in [known, hidden, unavailable] {
        let bytes = encode_stat(&s);
        assert_eq!(bytes.len(), Stat::SIZE);
        assert_eq!(Stat::get(&mut Reader::new(&bytes)), Ok(s));
    }
}

#[test]
fn stat_rejects_overlapping_masks() {
    let bytes = encode_stat(&stat(avail::SIZE | avail::MTIME, avail::SIZE));
    assert_eq!(Stat::get(&mut Reader::new(&bytes)), Err(Error::InvalidArgs));
}

#[test]
fn stat_rejects_unknown_bits() {
    let bytes = encode_stat(&stat(avail::ALL | 0x8000, 0));
    assert_eq!(Stat::get(&mut Reader::new(&bytes)), Err(Error::InvalidArgs));
    let bytes = encode_stat(&stat(0, 0x0100));
    assert_eq!(Stat::get(&mut Reader::new(&bytes)), Err(Error::InvalidArgs));
}

#[test]
fn stat_without_masks_is_truncated() {
    // Stara zprava bez masek (o 4 B kratsi) se neprecte.
    let bytes = encode_stat(&stat(avail::ALL, 0));
    let old = &bytes[..bytes.len() - 4];
    assert!(Stat::get(&mut Reader::new(old)).is_err());
}

#[test]
fn list_stat_page_roundtrip_keeps_masks() {
    let page = [
        entry(avail::ALL, 0),
        entry(avail::ALL & !(avail::SIZE | avail::TREE), avail::SIZE | avail::TREE),
        entry(avail::SIZE | avail::MTIME | avail::RIGHTS, 0),
    ];
    let mut w = Writer::new();
    encode_list_stat_page(&mut w, &page, true);
    let bytes = w.finish();
    // Hlavicka (dalsi u8 + pocet u32) a polozky s pevnou casti `FIXED`.
    let names: usize = page.iter().map(|e| e.name.len()).sum();
    assert_eq!(bytes.len(), 5 + page.len() * StatEntry::FIXED + names);
    let (more, got) = decode_list_stat_page(&bytes).unwrap();
    assert!(more);
    assert_eq!(got, page);
}

#[test]
fn list_stat_page_rejects_overlapping_masks() {
    let mut w = Writer::new();
    encode_list_stat_page(&mut w, &[entry(avail::TREE, avail::TREE)], false);
    assert_eq!(decode_list_stat_page(&w.finish()), Err(Error::InvalidArgs));
}

#[test]
fn acl_view_roundtrip() {
    use crate::perm::{Acl, AclView, Grant, InheritedGrant, Permissions, grant, rights};
    let v = AclView {
        perms: Permissions {
            owner: 1000,
            rights: rights::VIEW,
            acl: Some(Acl { flags: 0, grants: alloc::vec![Grant { who: 1001, rights: rights::EDIT, flags: grant::BOTH }] }),
        },
        inherited: alloc::vec![InheritedGrant { who: u64::MAX, rights: rights::RUN, from: "/home/alice".into() }],
    };
    let mut w = Writer::new();
    v.put(&mut w);
    let bytes = w.finish();
    assert_eq!(AclView::get(&mut Reader::new(&bytes)), Ok(v));
    // Prilis mnoho zdedenych grantu = chyba, ne alokace.
    let mut w = Writer::new();
    w.u64(1).u8(0).u8(0).u32(u32::MAX);
    assert_eq!(AclView::get(&mut Reader::new(&w.finish())), Err(Error::InvalidArgs));
}
