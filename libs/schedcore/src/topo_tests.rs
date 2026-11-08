//! Testy zjisteni typu jader a PPTT.

use abi::sched::core_kind;

use crate::topo::{arm_core_kind, default_capacity, is_hybrid, parse_pptt, x86_core_kind};

#[test]
fn x86_hybrid_decode() {
    let hybrid = 1 << 15;
    assert_eq!(x86_core_kind(0x20, hybrid, 0x4000_0001), core_kind::PERFORMANCE);
    assert_eq!(x86_core_kind(0x20, hybrid, 0x2000_0001), core_kind::EFFICIENCY);
    // Bez priznaku hybrid nebo stary procesor: nezname.
    assert_eq!(x86_core_kind(0x20, 0, 0x4000_0001), core_kind::UNKNOWN);
    assert_eq!(x86_core_kind(0x0D, hybrid, 0x4000_0001), core_kind::UNKNOWN);
}

#[test]
fn arm_midr_decode() {
    // Cortex-A55 r2p0 a Cortex-A78 r1p1.
    assert_eq!(arm_core_kind(0x412F_D050), core_kind::EFFICIENCY);
    assert_eq!(arm_core_kind(0x411F_D411), core_kind::PERFORMANCE);
    // Neoverse-N1 (server) a jiny vyrobce: nezname.
    assert_eq!(arm_core_kind(0x413F_D0C1), core_kind::UNKNOWN);
    assert_eq!(arm_core_kind(0x510F_8000), core_kind::UNKNOWN);
}

#[test]
fn capacities() {
    assert!(is_hybrid([core_kind::PERFORMANCE, core_kind::EFFICIENCY]));
    assert!(!is_hybrid([core_kind::PERFORMANCE, core_kind::PERFORMANCE, core_kind::UNKNOWN]));
    assert_eq!(default_capacity(core_kind::EFFICIENCY, true), 512);
    assert_eq!(default_capacity(core_kind::EFFICIENCY, false), 1024);
    assert_eq!(default_capacity(core_kind::PERFORMANCE, true), 1024);
}

/// Uzel hierarchie procesoru (typ 0, 20 B bez prostredku).
fn node(t: &mut Vec<u8>, flags: u32, parent: u32, uid: u32) -> u32 {
    let off = t.len() as u32;
    t.extend_from_slice(&[0, 20, 0, 0]);
    t.extend_from_slice(&flags.to_le_bytes());
    t.extend_from_slice(&parent.to_le_bytes());
    t.extend_from_slice(&uid.to_le_bytes());
    t.extend_from_slice(&0u32.to_le_bytes());
    off
}

#[test]
fn pptt_clusters() {
    let mut t = vec![0u8; 36];
    let pkg = node(&mut t, 1, 0, 0);
    let c0 = node(&mut t, 0, pkg, 0);
    let c1 = node(&mut t, 0, pkg, 1);
    // Rev. 1 bez priznaku listu: listy jsou uzly bez potomku.
    node(&mut t, 2, c0, 10);
    node(&mut t, 2, c0, 11);
    node(&mut t, 2, c1, 12);
    // Jadro se dvema vlakny SMT (priznak listu a vlakna).
    let core = node(&mut t, 0, c1, 99);
    node(&mut t, 2 | 4 | 8, core, 13);
    node(&mut t, 2 | 4 | 8, core, 14);
    let cpus = parse_pptt(&t);
    let get = |uid| cpus.iter().find(|c| c.uid == uid).unwrap().cluster;
    assert_eq!(get(10), get(11));
    assert_ne!(get(10), get(12));
    assert_eq!(get(12), get(13));
    assert_eq!(get(13), get(14));
    assert!(cpus.iter().all(|c| c.uid != 99), "jadro s vlakny neni list");
}

#[test]
fn pptt_garbage_does_not_panic() {
    let mut t = vec![0u8; 36];
    t.extend_from_slice(&[0, 2, 0, 0, 7, 7]);
    assert!(parse_pptt(&t).is_empty());
    assert!(parse_pptt(&[1, 2, 3]).is_empty());
}
