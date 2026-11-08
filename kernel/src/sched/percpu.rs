//! Data jednotlivych CPU.

use alloc::boxed::Box;
use alloc::collections::{BinaryHeap, VecDeque};
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::cmp::Ordering as CmpOrdering;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use super::thread::{QOS_CLASSES, Qos, Thread};
use crate::arch::{self, CpuLocalData as _};
use crate::sync::SpinLock;

/// Nejvic CPU (staticke pole ukazatelu, 8 B na CPU). Dnesni servery maji
/// pres 500 logickych CPU; dalsi CPU nad mez se nespusti (zaloguje se).
pub const MAX_CPUS: usize = 4096;

/// Fronty vlaken pripravenych k behu, jedna na tridu QoS.
#[derive(Default)]
pub struct RunQueue {
    pub queues: [VecDeque<Arc<Thread>>; QOS_CLASSES],
}

impl RunQueue {
    pub fn push(&mut self, t: Arc<Thread>) {
        self.queues[t.qos as usize].push_back(t);
    }

    pub fn pop(&mut self) -> Option<Arc<Thread>> {
        self.queues.iter_mut().find_map(|q| q.pop_front())
    }

    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }
}

/// Probuzeni vlakna v case `deadline` (ns od startu).
pub struct TimerEntry {
    pub deadline: u64,
    pub seq: u64,
    pub thread: Arc<Thread>,
}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.deadline == other.deadline
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    /// Obracene poradi: BinaryHeap je max-halda, chceme nejdrivejsi nahore.
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other.deadline.cmp(&self.deadline)
    }
}

#[repr(C)]
pub struct PerCpu {
    /// Ukazatel na sebe; musi byt prvni (`arch::cpu_local` ho cte primo,
    /// napr. x86 pres gs:[0]).
    self_ptr: u64,
    /// Data pro prechody do a z userspace; asembler k nim pristupuje pres
    /// per-CPU ukazatel na pevnem offsetu `arch::CPU_LOCAL_OFFSET`.
    pub arch: arch::CpuLocal,
    /// Pocitadla pozadavku na vyprazdneni TLB od jinych CPU a jejich
    /// vyrizeni (viz `mm::tlb`).
    pub tlb_requested: AtomicU64,
    pub tlb_done: AtomicU64,
    pub index: usize,
    pub hw_id: u32,
    pub rq: SpinLock<RunQueue>,
    pub timers: SpinLock<BinaryHeap<TimerEntry>>,
    current: UnsafeCell<Option<Arc<Thread>>>,
    idle: UnsafeCell<Option<Arc<Thread>>>,
    prev: UnsafeCell<Option<Arc<Thread>>>,
    pub need_resched: AtomicBool,
    pub is_idle: AtomicBool,
    pub current_qos: AtomicU8,
    pub slice_end: AtomicU64,
    /// Kdy zacalo bezet aktualni vlakno (ns; cas CPU procesu).
    pub run_start: AtomicU64,
}

// SAFETY: pole v UnsafeCell (`current`, `idle`, `prev`) cte a zapisuje jen
// CPU, kteremu struktura patri, a to se zakazanymi prerusenimi (viz
// pristupove metody). Ostatni pole jsou atomicka nebo pod zamkem.
unsafe impl Sync for PerCpu {}

const _: () = assert!(
    core::mem::offset_of!(PerCpu, arch) == arch::CPU_LOCAL_OFFSET,
    "asembler syscallu ocekava arch data na pevnem offsetu"
);

static CPUS: [AtomicPtr<PerCpu>; MAX_CPUS] =
    [const { AtomicPtr::new(core::ptr::null_mut()) }; MAX_CPUS];
static CPU_COUNT: AtomicUsize = AtomicUsize::new(0);

impl PerCpu {
    /// Vytvori data CPU, nastavi je jako per-CPU data aktualniho CPU
    /// a zaregistruje je. `idle` je vlakno reprezentujici prave bezici kontext.
    pub fn install(index: usize, hw_id: u32, idle: Arc<Thread>) -> &'static PerCpu {
        assert!(index < MAX_CPUS, "prilis mnoho CPU");
        let cpu = Box::leak(Box::new(PerCpu {
            self_ptr: 0,
            arch: arch::CpuLocal::for_this_cpu(),
            tlb_requested: AtomicU64::new(0),
            tlb_done: AtomicU64::new(0),
            index,
            hw_id,
            rq: SpinLock::new(RunQueue::default()),
            timers: SpinLock::new(BinaryHeap::new()),
            current: UnsafeCell::new(Some(idle.clone())),
            idle: UnsafeCell::new(Some(idle)),
            prev: UnsafeCell::new(None),
            need_resched: AtomicBool::new(false),
            is_idle: AtomicBool::new(true),
            current_qos: AtomicU8::new(Qos::Idle as u8),
            slice_end: AtomicU64::new(u64::MAX),
            run_start: AtomicU64::new(0),
        }));
        cpu.self_ptr = cpu as *const PerCpu as u64;
        // SAFETY: struktura je leaknuta (zije navzdy) a jeji prvni slovo
        // ukazuje na ni samu.
        unsafe { arch::set_cpu_local(cpu.self_ptr) };
        CPUS[index].store(cpu, Ordering::Release);
        CPU_COUNT.fetch_max(index + 1, Ordering::AcqRel);
        cpu
    }

    /// Aktualni vlakno. Volat se zakazanymi prerusenimi.
    pub fn current(&self) -> &Arc<Thread> {
        debug_assert!(!arch::interrupts::are_enabled());
        // SAFETY: viz `unsafe impl Sync`; volajici ma zakazana preruseni
        // a jde o data vlastniho CPU.
        unsafe { (*self.current.get()).as_ref().unwrap() }
    }

    pub fn idle(&self) -> &Arc<Thread> {
        // SAFETY: `idle` se po `install` uz nemeni.
        unsafe { (*self.idle.get()).as_ref().unwrap() }
    }

    /// # Safety
    ///
    /// Volat jen na vlastnim CPU se zakazanymi prerusenimi.
    pub unsafe fn set_current(&self, t: Arc<Thread>) {
        // SAFETY: predano volajicim.
        unsafe { *self.current.get() = Some(t) };
    }

    /// # Safety
    ///
    /// Volat jen na vlastnim CPU se zakazanymi prerusenimi.
    pub unsafe fn set_prev(&self, t: Arc<Thread>) {
        // SAFETY: predano volajicim.
        unsafe { *self.prev.get() = Some(t) };
    }

    /// # Safety
    ///
    /// Volat jen na vlastnim CPU se zakazanymi prerusenimi.
    pub unsafe fn take_prev(&self) -> Option<Arc<Thread>> {
        // SAFETY: predano volajicim.
        unsafe { (*self.prev.get()).take() }
    }
}

/// Data aktualniho CPU, pokud uz je zaregistrovane (pred `install` vraci
/// `None`; vyzaduje `arch::clear_cpu_local` na zacatku startu CPU).
pub fn try_this_cpu() -> Option<&'static PerCpu> {
    let p = arch::cpu_local() as *const PerCpu;
    // SAFETY: nenulova hodnota je vzdy leaknuta PerCpu z `install`.
    (!p.is_null()).then(|| unsafe { &*p })
}

/// Data aktualniho CPU.
pub fn this_cpu() -> &'static PerCpu {
    // SAFETY: GS baze ukazuje na leaknutou PerCpu (nastaveno v `install`).
    unsafe { &*(arch::cpu_local() as *const PerCpu) }
}

pub fn cpu(index: usize) -> Option<&'static PerCpu> {
    let p = CPUS.get(index)?.load(Ordering::Acquire);
    // SAFETY: registrovane PerCpu jsou leaknute.
    (!p.is_null()).then(|| unsafe { &*p })
}

pub fn cpu_count() -> usize {
    CPU_COUNT.load(Ordering::Acquire)
}

pub fn all_cpus() -> impl Iterator<Item = &'static PerCpu> {
    (0..cpu_count()).filter_map(cpu)
}
