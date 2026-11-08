//! Planovani: trida QoS vlaken a procesu, strop jobu, udaje o CPU.
//!
//! Trida rika, co vlakno dela: `Interactive` (odezva na uzivatele),
//! `Normal`, `Background` (udrzba, davky) a `Realtime` (pevny rozpocet,
//! napr. zvuk). Vyssi tridu, nez dovoli strop jobu, si program nenastavi;
//! snizit si ji muze vzdy. Popis: `docs/cs/planovac.md`.

use abi::sched::{CpuInfo, QosInfo, class, op};

use crate::{Job, Process, Result, Thread, sys};

/// Trida QoS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qos {
    /// Pevny rozpocet `budget_us` v kazde `period_us` (0, 0 = 1 ms / 10 ms).
    Realtime { budget_us: u32, period_us: u32 },
    Interactive,
    Normal,
    Background,
}

impl Qos {
    fn args(self) -> (u64, u64, u64) {
        match self {
            Self::Realtime { budget_us, period_us } => (class::REALTIME as u64, budget_us as u64, period_us as u64),
            Self::Interactive => (class::INTERACTIVE as u64, 0, 0),
            Self::Normal => (class::NORMAL as u64, 0, 0),
            Self::Background => (class::BACKGROUND as u64, 0, 0),
        }
    }

    /// Cislo tridy z ABI.
    pub fn raw(self) -> u8 {
        self.args().0 as u8
    }
}

/// Vychozi trida systemovych sluzeb a ovladacu podle jmena programu
/// (pouziva init a spravce zarizeni; manifest ovladace ma prednost):
///
/// - `compd`, `inputd` a ovladace vstupu: Interactive (cesta vstup ->
///   snimek predbiha bezne programy),
/// - `audiod` a `hda`: Realtime 1 ms / 5 ms (zvuk bez prerusovani),
/// - ostatni: Normal (`None`).
pub fn service_default(name: &str) -> Option<Qos> {
    match name {
        "compd" | "inputd" | "ps2" | "virtio-input" | "usb-hid" => Some(Qos::Interactive),
        "audiod" | "hda" => Some(Qos::Realtime {
            budget_us: 1000,
            period_us: 5000,
        }),
        _ => None,
    }
}

/// Trida z manifestu ovladace (`driverkit`: trida, rozpocet, perioda).
pub fn from_raw(class: u8, budget_us: u32, period_us: u32) -> Option<Qos> {
    Some(match class {
        class::REALTIME => Qos::Realtime { budget_us, period_us },
        class::INTERACTIVE => Qos::Interactive,
        class::NORMAL => Qos::Normal,
        class::BACKGROUND => Qos::Background,
        _ => return None,
    })
}

/// Nastavi tridu volajiciho vlakna.
pub fn set_current(qos: Qos) -> Result<()> {
    let (c, b, p) = qos.args();
    sys::sched_control(op::THREAD_SET, 0, c, b, p).map(|_| ())
}

/// Nastavi tridu vlakna (handle s pravem `MANAGE`).
pub fn set_thread(thread: &Thread, qos: Qos) -> Result<()> {
    let (c, b, p) = qos.args();
    sys::sched_control(op::THREAD_SET, thread.handle().raw() as u64, c, b, p).map(|_| ())
}

/// Nastavi tridu vsech vlaken procesu i tech, ktera vytvori pozdeji
/// (handle s pravem `MANAGE`).
pub fn set_process(process: &Process, qos: Qos) -> Result<()> {
    let (c, b, p) = qos.args();
    sys::sched_control(op::PROCESS_SET, process.handle().raw() as u64, c, b, p).map(|_| ())
}

/// Strop tridy jobu (pravo `WRITE`; nejvys strop rodice).
pub fn set_job_max(job: &Job, qos: Qos) -> Result<()> {
    sys::sched_control(op::JOB_SET_MAX, job.handle().raw() as u64, qos.raw() as u64, 0, 0).map(|_| ())
}

/// Trida volajiciho vlakna (zakladni, ucinna, strop jobu).
pub fn current() -> Result<QosInfo> {
    let mut buf = [0u8; QosInfo::SIZE];
    sys::sched_control(op::THREAD_GET, 0, buf.as_mut_ptr() as u64, 0, 0)?;
    Ok(QosInfo::decode(&buf))
}

/// Udaje o CPU `index` (typ jadra, kapacita, cas behu po tridach) a pocet CPU.
pub fn cpu_info(index: usize) -> Result<(CpuInfo, usize)> {
    let mut buf = [0u8; CpuInfo::SIZE];
    let n = sys::sched_control(op::CPU_INFO, index as u64, buf.as_mut_ptr() as u64, 0, 0)?;
    Ok((CpuInfo::decode(&buf), n as usize))
}

/// Kapacita jadra z ACPI CPPC (sluzba ACPI; Resource s pravem `POWER`).
pub fn cpu_hint(resource: &crate::hw::Resource, index: usize, capacity: u32) -> Result<()> {
    sys::sched_control(op::CPU_HINT, resource.handle().raw() as u64, index as u64, capacity as u64, 0).map(|_| ())
}
