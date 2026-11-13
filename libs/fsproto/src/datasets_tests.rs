//! Testy kodeku seznamu datasetu (`DATASETS`).

use alloc::vec;

use libos::Error;

use crate::datasets::{DatasetEntry, MAX_DATASETS, Origin, decode, encode};
use crate::{Stat, Writer, avail, kind};

fn entry(path: &str, origin: Origin, known: u16, hidden: u16) -> DatasetEntry {
    let root = Stat { oid: 1, kind: kind::DIR, tree_bytes: if known & avail::TREE != 0 { 4096 } else { 0 }, known, hidden, ..Stat::default() };
    DatasetEntry { path: path.into(), id: 7, root, origin }
}

#[test]
fn datasets_roundtrip_all_origin_states() {
    let list = vec![
        entry("/", Origin::None, avail::ALL, 0),
        entry("/home/jana/Projekty", Origin::Known("/home/jana/Fotky".into()), avail::ALL, 0),
        entry("/home/jana/Zkouska", Origin::Hidden, avail::RIGHTS, avail::TREE | avail::SIZE),
        entry("/home/jana/Stary", Origin::Gone, avail::ALL, 0),
    ];
    let mut w = Writer::new();
    encode(&mut w, &list);
    assert_eq!(decode(&w.finish()).unwrap(), list);
}

#[test]
fn datasets_rejects_bad_input() {
    // Neznamy stav puvodu.
    let mut w = Writer::new();
    encode(&mut w, &[entry("/", Origin::None, 0, 0)]);
    let mut b = w.finish();
    *b.last_mut().unwrap() = 9;
    assert_eq!(decode(&b), Err(Error::InvalidArgs));
    // Zbytek dat za seznamem.
    let mut w = Writer::new();
    encode(&mut w, &[]);
    let mut b = w.finish();
    b.push(0);
    assert_eq!(decode(&b), Err(Error::InvalidArgs));
    // Prilis mnoho polozek.
    let mut w = Writer::new();
    w.u32(MAX_DATASETS as u32 + 1);
    assert_eq!(decode(&w.finish()), Err(Error::InvalidArgs));
    // Prekryv masek known/hidden odmitne uz kodek Stat.
    let mut w = Writer::new();
    encode(&mut w, &[entry("/", Origin::None, avail::TREE, avail::TREE)]);
    assert_eq!(decode(&w.finish()), Err(Error::InvalidArgs));
}
