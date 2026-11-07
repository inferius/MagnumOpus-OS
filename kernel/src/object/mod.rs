//! Objektovy model jadra.
//!
//! Vse, na co lze ukazat, je kernelovy objekt ([`KernelObject`]). Proces
//! k objektu pristupuje jen pres handle ve sve tabulce ([`HandleTable`]),
//! ktery nese prava ([`Rights`]). Kazdy objekt ma signaly ([`Signals`]),
//! na ktere lze cekat ([`wait`]) - primo vlaknem, nebo pres port
//! ([`port`], jednorazova registrace s paketem). Pamet objektu vytvorenych
//! na pozadani procesu se strhava z rozpoctu jeho jobu ([`job`]).

pub mod channel;
pub mod event;
pub mod handle;
pub mod interrupt;
pub mod job;
pub mod port;
pub mod process;
pub mod resource;
pub mod vmo;
pub mod wait;

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::any::Any;

pub use handle::{Handle, HandleTable};

pub use crate::error::Error;
pub use abi::{ObjectType, Rights, Signals};

use crate::sched::{self, Thread};
use crate::sync::SpinLock;
use job::{Charge, Job, Pool, object_cost};

pub trait KernelObject: Any + Send + Sync {
    fn core(&self) -> &ObjectCore;
    fn object_type(&self) -> ObjectType;
    /// Prava, ktera dostane prvni handle na novy objekt.
    fn default_rights(&self) -> Rights;
}

/// Kdo ceka na signaly objektu.
enum Waiter {
    /// Vlakno v `wait_many` (probudi se).
    Thread { thread: Arc<Thread>, seq: u64 },
    /// Jednorazova registrace portu (dostane paket s klicem).
    Port { port: Weak<port::Port>, key: u64 },
}

struct Observer {
    waiter: Waiter,
    mask: Signals,
}

impl Drop for Observer {
    /// Nedorucena registrace portu (objekt zanikl, port ji uz nedostane)
    /// vrati portu rezervovane misto.
    fn drop(&mut self) {
        if let Waiter::Port { port, .. } = &self.waiter
            && let Some(p) = port.upgrade()
        {
            p.unreserve();
        }
    }
}

/// Spolecna cast vsech objektu: signaly, seznam cekajicich a strzeni
/// z rozpoctu jobu (vrati se se zanikem objektu).
pub struct ObjectCore {
    inner: SpinLock<CoreInner>,
    charge: SpinLock<Charge>,
}

/// Strhne pamet objektu (odhad [`object_cost`]) z rozpoctu jadra jobu
/// `job`; pri prekroceni rozpoctu objekt zanikne a vrati `NoResources`.
pub fn charged<T: KernelObject>(obj: Arc<T>, job: &Arc<Job>) -> Result<Arc<T>, Error> {
    let c = job.charge(Pool::Kernel, object_cost::<T>())?;
    obj.core().set_charge(c);
    Ok(obj)
}

struct CoreInner {
    signals: Signals,
    observers: Vec<Observer>,
}

impl ObjectCore {
    pub const fn new(initial: Signals) -> Self {
        Self {
            inner: SpinLock::new(CoreInner {
                signals: initial,
                observers: Vec::new(),
            }),
            charge: SpinLock::new(Charge::none()),
        }
    }

    /// Prida k objektu strzeni (vrati se se zanikem objektu).
    pub fn set_charge(&self, c: Charge) {
        *self.charge.lock() = c;
    }

    pub fn signals(&self) -> Signals {
        self.inner.lock().signals
    }

    /// Zmeni signaly, probudi vlakna a doruci pakety registracim portu,
    /// na jejichz masku nova hodnota sedi.
    pub fn update(&self, clear: Signals, set: Signals) {
        let mut inner = self.inner.lock();
        let signals = (inner.signals & !clear) | set;
        inner.signals = signals;
        inner.observers.retain_mut(|o| {
            if !o.mask.intersects(signals) {
                // Registrace zavreneho portu uz nikdo nevybere.
                return !matches!(&o.waiter, Waiter::Port { port, .. } if port.strong_count() == 0);
            }
            match &mut o.waiter {
                Waiter::Thread { thread, seq } => sched::wake(thread, *seq),
                Waiter::Port { port, key } => {
                    // Doruceno: rezervace se spotrebuje (Drop ji uz nevraci).
                    let port = core::mem::take(port);
                    if let Some(p) = port.upgrade() {
                        p.deliver(*key, signals);
                    }
                }
            }
            false
        });
    }

    /// Jednorazova registrace portu: az signaly protnou `mask`, port dostane
    /// paket s klicem `key` (hned, pokud uz protinaji). Misto v portu se
    /// rezervuje predem.
    pub fn observe_async(&self, port: &Arc<port::Port>, key: u64, mask: Signals) -> Result<(), Error> {
        port.reserve()?;
        let mut inner = self.inner.lock();
        if inner.signals.intersects(mask) {
            port.deliver(key, inner.signals);
            return Ok(());
        }
        if inner.observers.try_reserve(1).is_err() {
            drop(inner);
            port.unreserve();
            return Err(Error::NoMemory);
        }
        inner.observers.push(Observer {
            waiter: Waiter::Port {
                port: Arc::downgrade(port),
                key,
            },
            mask,
        });
        Ok(())
    }

    /// Pokud uz signaly odpovidaji masce, vrati je. Jinak zaregistruje
    /// vlakno jako cekajici (atomicky vuci `update`, takze probuzeni neutece).
    fn check_or_observe(&self, thread: &Arc<Thread>, seq: u64, mask: Signals) -> Option<Signals> {
        let mut inner = self.inner.lock();
        if inner.signals.intersects(mask) {
            return Some(inner.signals);
        }
        inner.observers.push(Observer {
            waiter: Waiter::Thread {
                thread: thread.clone(),
                seq,
            },
            mask,
        });
        None
    }

    fn remove_observer(&self, thread: &Thread, seq: u64) {
        self.inner.lock().observers.retain(|o| match &o.waiter {
            Waiter::Thread { thread: t, seq: s } => !(core::ptr::eq(&**t, thread) && *s == seq),
            Waiter::Port { .. } => true,
        });
    }
}
