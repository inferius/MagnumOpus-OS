//! Topologie CPU: typ jadra, kapacita, clustery.
//!
//! - x86_64 Intel hybrid: `CPUID.07H:EDX[15]` = hybridni procesor,
//!   `CPUID.1AH:EAX[31:24]` = typ jadra (0x40 Core = vykonne, 0x20 Atom =
//!   usporne). Cte se na kazdem CPU zvlast (kazde vraci svuj typ).
//! - aarch64: `MIDR_EL1` (vyrobce 0x41 = ARM, cislo jadra) podle tabulky.
//! - ACPI PPTT: strom procesor -> jadro -> cluster -> balicek; vlakno
//!   (SMT) a jadro pod stejnym clusterem sdili cache.
//!
//! Kapacita: 1024 = nejvykonnejsi jadro; usporne jadro 512 (odhad, presne
//! cislo dodava ACPI CPPC pres sluzbu `acpid`). Kdyz maji vsechna jadra
//! stejny typ, system je homogenni (vsechna 1024).

use alloc::vec::Vec;

use abi::sched::{CAPACITY_MAX, core_kind};

/// Odhad kapacity usporneho jadra (Gracemont proti Golden Cove, Cortex-A55
/// proti A78 - radove polovina).
pub const EFFICIENCY_CAPACITY: u32 = 512;

/// Typ jadra z CPUID: `max_leaf` = `CPUID.0H:EAX`, `leaf7_edx` =
/// `CPUID.(07H,0):EDX`, `leaf1a_eax` = `CPUID.1AH:EAX`.
pub fn x86_core_kind(max_leaf: u32, leaf7_edx: u32, leaf1a_eax: u32) -> u8 {
    if max_leaf < 0x1A || leaf7_edx & (1 << 15) == 0 {
        return core_kind::UNKNOWN;
    }
    match leaf1a_eax >> 24 {
        0x40 => core_kind::PERFORMANCE,
        0x20 => core_kind::EFFICIENCY,
        _ => core_kind::UNKNOWN,
    }
}

/// Usporna jadra ARM (cislo dilu z MIDR).
const ARM_LITTLE: &[u16] = &[
    0xD01, // Cortex-A32
    0xD02, // Cortex-A34
    0xD03, // Cortex-A53
    0xD04, // Cortex-A35
    0xD05, // Cortex-A55
    0xD46, // Cortex-A510
    0xD80, // Cortex-A520
];

/// Vykonna jadra ARM.
const ARM_BIG: &[u16] = &[
    0xD07, // Cortex-A57
    0xD08, // Cortex-A72
    0xD09, // Cortex-A73
    0xD0A, // Cortex-A75
    0xD0B, // Cortex-A76
    0xD0D, // Cortex-A77
    0xD41, // Cortex-A78
    0xD44, // Cortex-X1
    0xD47, // Cortex-A710
    0xD48, // Cortex-X2
    0xD4B, // Cortex-A78C
    0xD4D, // Cortex-A715
    0xD4E, // Cortex-X3
    0xD81, // Cortex-A720
    0xD82, // Cortex-X4
    0xD85, // Cortex-X925
    0xD87, // Cortex-A725
];

/// Typ jadra z `MIDR_EL1`.
pub fn arm_core_kind(midr: u64) -> u8 {
    let implementer = (midr >> 24) & 0xFF;
    let part = ((midr >> 4) & 0xFFF) as u16;
    if implementer != 0x41 {
        return core_kind::UNKNOWN;
    }
    if ARM_LITTLE.contains(&part) {
        core_kind::EFFICIENCY
    } else if ARM_BIG.contains(&part) {
        core_kind::PERFORMANCE
    } else {
        core_kind::UNKNOWN
    }
}

/// Je system heterogenni (aspon dva ruzne zname typy jader)?
pub fn is_hybrid(kinds: impl IntoIterator<Item = u8>) -> bool {
    let mut seen = 0u8;
    for k in kinds {
        if k != core_kind::UNKNOWN {
            seen |= 1 << k;
        }
    }
    seen.count_ones() > 1
}

/// Vychozi kapacita jadra; `hybrid` = system ma vic typu jader.
pub fn default_capacity(kind: u8, hybrid: bool) -> u32 {
    if hybrid && kind == core_kind::EFFICIENCY {
        EFFICIENCY_CAPACITY
    } else {
        CAPACITY_MAX
    }
}

/// Procesor z PPTT: ACPI Processor UID a cislo clusteru (0, 1, ... v
/// poradi prvniho vyskytu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpttCpu {
    pub uid: u32,
    pub cluster: u32,
}

/// Priznaky uzlu hierarchie procesoru (typ 0).
const PPTT_ID_VALID: u32 = 1 << 1;
const PPTT_THREAD: u32 = 1 << 2;
const PPTT_LEAF: u32 = 1 << 3;

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

/// Uzel na offsetu `off`: (priznaky, rodic, UID). Jen typ 0 s delkou >= 20.
fn pptt_node(t: &[u8], off: usize) -> Option<(u32, u32, u32)> {
    if *t.get(off)? != 0 || (*t.get(off + 1)? as usize) < 20 {
        return None;
    }
    Some((u32_at(t, off + 4)?, u32_at(t, off + 8)?, u32_at(t, off + 12)?))
}

/// Uzly typu 0 tabulky: (offset, priznaky, rodic, UID).
fn pptt_nodes(t: &[u8]) -> Vec<(u32, u32, u32, u32)> {
    let mut out = Vec::new();
    let mut off = 36;
    while off + 4 <= t.len() {
        let len = t[off + 1] as usize;
        if len < 4 {
            break;
        }
        if let Some((flags, parent, uid)) = pptt_node(t, off) {
            out.push((off as u32, flags, parent, uid));
        }
        off += len;
    }
    out
}

/// Rozebere tabulku PPTT (cela tabulka vcetne hlavicky 36 B). Vadne uzly
/// preskoci; vraci listy (procesory) s platnym UID.
///
/// List = uzel s priznakem listu (PPTT rev. 2+), nebo uzel, ktery neni
/// rodicem zadneho jineho (starsi tabulky priznak nemaji).
pub fn parse_pptt(t: &[u8]) -> Vec<PpttCpu> {
    let nodes = pptt_nodes(t);
    let mut out = Vec::new();
    let mut clusters: Vec<u32> = Vec::new();
    for &(off, flags, parent, uid) in &nodes {
        let leaf = flags & PPTT_LEAF != 0 || !nodes.iter().any(|n| n.2 == off);
        if !leaf || flags & PPTT_ID_VALID == 0 {
            continue;
        }
        // Vlakno SMT: cluster je az rodic jadra.
        let mut node = parent;
        if flags & PPTT_THREAD != 0
            && let Some(&(_, _, p, _)) = nodes.iter().find(|n| n.0 == parent)
        {
            node = p;
        }
        let cluster = match clusters.iter().position(|&c| c == node) {
            Some(i) => i,
            None => {
                clusters.push(node);
                clusters.len() - 1
            }
        };
        out.push(PpttCpu {
            uid,
            cluster: cluster as u32,
        });
    }
    out
}
