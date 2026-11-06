//! Objektovy model jadra.
//!
//! Vse, na co lze ukazat, je kernelovy objekt ([`KernelObject`]). Proces
//! k objektu pristupuje jen pres handle ve sve tabulce ([`HandleTable`]),
//! ktery nese prava ([`Rights`]). Kazdy objekt ma signaly ([`Signals`]),
//! na ktere lze cekat ([`wait`]).

pub mod channel;
pub mod event;
pub mod handle;
pub mod interrupt;
pub mod process;
pub mod resource;
pub mod vmo;
pub mod wait;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

pub use handle::{Handle, HandleTable};

pub use crate::error::Error;
pub use abi::{ObjectType, Rights, Signals};

use crate::sched::{self, Thread};
use crate::sync::SpinLock;

pub trait KernelObject: Any + Send + Sync {
    fn core(&self) -> &ObjectCore;
    #[allow(dead_code, reason = "API pro syscally (milnik 4)")]
    fn object_type(&self) -> ObjectType;
    /// Prava, ktera dostane prvni handle na novy objekt.
    fn default_rights(&self) -> Rights;
}

/// Vlakno cekajici na signaly objektu.
struct Observer {
    thread: Arc<Thread>,
    seq: u64,
    mask: Signals,
}

/// Spolecna cast vsech objektu: signaly a seznam cekajicich.
pub struct ObjectCore {
    inner: SpinLock<CoreInner>,
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
        }
    }

    pub fn signals(&self) -> Signals {
        self.inner.lock().signals
    }

    /// Zmeni signaly a probudi vlakna, na jejichz masku nova hodnota sedi.
    pub fn update(&self, clear: Signals, set: Signals) {
        let mut inner = self.inner.lock();
        let signals = (inner.signals & !clear) | set;
        inner.signals = signals;
        inner.observers.retain(|o| {
            if o.mask.intersects(signals) {
                sched::wake(&o.thread, o.seq);
                false
            } else {
                true
            }
        });
    }

    /// Pokud uz signaly odpovidaji masce, vrati je. Jinak zaregistruje
    /// vlakno jako cekajici (atomicky vuci `update`, takze probuzeni neutece).
    fn check_or_observe(&self, thread: &Arc<Thread>, seq: u64, mask: Signals) -> Option<Signals> {
        let mut inner = self.inner.lock();
        if inner.signals.intersects(mask) {
            return Some(inner.signals);
        }
        inner.observers.push(Observer {
            thread: thread.clone(),
            seq,
            mask,
        });
        None
    }

    fn remove_observer(&self, thread: &Thread, seq: u64) {
        self.inner
            .lock()
            .observers
            .retain(|o| !(core::ptr::eq(&*o.thread, thread) && o.seq == seq));
    }
}
