//! Topologie CPU pro planovac: typ jadra, kapacita, cluster.
//!
//! Typ jadra zjisti kazde CPU samo pri registraci (`arch::core_kind`:
//! CPUID 0x1A, MIDR). Kapacity se prepocitaji po kazdem novem CPU:
//! system je hybridni, jen kdyz ma vic znamych typu jader. Presnejsi
//! kapacitu dodava sluzba `acpid` z ACPI CPPC (`CPU_HINT`). Clustery
//! z tabulky PPTT (podle ACPI Processor UID z MADT).
//!
//! QEMU hybridni topologii neumi: na x86_64 i aarch64 `virt` vsechna jadra
//! vraceji stejny typ (homogenni system, kapacita 1024). Hybridni chovani
//! je overene jen simulaci v `libs/schedcore`.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use abi::sched::CAPACITY_MAX;
use schedcore::topo::{default_capacity, is_hybrid};

use super::percpu::{self, MAX_CPUS, PerCpu};
use crate::acpi::Platform;
use crate::log;
use crate::sync::SpinLock;

/// Kapacita z `CPU_HINT` (0 = odhad podle typu jadra).
static HINTS: [AtomicU32; MAX_CPUS] = [const { AtomicU32::new(0) }; MAX_CPUS];
static HYBRID: AtomicBool = AtomicBool::new(false);
/// Hardwarove ID -> cluster (z PPTT).
static CLUSTERS: SpinLock<Vec<(u32, u32)>> = SpinLock::new(Vec::new());

/// Zapamatuje clustery z PPTT (pred startem dalsich CPU).
pub fn set_platform(platform: &Platform) {
    let mut map = CLUSTERS.lock();
    map.clear();
    for cpu in &platform.cpus {
        if let Some(p) = platform.pptt.iter().find(|p| p.uid == cpu.uid) {
            map.push((cpu.hw_id, p.cluster));
        }
    }
    drop(map);
    // Bootovaci CPU uz je zaregistrovane.
    for cpu in percpu::all_cpus() {
        cpu.cluster.store(cluster_of(cpu.hw_id), Ordering::Relaxed);
    }
}

fn cluster_of(hw_id: u32) -> u32 {
    CLUSTERS
        .lock()
        .iter()
        .find(|&&(h, _)| h == hw_id)
        .map_or(0, |&(_, c)| c)
}

/// Zjisti typ jadra prave registrovaneho CPU (vola ho to CPU samo).
pub(super) fn init_this_cpu(cpu: &PerCpu) {
    cpu.kind.store(crate::arch::core_kind(), Ordering::Relaxed);
    cpu.cluster.store(cluster_of(cpu.hw_id), Ordering::Relaxed);
    recompute();
}

/// Prepocita kapacity vsech CPU.
fn recompute() {
    let kinds_differ = is_hybrid(percpu::all_cpus().map(|c| c.kind.load(Ordering::Relaxed)));
    let (mut min, mut max) = (u32::MAX, 0);
    for cpu in percpu::all_cpus() {
        let hint = HINTS[cpu.index].load(Ordering::Relaxed);
        let cap = if hint != 0 {
            hint
        } else {
            default_capacity(cpu.kind.load(Ordering::Relaxed), kinds_differ)
        };
        (min, max) = (min.min(cap), max.max(cap));
        cpu.capacity.store(cap, Ordering::Relaxed);
    }
    let hybrid = kinds_differ || min < max;
    let was = HYBRID.swap(hybrid, Ordering::Relaxed);
    if hybrid && !was {
        log::info!("planovac: hybridni CPU (vykonna a usporna jadra)");
    }
}

/// Je system hybridni (vic typu jader nebo ruzne kapacity z CPPC)?
pub fn hybrid() -> bool {
    HYBRID.load(Ordering::Relaxed)
}

/// Kapacita CPU `index` z ACPI CPPC (`0` = zpet na odhad).
pub fn set_hint(index: usize, capacity: u32) -> Result<(), crate::error::Error> {
    if percpu::cpu(index).is_none() || capacity > CAPACITY_MAX {
        return Err(crate::error::Error::InvalidArgs);
    }
    HINTS[index].store(capacity, Ordering::Relaxed);
    recompute();
    Ok(())
}

/// Udaje o CPU pro `CPU_INFO`.
pub fn cpu_info(index: usize) -> Option<abi::sched::CpuInfo> {
    let cpu = percpu::cpu(index)?;
    let run_ns = cpu.rq.lock().run_ns();
    Some(abi::sched::CpuInfo {
        kind: cpu.kind.load(Ordering::Relaxed),
        _reserved: [0; 3],
        capacity: cpu.capacity.load(Ordering::Relaxed),
        cluster: cpu.cluster.load(Ordering::Relaxed),
        hw_id: cpu.hw_id,
        run_ns,
        idle_ns: cpu.idle_ns.load(Ordering::Relaxed),
        _reserved2: [0],
    })
}
