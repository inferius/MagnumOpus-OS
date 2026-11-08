//! Planovani: tridy QoS vlaken, strop jobu, Realtime rozpocet, udaje o CPU.
//!
//! Vse jde pres jeden syscall `SCHED_CONTROL` (`crate::syscall`), prvni
//! argument je operace z [`op`]. Popis chovani: `docs/cs/planovac.md`.
//!
//! Trida rika, co vlakno dela (ne cislo priority). Mensi cislo = vyssi
//! trida. Ucinna trida je vzdy nejvys strop jobu procesu a vsech predku;
//! docasne ji muze zvednout zprava od vlakna vyssi tridy (dedeni pres IPC).

/// Tridy QoS.
pub mod class {
    /// Pevny rozpocet `budget` v kazde `period`; pred vsemi ostatnimi.
    pub const REALTIME: u8 = 0;
    /// Odezva na uzivatele (kompozitor, vstup); vaha 15.
    pub const INTERACTIVE: u8 = 1;
    /// Bezna prace; vaha 4.
    pub const NORMAL: u8 = 2;
    /// Udrzba a davky; vaha 1, zaruceny podil.
    pub const BACKGROUND: u8 = 3;
    /// Pocet trid (pro tabulky).
    pub const COUNT: usize = 4;
}

/// Operace `SCHED_CONTROL`.
pub mod op {
    /// `(op, thread, class, budget_us, period_us)`: trida vlakna (`MANAGE`;
    /// handle 0 = volajici vlakno). Budget a perioda jen u Realtime.
    pub const THREAD_SET: u64 = 0;
    /// `(op, process, class, budget_us, period_us)`: trida vsech vlaken
    /// procesu i budoucich (`MANAGE`).
    pub const PROCESS_SET: u64 = 1;
    /// `(op, job, class)`: strop tridy jobu (`WRITE`); nejvys strop rodice.
    pub const JOB_SET_MAX: u64 = 2;
    /// `(op, thread, out_ptr)`: zapise [`super::QosInfo`] (`READ`; 0 = sebe).
    pub const THREAD_GET: u64 = 3;
    /// `(op, cpu_index, out_ptr) -> pocet CPU`: zapise [`super::CpuInfo`].
    pub const CPU_INFO: u64 = 4;
    /// `(op, resource, cpu_index, capacity)`: kapacita jadra z ACPI CPPC
    /// (Resource s pravem `POWER`, 1..=1024; 0 = zpet na odhad z CPUID/MIDR).
    pub const CPU_HINT: u64 = 5;
}

/// Typ jadra CPU.
pub mod core_kind {
    /// Homogenni system nebo nezjisteno.
    pub const UNKNOWN: u8 = 0;
    /// Vykonne jadro (Intel Core, ARM Cortex-A7x/X).
    pub const PERFORMANCE: u8 = 1;
    /// Usporne jadro (Intel Atom, ARM Cortex-A5x/A5xx).
    pub const EFFICIENCY: u8 = 2;
}

/// Kapacita nejvykonnejsiho jadra.
pub const CAPACITY_MAX: u32 = 1024;

/// Nejkratsi a nejdelsi perioda Realtime (us).
pub const RT_PERIOD_MIN_US: u32 = 500;
pub const RT_PERIOD_MAX_US: u32 = 1_000_000;
/// Nejkratsi rozpocet Realtime (us).
pub const RT_BUDGET_MIN_US: u32 = 50;
/// Vychozi rozpocet a perioda Realtime (kdyz volajici zada 0).
pub const RT_DEFAULT_BUDGET_US: u32 = 1_000;
pub const RT_DEFAULT_PERIOD_US: u32 = 10_000;

/// Vystup `THREAD_GET` (16 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct QosInfo {
    /// Pozadovana trida vlakna.
    pub base: u8,
    /// Trida, se kterou se ted planuje (strop jobu, zvednuti pres IPC,
    /// vycerpany Realtime rozpocet).
    pub effective: u8,
    /// Strop jobu (vcetne predku).
    pub job_max: u8,
    pub _reserved: u8,
    /// Realtime rozpocet a perioda (us), jinak 0.
    pub budget_us: u32,
    pub period_us: u32,
    pub _reserved2: u32,
}

/// Vystup `CPU_INFO` (64 B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct CpuInfo {
    /// Typ jadra z [`core_kind`].
    pub kind: u8,
    pub _reserved: [u8; 3],
    /// Kapacita (1024 = nejvykonnejsi jadro systemu).
    pub capacity: u32,
    /// Cislo clusteru (sdilena cache; z PPTT, jinak 0).
    pub cluster: u32,
    /// Hardwarove ID CPU.
    pub hw_id: u32,
    /// Cas behu po tridach (ns, index = trida).
    pub run_ns: [u64; class::COUNT],
    /// Cas necinnosti (ns).
    pub idle_ns: u64,
    pub _reserved2: [u64; 1],
}

impl QosInfo {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = self.base;
        b[1] = self.effective;
        b[2] = self.job_max;
        b[4..8].copy_from_slice(&self.budget_us.to_le_bytes());
        b[8..12].copy_from_slice(&self.period_us.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            base: b[0],
            effective: b[1],
            job_max: b[2],
            _reserved: 0,
            budget_us: u32_at(4),
            period_us: u32_at(8),
            _reserved2: 0,
        }
    }
}

impl CpuInfo {
    pub const SIZE: usize = 64;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0] = self.kind;
        b[4..8].copy_from_slice(&self.capacity.to_le_bytes());
        b[8..12].copy_from_slice(&self.cluster.to_le_bytes());
        b[12..16].copy_from_slice(&self.hw_id.to_le_bytes());
        for (i, v) in self.run_ns.iter().enumerate() {
            b[16 + i * 8..24 + i * 8].copy_from_slice(&v.to_le_bytes());
        }
        b[48..56].copy_from_slice(&self.idle_ns.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        Self {
            kind: b[0],
            _reserved: [0; 3],
            capacity: u32_at(4),
            cluster: u32_at(8),
            hw_id: u32_at(12),
            run_ns: [u64_at(16), u64_at(24), u64_at(32), u64_at(40)],
            idle_ns: u64_at(48),
            _reserved2: [0],
        }
    }
}

const _: () = assert!(core::mem::size_of::<QosInfo>() == 16);
const _: () = assert!(core::mem::size_of::<CpuInfo>() == 64);
