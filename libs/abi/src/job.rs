//! Joby: rozpocty prostredku pro skupiny procesu.
//!
//! Kazdy proces patri do jednoho jobu. Joby tvori strom (koren dostane
//! init od jadra). Vse, co proces stoji, se strhava z rozpoctu jeho jobu
//! a zaroven ze vsech predku - limit predka tak plati pro cely jeho podstrom.
//!
//! Tri oddelene rozpocty v bajtech ([`pool`]):
//! - **pamet jadra**: objekty jadra, sloty handlu, registrace portu,
//!   zpravy ve frontach kanalu, mapovani a tabulky stranek, zasobniky
//!   jadra vlaken,
//! - **uzivatelska pamet**: stranky objektu pameti (vcetne pameti pro DMA),
//! - **pripnuta pamet (DMA)**: stranky, ktere se nesmi presunout ani
//!   vystehovat (`VMO_PINNED`, `VMO_CREATE_CONTIGUOUS`); strhava se navic
//!   k uzivatelske pameti. Kvota je povinna: korenovy job ma pevny limit
//!   (polovina pameti pro programy), takze pripnuta pamet nikdy nezabere
//!   cely system.
//!
//! Prekroceni rozpoctu = `NO_RESOURCES`. Nedostatek fyzicke pameti systemu
//! je dal `NO_MEMORY` (rezerva jadra se pro programy nepouzije nikdy).

/// Rozpocty jobu (argument `pool` syscallu `JOB_SET_LIMIT`).
pub mod pool {
    /// Pamet jadra (objekty, handly, fronty, tabulky stranek).
    pub const KERNEL: u32 = 0;
    /// Uzivatelska pamet (stranky objektu pameti).
    pub const USER: u32 = 1;
    /// Pripnuta pamet pro DMA (navic k `USER`).
    pub const DMA: u32 = 2;
    /// Pocet rozpoctu.
    pub const COUNT: usize = 3;
}

/// Limit "bez vlastniho omezeni" (plati jen limity predku).
pub const UNLIMITED: u64 = u64::MAX;

/// Stav jednoho rozpoctu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PoolInfo {
    /// Strzeno z tohoto jobu (vcetne potomku).
    pub used: u64,
    /// Vlastni limit jobu ([`UNLIMITED`] = jen limity predku).
    pub limit: u64,
    /// Kolik jeste jde strhnout: nejmensi volne misto pres job a vsechny
    /// predky ([`UNLIMITED`], kdyz nikde neni limit).
    pub available: u64,
}

/// Vystup `JOB_INFO` (80 B, little endian: pro jadro a pak uzivatelskou
/// pamet vzdy `used`, `limit`, `available` jako u64, pak cislo jobu a nakonec
/// stejna trojice pro pripnutou pamet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JobInfo {
    pub kernel: PoolInfo,
    pub user: PoolInfo,
    /// Pripnuta pamet pro DMA.
    pub dma: PoolInfo,
    /// Cislo jobu (jedinecne do restartu; stejne jako v [`ProcEntry::job`]).
    pub id: u64,
}

impl JobInfo {
    pub const SIZE: usize = 80;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        let values = [
            self.kernel.used,
            self.kernel.limit,
            self.kernel.available,
            self.user.used,
            self.user.limit,
            self.user.available,
            self.id,
            self.dma.used,
            self.dma.limit,
            self.dma.available,
        ];
        for (i, v) in values.iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let v = |i: usize| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        Self {
            kernel: PoolInfo {
                used: v(0),
                limit: v(1),
                available: v(2),
            },
            user: PoolInfo {
                used: v(3),
                limit: v(4),
                available: v(5),
            },
            id: v(6),
            dma: PoolInfo {
                used: v(7),
                limit: v(8),
                available: v(9),
            },
        }
    }
}

/// Jeden proces ve vystupu `JOB_PROCESSES` (64 B, little endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcEntry {
    pub pid: u64,
    /// Cislo jobu procesu (muze to byt potomek dotazovaneho jobu).
    pub job: u64,
    /// Cas na CPU (ns, soucet vlaken).
    pub cpu_ns: u64,
    /// Zivych vlaken.
    pub threads: u32,
    /// [`proc_flags`].
    pub flags: u32,
    /// Jmeno procesu (UTF-8, doplneno nulami, oriznute na 32 B).
    pub name: [u8; 32],
}

/// Priznaky [`ProcEntry::flags`].
pub mod proc_flags {
    /// Proces uz skoncil (ceka jen na uklid handlu).
    pub const FINISHED: u32 = 1;
    /// Proces pouzil JIT: spustitelnou pamet bez pecete (audit).
    pub const JIT_USED: u32 = 2;
}

/// Politiky jobu nastavovane pres `JOB_SET_LIMIT(job, policy, hodnota)`
/// (cisla navazuji na `pool`).
pub mod policy {
    /// Opravneni JIT: hodnota = bity [`super::jit`].
    pub const JIT: u32 = 2;
}

/// Bity politiky JIT jobu. Plati jen, kdyz vsichni predci smi udelovat
/// (`GRANT`); nastavit je smi jen ten, jehoz rodicovsky job udelovat smi.
/// Novy job nema nic, korenovy jen `GRANT`.
pub mod jit {
    /// Procesy jobu smi mapovat spustitelnou pamet bez pecete (JIT).
    pub const RUN: u64 = 1;
    /// Job smi JIT udelit podrizenym jobum.
    pub const GRANT: u64 = 2;
}

impl ProcEntry {
    pub const SIZE: usize = 64;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..8].copy_from_slice(&self.pid.to_le_bytes());
        b[8..16].copy_from_slice(&self.job.to_le_bytes());
        b[16..24].copy_from_slice(&self.cpu_ns.to_le_bytes());
        b[24..28].copy_from_slice(&self.threads.to_le_bytes());
        b[28..32].copy_from_slice(&self.flags.to_le_bytes());
        b[32..64].copy_from_slice(&self.name);
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < Self::SIZE {
            return None;
        }
        let u = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let w = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let mut name = [0u8; 32];
        name.copy_from_slice(&b[32..64]);
        Some(Self { pid: u(0), job: u(8), cpu_ns: u(16), threads: w(24), flags: w(28), name })
    }

    /// Jmeno bez doplnujicich nul.
    pub fn name(&self) -> &str {
        let n = self.name.iter().position(|&c| c == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..n]).unwrap_or("?")
    }

    /// Jmeno pro zaznam (oriznute na 32 B na hranici znaku).
    pub fn name_bytes(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut end = s.len().min(32);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        out[..end].copy_from_slice(&s.as_bytes()[..end]);
        out
    }
}
