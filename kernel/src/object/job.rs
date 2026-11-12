//! Job: rozpocet prostredku pro skupinu procesu.
//!
//! Joby tvori strom. Kazdy proces patri do jednoho jobu a vse, co stoji,
//! se strhava ([`Charge`]) z jeho jobu a zaroven ze vsech predku: limit
//! jobu plati pro cely jeho podstrom a potomek ho nemuze obejit ani tim,
//! ze si zvedne vlastni limit (predek ho porad omezuje).
//!
//! Dva rozpocty v bajtech (`abi::job::pool`): pamet jadra a uzivatelska
//! pamet. Strzeni je bez zamku (atomicke citace) a vraci se automaticky,
//! kdyz zanikne vec, ktera ho drzi ([`Charge`] v `Drop`).
//!
//! Soubeh: dve soucasna strzeni tesne pod limitem mohou obe selhat (citac
//! chvili ukazuje obe). Opacne to nejde - limit se nikdy neprekroci.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use schedcore::Class;

use abi::job::{JobInfo, PoolInfo, UNLIMITED, pool};

use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};

/// Rozpocet jobu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pool {
    Kernel = pool::KERNEL as isize,
    User = pool::USER as isize,
}

impl Pool {
    pub fn from_raw(v: u64) -> Option<Self> {
        match v {
            v if v == pool::KERNEL as u64 => Some(Self::Kernel),
            v if v == pool::USER as u64 => Some(Self::User),
            _ => None,
        }
    }
}

/// Odhad pameti jadra pro objekt typu `T` za `Arc` (data + citace,
/// zaokrouhleno na tridu heapu - mocnina dvou).
pub const fn object_cost<T>() -> u64 {
    (core::mem::size_of::<T>() + 16).next_power_of_two() as u64
}

/// Cislovani jobu (od 1).
static NEXT_JOB_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

pub struct Job {
    core: ObjectCore,
    /// Cislo jobu (vystup `JOB_INFO`, `JOB_PROCESSES`).
    pub id: u64,
    parent: Option<Arc<Job>>,
    limits: [AtomicU64; pool::COUNT],
    used: [AtomicU64; pool::COUNT],
    /// Vlastni strop tridy QoS (`Class as u8`); plati i strop predku.
    qos_max: AtomicU8,
    /// Job ukonceny `JOB_KILL`: neprijme novy proces ani podjob.
    killed: AtomicBool,
    /// Strzeni za tento objekt (z rozpoctu jobu, ktery ho vytvoril).
    _charge: Charge,
}

impl Job {
    /// Korenovy job (dostane init). Bez limitu: pamet systemu chrani
    /// rezerva jadra, rozdeleni mezi sluzby urci init.
    pub fn new_root() -> Arc<Self> {
        Self::make(None, Charge::none())
    }

    /// Podrizeny job bez vlastniho limitu (plati limity predku). Objekt
    /// se strhne z rozpoctu `parent`.
    pub fn new_child(parent: &Arc<Job>) -> Result<Arc<Self>, Error> {
        if parent.is_killed() {
            return Err(Error::BadState);
        }
        let charge = parent.charge(Pool::Kernel, object_cost::<Job>())?;
        let child = Self::make(Some(parent.clone()), charge);
        // Soubeh s `JOB_KILL` predka: podjob vznikly behem nej neprezije
        // (kontrola po vytvoreni; zabity predek je videt pres retez).
        if child.is_killed() {
            return Err(Error::BadState);
        }
        Ok(child)
    }

    /// Je job (nebo nektery predek) ukonceny `JOB_KILL`?
    pub fn is_killed(&self) -> bool {
        let mut node = Some(self);
        while let Some(j) = node {
            if j.killed.load(Ordering::SeqCst) {
                return true;
            }
            node = j.parent.as_deref();
        }
        false
    }

    /// Oznaci job jako ukonceny (dalsi procesy a podjoby neprijme).
    /// Procesy ukonci volajici (`JOB_KILL` projde registr procesu).
    pub fn mark_killed(&self) {
        self.killed.store(true, Ordering::SeqCst);
    }

    fn make(parent: Option<Arc<Job>>, charge: Charge) -> Arc<Self> {
        Arc::new(Self {
            id: NEXT_JOB_ID.fetch_add(1, Ordering::Relaxed),
            core: ObjectCore::new(Signals::NONE),
            parent,
            limits: [AtomicU64::new(UNLIMITED), AtomicU64::new(UNLIMITED)],
            used: [AtomicU64::new(0), AtomicU64::new(0)],
            qos_max: AtomicU8::new(Class::Realtime as u8),
            killed: AtomicBool::new(false),
            _charge: charge,
        })
    }

    /// Nastavi vlastni limit. Limit pod aktualni spotrebou je povoleny:
    /// nic se neodebira, jen dalsi strzeni selzou.
    pub fn set_limit(&self, pool: Pool, bytes: u64) {
        self.limits[pool as usize].store(bytes, Ordering::Release);
    }

    /// Strhne `bytes` z rozpoctu `pool` tohoto jobu a vsech predku.
    pub fn charge(self: &Arc<Self>, pool: Pool, bytes: u64) -> Result<Charge, Error> {
        self.try_charge(pool, bytes)?;
        Ok(Charge {
            job: Some(self.clone()),
            pool,
            bytes,
        })
    }

    /// Strhne bez drzitele (vraci se pres [`Job::uncharge`]).
    pub fn try_charge(&self, pool: Pool, bytes: u64) -> Result<(), Error> {
        if bytes == 0 {
            return Ok(());
        }
        let p = pool as usize;
        let mut node = Some(self);
        while let Some(j) = node {
            let prev = j.used[p].fetch_add(bytes, Ordering::AcqRel);
            if prev.saturating_add(bytes) > j.limits[p].load(Ordering::Acquire) {
                j.used[p].fetch_sub(bytes, Ordering::AcqRel);
                // Vratit, co uz se strhlo u potomku `j` na ceste.
                let mut back = Some(self);
                while let Some(b) = back {
                    if core::ptr::eq(b, j) {
                        break;
                    }
                    b.used[p].fetch_sub(bytes, Ordering::AcqRel);
                    back = b.parent.as_deref();
                }
                return Err(Error::NoResources);
            }
            node = j.parent.as_deref();
        }
        Ok(())
    }

    /// Vrati strzeni z [`Job::try_charge`].
    pub fn uncharge(&self, pool: Pool, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let p = pool as usize;
        let mut node = Some(self);
        while let Some(j) = node {
            j.used[p].fetch_sub(bytes, Ordering::AcqRel);
            node = j.parent.as_deref();
        }
    }

    fn pool_info(&self, pool: Pool) -> PoolInfo {
        let p = pool as usize;
        let mut available = UNLIMITED;
        let mut node = Some(self);
        while let Some(j) = node {
            let limit = j.limits[p].load(Ordering::Acquire);
            if limit != UNLIMITED {
                available = available.min(limit.saturating_sub(j.used[p].load(Ordering::Acquire)));
            }
            node = j.parent.as_deref();
        }
        PoolInfo {
            used: self.used[p].load(Ordering::Acquire),
            limit: self.limits[p].load(Ordering::Acquire),
            available,
        }
    }

    pub fn info(&self) -> JobInfo {
        JobInfo {
            kernel: self.pool_info(Pool::Kernel),
            user: self.pool_info(Pool::User),
            id: self.id,
        }
    }

    /// Ucinny strop tridy QoS: nejnizsi z vlastniho a vsech predku.
    pub fn qos_cap(&self) -> Class {
        let mut cap = Class::Realtime;
        let mut node = Some(self);
        while let Some(j) = node {
            if let Some(c) = Class::from_raw(j.qos_max.load(Ordering::Acquire)) {
                cap = cap.min(c);
            }
            node = j.parent.as_deref();
        }
        cap
    }

    /// Nastavi strop tridy. Nad strop rodice (vcetne jeho predku) nejde:
    /// kdo ma handle na podrizeny job, si tak nezvedne vic, nez smi rodic.
    pub fn set_qos_max(&self, class: Class) -> Result<(), Error> {
        let parent_cap = self.parent.as_deref().map_or(Class::Realtime, Job::qos_cap);
        if class.above(parent_cap) {
            return Err(Error::AccessDenied);
        }
        self.qos_max.store(class as u8, Ordering::Release);
        Ok(())
    }

    /// Je job `self` roven `ancestor`, nebo jeho potomek?
    pub fn within(&self, ancestor: &Job) -> bool {
        let mut node = Some(self);
        while let Some(j) = node {
            if core::ptr::eq(j, ancestor) {
                return true;
            }
            node = j.parent.as_deref();
        }
        false
    }
}

impl KernelObject for Job {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Job
    }

    /// `MANAGE` = spoustet procesy a zakladat podrizene joby, `WRITE` =
    /// menit limity, `READ` = stav rozpoctu.
    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MANAGE
    }
}

/// Strzeni z rozpoctu jobu; vrati se pri zaniku.
pub struct Charge {
    job: Option<Arc<Job>>,
    pool: Pool,
    bytes: u64,
}

impl Charge {
    /// Nic nestrzeno (objekty jadra mimo procesy, testy).
    pub const fn none() -> Self {
        Self {
            job: None,
            pool: Pool::Kernel,
            bytes: 0,
        }
    }

    /// Strhne dalsich `bytes` ze stejneho rozpoctu.
    pub fn grow(&mut self, bytes: u64) -> Result<(), Error> {
        if let Some(j) = &self.job {
            j.try_charge(self.pool, bytes)?;
        }
        self.bytes += bytes;
        Ok(())
    }

    /// Vrati cast strzeni.
    pub fn shrink(&mut self, bytes: u64) {
        let b = bytes.min(self.bytes);
        if let Some(j) = &self.job {
            j.uncharge(self.pool, b);
        }
        self.bytes -= b;
    }
}

impl Default for Charge {
    fn default() -> Self {
        Self::none()
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        if let Some(j) = &self.job {
            j.uncharge(self.pool, self.bytes);
        }
    }
}
