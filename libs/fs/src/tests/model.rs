//! Property-based test: nahodne sekvence operaci se porovnavaji s
//! jednoduchym modelem v pameti (mapa cesta -> obsah). Po kazde operaci
//! musi system i model vracet totez. Mezi operacemi: commit, fsync, "pad"
//! (pripojeni bez ulozeni = navrat k poslednimu trvalemu stavu), snapshoty
//! a jejich mazani (obsah snapshotu se kontroluje proti ulozenemu modelu).
//! Na konci musi projit kontrola konzistence.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use proptest::prelude::*;

use super::{assert_clean, new_fs_with, opts};
use crate::{Fs, Kind, MemDevice, fold_name};

#[derive(Debug, Clone)]
enum Op {
    Create {
        dir: usize,
        name: usize,
        kind_dir: bool,
    },
    Write {
        file: usize,
        offset: u32,
        len: u16,
        byte: u8,
    },
    Truncate {
        file: usize,
        size: u32,
    },
    Remove {
        path: usize,
    },
    Rename {
        from: usize,
        dir: usize,
        name: usize,
    },
    CommitRemount,
    Fsync,
    /// Pripojeni bez commitu: plati posledni commit + fsync.
    Crash,
    Snapshot,
    DeleteSnapshot {
        which: usize,
    },
}

const NAMES: [&str; 6] = ["a", "B", "dlouhe-jmeno.txt", "Žluťoučký", "x", "Data"];

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..8usize, 0..NAMES.len(), any::<bool>()).prop_map(|(dir, name, kind_dir)| Op::Create { dir, name, kind_dir }),
        4 => (0..8usize, 0..150_000u32, 0..9000u16, any::<u8>()).prop_map(|(file, offset, len, byte)| Op::Write { file, offset, len, byte }),
        1 => (0..8usize, 0..120_000u32).prop_map(|(file, size)| Op::Truncate { file, size }),
        1 => (0..8usize).prop_map(|path| Op::Remove { path }),
        1 => (0..8usize, 0..8usize, 0..NAMES.len()).prop_map(|(from, dir, name)| Op::Rename { from, dir, name }),
        1 => Just(Op::CommitRemount),
        1 => Just(Op::Fsync),
        1 => Just(Op::Crash),
        1 => Just(Op::Snapshot),
        1 => (0..8usize).prop_map(|which| Op::DeleteSnapshot { which }),
    ]
}

/// Model: cesty (slozene klice) -> (zobrazovane jmeno cesty, obsah nebo adresar).
/// (zobrazovana cesta, obsah souboru nebo None u adresare)
type Entry = (String, Option<Vec<u8>>);

#[derive(Default, Clone)]
struct Model {
    entries: BTreeMap<String, Entry>,
}

fn key(path: &str) -> String {
    path.split('/').map(fold_name).collect::<Vec<_>>().join("/")
}

impl Model {
    fn dirs(&self) -> Vec<String> {
        let mut d = alloc::vec![String::new()];
        d.extend(
            self.entries
                .iter()
                .filter(|(_, (_, c))| c.is_none())
                .map(|(_, (p, _))| p.clone()),
        );
        d
    }

    fn files(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, (_, c))| c.is_some())
            .map(|(_, (p, _))| p.clone())
            .collect()
    }

    fn all(&self) -> Vec<String> {
        self.entries.values().map(|(p, _)| p.clone()).collect()
    }
}

fn pick<T: Clone>(v: &[T], i: usize) -> Option<T> {
    if v.is_empty() {
        None
    } else {
        Some(v[i % v.len()].clone())
    }
}

fn run(ops: Vec<Op>, small_ags: bool) {
    let mut o = opts("model");
    if small_ags {
        o.ag_blocks = Some(256);
    }
    let mut fs: Fs<MemDevice> = new_fs_with(8192, &o);
    let mut m = Model::default();
    let mut durable = m.clone();
    let mut snaps: Vec<(String, Model)> = Vec::new();
    let mut snap_seq = 0;
    for op in ops {
        match op {
            Op::Create {
                dir,
                name,
                kind_dir,
            } => {
                let Some(d) = pick(&m.dirs(), dir) else {
                    continue;
                };
                let path = format!("{d}/{}", NAMES[name]);
                let kind = if kind_dir { Kind::Dir } else { Kind::File };
                let r = fs.create(&path, kind);
                match m.entries.entry(key(&path)) {
                    alloc::collections::btree_map::Entry::Occupied(_) => assert!(r.is_err()),
                    alloc::collections::btree_map::Entry::Vacant(v) => {
                        r.unwrap();
                        v.insert((path, if kind_dir { None } else { Some(Vec::new()) }));
                    }
                }
            }
            Op::Write {
                file,
                offset,
                len,
                byte,
            } => {
                let Some(f) = pick(&m.files(), file) else {
                    continue;
                };
                let data = alloc::vec![byte; len as usize];
                fs.write(&f, offset as u64, &data).unwrap();
                let content = m.entries.get_mut(&key(&f)).unwrap().1.as_mut().unwrap();
                if len > 0 {
                    let end = offset as usize + len as usize;
                    if content.len() < end {
                        content.resize(end, 0);
                    }
                    content[offset as usize..end].copy_from_slice(&data);
                }
            }
            Op::Truncate { file, size } => {
                let Some(f) = pick(&m.files(), file) else {
                    continue;
                };
                fs.truncate(&f, size as u64).unwrap();
                m.entries
                    .get_mut(&key(&f))
                    .unwrap()
                    .1
                    .as_mut()
                    .unwrap()
                    .resize(size as usize, 0);
            }
            Op::Remove { path } => {
                let Some(p) = pick(&m.all(), path) else {
                    continue;
                };
                let k = key(&p);
                let has_children = m.entries.keys().any(|c| c.starts_with(&format!("{k}/")));
                let r = fs.remove(&p);
                if has_children {
                    assert!(r.is_err());
                } else {
                    r.unwrap();
                    m.entries.remove(&k);
                }
            }
            Op::Rename { from, dir, name } => {
                let Some(src) = pick(&m.all(), from) else {
                    continue;
                };
                let Some(d) = pick(&m.dirs(), dir) else {
                    continue;
                };
                let dst = format!("{d}/{}", NAMES[name]);
                let (ks, kd) = (key(&src), key(&dst));
                let into_self = kd == ks || kd.starts_with(&format!("{ks}/"));
                let exists = m.entries.contains_key(&kd) && kd != ks;
                let r = fs.rename(&src, &dst);
                if into_self && kd != ks || exists {
                    assert!(r.is_err(), "rename {src} -> {dst} melo selhat");
                    continue;
                }
                r.unwrap();
                // Presun cele vetve v modelu.
                let moved: Vec<(String, Entry)> = m
                    .entries
                    .iter()
                    .filter(|(k, _)| **k == ks || k.starts_with(&format!("{ks}/")))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (k, _) in &moved {
                    m.entries.remove(k);
                }
                for (k, (p, c)) in moved {
                    let nk = format!("{kd}{}", &k[ks.len()..]);
                    let np = format!("{dst}{}", &p[src.len()..]);
                    m.entries.insert(nk, (np, c));
                }
            }
            Op::CommitRemount => {
                fs.commit().unwrap();
                durable = m.clone();
                fs = Fs::mount(fs.into_device()).unwrap();
            }
            Op::Fsync => {
                fs.fsync().unwrap();
                durable = m.clone();
            }
            Op::Crash => {
                fs = Fs::mount(fs.into_device()).unwrap();
                m = durable.clone();
            }
            Op::Snapshot => {
                let name = format!("snap{snap_seq}");
                snap_seq += 1;
                fs.snapshot(&name).unwrap();
                durable = m.clone();
                snaps.push((name, m.clone()));
            }
            Op::DeleteSnapshot { which } => {
                if snaps.is_empty() {
                    continue;
                }
                let (name, _) = snaps.remove(which % snaps.len());
                fs.delete_snapshot(&name).unwrap();
            }
        }
        check(&mut fs, &m);
    }
    for (name, sm) in &snaps {
        for (path, content) in sm.entries.values() {
            let st = fs.snapshot_stat(name, path).unwrap();
            if let Some(c) = content {
                assert_eq!(st.size, c.len() as u64);
                assert_eq!(
                    &fs.snapshot_read(name, path, 0, c.len() + 1).unwrap(),
                    c,
                    "snapshot {name}: {path}"
                );
            }
        }
    }
    assert_clean(&mut fs);
}

fn check(fs: &mut Fs<MemDevice>, m: &Model) {
    let mut total_bytes = 0u64;
    let mut files = 0u64;
    for (path, content) in m.entries.values() {
        let st = fs.stat(path).unwrap();
        if let Some(c) = content {
            assert_eq!(st.size, c.len() as u64, "velikost {path}");
            assert_eq!(&fs.read(path, 0, c.len() + 10).unwrap(), c, "obsah {path}");
            total_bytes += c.len() as u64;
            files += 1;
        }
    }
    let root = fs.stat("/").unwrap();
    assert_eq!(root.tree_bytes, total_bytes, "souhrn velikosti korene");
    assert_eq!(root.tree_files, files, "souhrn poctu souboru korene");
    assert_eq!(
        fs.list("/").unwrap().len(),
        m.entries.keys().filter(|k| !k[1..].contains('/')).count()
    );
}

proptest! {
    #![proptest_config(ProptestConfig { cases: super::cases(64), ..ProptestConfig::default() })]

    #[test]
    fn matches_model(ops in proptest::collection::vec(op(), 1..60), small_ags in any::<bool>()) {
        run(ops, small_ags);
    }
}
