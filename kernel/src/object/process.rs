//! Proces: adresni prostor, tabulka handlu a vlakna.
//!
//! Zivotni cyklus:
//! 1. `Process::new` - prazdny proces (bez vlaken, nespusteny),
//! 2. rodic namapuje pamet a spusti prvni vlakno (`start`),
//! 3. proces konci, kdyz skonci posledni vlakno; `terminate` (exit nebo
//!    kill) nastavi navratovy kod a pozada vsechna vlakna o ukonceni.
//! 4. Po skonceni: signal `TERMINATED`, tabulka handlu se vyprazdni (objekty
//!    se uvolni), adresni prostor zanikne s posledni referenci na proces.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::job::{Job, Pool, object_cost};
use super::{Error, HandleTable, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::log;
use crate::mm::AddressSpace;
use crate::sched::Thread;
use crate::sync::{SpinLock, SpinLockGuard};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct State {
    started: bool,
    /// Vlakna procesu (slabe odkazy, vlakno drzi silny odkaz na proces).
    threads: Vec<Weak<Thread>>,
    /// Pocet vlaken, ktera bezi (spustena a neskoncila).
    live: usize,
    /// Navratovy kod, jakmile proces konci nebo skoncil.
    exit_code: Option<i64>,
    finished: bool,
}

pub struct Process {
    core: ObjectCore,
    pub id: u64,
    pub name: String,
    handles: SpinLock<HandleTable>,
    space: AddressSpace,
    state: SpinLock<State>,
    /// Job, do ktereho proces patri: z jeho rozpoctu se strhava vse, co
    /// proces stoji (objekty, handly, mapovani, vlakna, pamet).
    job: Arc<Job>,
    /// Cas na CPU vsech vlaken (ns; pricita scheduler pri prepnuti).
    pub cpu_ns: AtomicU64,
    /// Trida QoS novych vlaken a Realtime parametry (ns) z `PROCESS_SET`.
    qos: SpinLock<(schedcore::Class, Option<(u64, u64)>)>,
    /// Fronty futexu (`FUTEX`): virtualni adresa ->
    /// cekajici vlakna (vlakno, poradove cislo cekani) v poradi prichodu.
    /// Polozek je nejvys tolik, kolik ma proces vlaken.
    pub futex: SpinLock<BTreeMap<u64, VecDeque<(Arc<Thread>, u64)>>>,
}

/// Vsechny procesy (slabe odkazy) pro `JOB_PROCESSES`. Mrtve se uklidi pri
/// rustu pole (amortizovane O(1)).
static REGISTRY: SpinLock<Vec<Weak<Process>>> = SpinLock::new(Vec::new());

/// Zivi procesy (pro vycet jobu).
pub fn live_processes() -> Vec<Arc<Process>> {
    REGISTRY.lock().iter().filter_map(|w| w.upgrade()).collect()
}

/// Pamet jadra za proces mimo jeho objekty: struktura a korenova tabulka
/// stranek.
const PROCESS_COST: u64 = object_cost::<Process>() + crate::arch::paging::PAGE_SIZE;

impl Process {
    /// Novy proces v jobu `job` (strhne se z jeho rozpoctu jadra).
    pub fn new(name: &str, job: Arc<Job>) -> Result<Arc<Self>, Error> {
        let charge = job.charge(Pool::Kernel, PROCESS_COST)?;
        let space = AddressSpace::new(Some(job.clone())).ok_or(Error::NoMemory)?;
        let table = HandleTable::new(job.charge(Pool::Kernel, 0)?);
        let p = Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: String::from(name),
            handles: SpinLock::new(table),
            space,
            state: SpinLock::new(State {
                started: false,
                threads: Vec::new(),
                live: 0,
                exit_code: None,
                finished: false,
            }),
            job,
            cpu_ns: AtomicU64::new(0),
            qos: SpinLock::new((schedcore::Class::Normal, None)),
            futex: SpinLock::new(BTreeMap::new()),
        });
        p.core.set_charge(charge);
        {
            let mut r = REGISTRY.lock();
            if r.len() == r.capacity() {
                r.retain(|w| w.strong_count() > 0);
            }
            r.push(Arc::downgrade(&p));
        }
        // Soubeh s `JOB_KILL`: nejdriv registrace, pak kontrola. Bud
        // `JOB_KILL` proces v registru uvidi a ukonci, nebo tady uvidime
        // jeho priznak (oboji SeqCst) - proces neproklouzne.
        if p.job.is_killed() {
            p.terminate(abi::process::EXIT_KILLED);
            return Err(Error::BadState);
        }
        log::debug!("proces {} ({}) vytvoren", p.id, p.name);
        Ok(p)
    }

    /// Trida QoS pro nova vlakna procesu (a jejich Realtime parametry).
    pub fn default_qos(&self) -> (schedcore::Class, Option<(u64, u64)>) {
        *self.qos.lock()
    }

    pub fn set_default_qos(&self, class: schedcore::Class, rt: Option<(u64, u64)>) {
        *self.qos.lock() = (class, rt);
    }

    /// Zive vlakna procesu.
    pub fn threads(&self) -> Vec<Arc<Thread>> {
        self.state.lock().threads.iter().filter_map(Weak::upgrade).collect()
    }

    pub fn handles(&self) -> SpinLockGuard<'_, HandleTable> {
        self.handles.lock()
    }

    /// Zivych vlaken a zda proces skoncil (vycet `JOB_PROCESSES`).
    pub fn thread_stats(&self) -> (u32, bool) {
        let st = self.state.lock();
        (st.live as u32, st.finished || st.exit_code.is_some())
    }

    /// Job procesu (rozpocet pro vse, co proces vytvori).
    pub fn job(&self) -> &Arc<Job> {
        &self.job
    }

    pub fn address_space(&self) -> &AddressSpace {
        &self.space
    }

    /// Zaregistruje nove (jeste nespustene) vlakno procesu.
    pub fn add_thread(&self, t: &Arc<Thread>) -> Result<(), Error> {
        let mut st = self.state.lock();
        if st.exit_code.is_some() {
            return Err(Error::BadState);
        }
        // Mrtve odkazy uklidit jen pred rustem pole (amortizovane O(1)).
        if st.threads.len() == st.threads.capacity() {
            st.threads.retain(|w| w.strong_count() > 0);
        }
        // Pocet vlaken omezuje rozpocet jobu (zasobnik jadra kazdeho vlakna).
        st.threads.try_reserve(1).map_err(|_| Error::NoMemory)?;
        st.threads.push(Arc::downgrade(t));
        Ok(())
    }

    /// Zaznamena spusteni vlakna. `first` = prvni vlakno procesu
    /// (PROCESS_START), jinak proces uz musi bezet (THREAD_START).
    pub fn thread_starting(&self, first: bool) -> Result<(), Error> {
        let mut st = self.state.lock();
        if st.exit_code.is_some() || st.started != !first {
            return Err(Error::BadState);
        }
        st.started = true;
        st.live += 1;
        Ok(())
    }

    /// Vlakno procesu skoncilo. Posledni vlakno proces dokonci.
    pub fn thread_exited(&self) {
        let finish = {
            let mut st = self.state.lock();
            st.live -= 1;
            if st.live == 0 && !st.finished {
                st.finished = true;
                st.exit_code.get_or_insert(abi::process::EXIT_NO_THREADS);
                true
            } else {
                false
            }
        };
        if finish {
            self.finish();
        }
    }

    /// Ukonci proces s kodem `code` (prvni volani vyhrava). Vsechna vlakna
    /// dostanou pozadavek na ukonceni; skonci pri navratu do userspace nebo
    /// z cekani v jadre.
    pub fn terminate(&self, code: i64) {
        let (threads, finish_now) = {
            let mut st = self.state.lock();
            if st.exit_code.is_some() {
                return;
            }
            st.exit_code = Some(code);
            let threads: Vec<Arc<Thread>> = st.threads.iter().filter_map(Weak::upgrade).collect();
            // Proces bez bezicich vlaken (napr. nespusteny) konci hned.
            let finish_now = st.live == 0 && !st.finished;
            if finish_now {
                st.finished = true;
            }
            (threads, finish_now)
        };
        log::debug!(
            "proces {} ({}) konci s kodem {code}: {}",
            self.id,
            self.name,
            abi::process::describe_exit(code)
        );
        for t in &threads {
            crate::sched::request_kill(t);
        }
        if finish_now {
            self.finish();
        }
    }

    fn finish(&self) {
        let code = self.exit_code().unwrap_or(0);
        log::info!(
            "proces {} ({}) skoncil, kod {code} ({})",
            self.id,
            self.name,
            abi::process::describe_exit(code)
        );
        // Uvolnit vsechny objekty, na ktere proces drzel handly. Tabulku
        // vymenime za prazdnou a zrusime ji az mimo zamek (Drop objektu
        // muze budit dalsi vlakna).
        let table = core::mem::take(&mut *self.handles.lock());
        drop(table);
        if crate::console::process_finished(self.id) {
            log::info!("framebuffer: proces {} skoncil, vracen jadru", self.id);
        }
        self.core.update(Signals::NONE, Signals::TERMINATED);
    }

    pub fn exit_code(&self) -> Option<i64> {
        let st = self.state.lock();
        if st.finished { st.exit_code } else { None }
    }

    pub fn is_started(&self) -> bool {
        self.state.lock().started
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        log::debug!("proces {} ({}) uvolnen", self.id, self.name);
    }
}

impl KernelObject for Process {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Process
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::MANAGE | Rights::WRITE | Rights::READ
    }
}
