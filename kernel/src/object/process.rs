//! Proces: adresni prostor, tabulka handlu a vlakna.
//!
//! Zivotni cyklus:
//! 1. `Process::new` - prazdny proces (bez vlaken, nespusteny),
//! 2. rodic namapuje pamet a spusti prvni vlakno (`start`),
//! 3. proces konci, kdyz skonci posledni vlakno; `terminate` (exit nebo
//!    kill) nastavi navratovy kod a pozada vsechna vlakna o ukonceni.
//! 4. Po skonceni: signal `TERMINATED`, tabulka handlu se vyprazdni (objekty
//!    se uvolni), adresni prostor zanikne s posledni referenci na proces.

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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
    /// Registrace a pakety ve vsech portech procesu (kvota
    /// `PROCESS_MAX_PORT_PENDING`, sdili ji porty vytvorene procesem).
    port_budget: Arc<AtomicUsize>,
}

impl Process {
    pub fn new(name: &str) -> Result<Arc<Self>, Error> {
        let space = AddressSpace::new().ok_or(Error::NoMemory)?;
        let p = Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: String::from(name),
            handles: SpinLock::new(HandleTable::new()),
            space,
            state: SpinLock::new(State {
                started: false,
                threads: Vec::new(),
                live: 0,
                exit_code: None,
                finished: false,
            }),
            port_budget: Arc::new(AtomicUsize::new(0)),
        });
        log::debug!("proces {} ({}) vytvoren", p.id, p.name);
        Ok(p)
    }

    pub fn handles(&self) -> SpinLockGuard<'_, HandleTable> {
        self.handles.lock()
    }

    /// Kvota portu procesu (predava se portum, ktere proces vytvori).
    pub fn port_budget(&self) -> Arc<AtomicUsize> {
        self.port_budget.clone()
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
        st.threads.retain(|w| w.strong_count() > 0);
        if st.threads.len() >= abi::limits::PROCESS_MAX_THREADS {
            return Err(Error::NoResources);
        }
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
