//! Joby: rozpocty prostredku pro skupiny procesu.
//!
//! Proces bezi v jobu, ktery dostal od rodice ([`current`]); `spawn`
//! spousti potomky do stejneho jobu, `spawn::spawn_in` do jineho.
//! Rodic potomkovi omezi pamet tak, ze mu zalozi podrizeny job
//! ([`Job::create`]), nastavi limity ([`Job::set_limit`]) a spusti ho v nem.
//! Potomek dostane job bez prava `WRITE`, takze si limity nezmeni.
//!
//! ```ignore
//! let job = libos::job::Job::create(&libos::job::current()?)?;
//! job.set_limit(pool::USER, 64 << 20)?; // 64 MiB uzivatelske pameti
//! job.set_limit(pool::KERNEL, 4 << 20)?; // 4 MiB pameti jadra
//! let (proc, ch) = libos::spawn::spawn_in(&job, "sluzba", image, handles)?;
//! ```

use core::sync::atomic::{AtomicU32, Ordering};

use abi::job::JobInfo;
pub use abi::job::{UNLIMITED, pool};
use abi::{Error, Result, Rights};

use crate::handle::object_type;
use crate::{Handle, sys};

#[derive(Debug)]
pub struct Job(Handle);
object_type!(Job);

/// Prava jobu predavana potomkovi: vse krome `WRITE` (limity).
pub const CHILD_RIGHTS: Rights = Rights(
    Rights::BASIC.0 | Rights::READ.0 | Rights::MANAGE.0,
);

impl Job {
    /// Podrizeny job bez vlastniho limitu (plati limity `parent`).
    pub fn create(parent: &Job) -> Result<Self> {
        sys::job_create(parent.0.raw()).map(|h| Self(Handle::from_raw(h)))
    }

    /// Nastavi limit rozpoctu `pool` ([`pool::KERNEL`], [`pool::USER`])
    /// v bajtech; [`UNLIMITED`] = bez vlastniho limitu.
    pub fn set_limit(&self, pool: u32, bytes: u64) -> Result<()> {
        sys::job_set_limit(self.0.raw(), pool, bytes)
    }

    /// Spotreba a limity.
    pub fn info(&self) -> Result<JobInfo> {
        sys::job_info(self.0.raw())
    }

    /// Procesy jobu a jeho potomku (jmeno, cislo, job, cas CPU, vlakna).
    pub fn processes(&self) -> Result<alloc::vec::Vec<abi::job::ProcEntry>> {
        let mut cap = 64usize;
        loop {
            let mut buf = alloc::vec![0u8; cap * abi::job::ProcEntry::SIZE];
            let n = sys::job_processes(self.0.raw(), &mut buf)?;
            if n <= cap {
                return Ok(buf
                    .chunks(abi::job::ProcEntry::SIZE)
                    .take(n)
                    .filter_map(abi::job::ProcEntry::decode)
                    .collect());
            }
            cap = n.min(4096);
            if n > 4096 {
                // Vic nez limit syscallu: prvnich 4096.
                let mut buf = alloc::vec![0u8; 4096 * abi::job::ProcEntry::SIZE];
                sys::job_processes(self.0.raw(), &mut buf)?;
                return Ok(buf.chunks(abi::job::ProcEntry::SIZE).filter_map(abi::job::ProcEntry::decode).collect());
            }
        }
    }

    /// Novy handle na stejny job; `None` = stejna prava.
    pub fn duplicate(&self, rights: Option<Rights>) -> Result<Self> {
        self.0.duplicate(rights).map(Self)
    }
}

/// Job vlastniho procesu (syrovy handle drzi libos, nikdy se nezavre).
static CURRENT: AtomicU32 = AtomicU32::new(0);

pub(crate) fn set_current(raw: u32) {
    CURRENT.store(raw, Ordering::Relaxed);
}

/// Syrovy handle vlastniho jobu (0 = proces job nedostal).
pub(crate) fn current_raw() -> u32 {
    CURRENT.load(Ordering::Relaxed)
}

/// Novy handle na job, ve kterem proces bezi. `NOT_FOUND`, kdyz ho rodic
/// nepredal.
pub fn current() -> Result<Job> {
    match current_raw() {
        0 => Err(Error::NotFound),
        raw => sys::handle_duplicate(raw, None).map(|h| Job(Handle::from_raw(h))),
    }
}

/// Stav rozpoctu vlastniho jobu.
pub fn current_info() -> Result<JobInfo> {
    match current_raw() {
        0 => Err(Error::NotFound),
        raw => sys::job_info(raw),
    }
}
