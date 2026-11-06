//! Objektovy model jadra.
//!
//! Vse, na co lze ukazat, je kernelovy objekt ([`KernelObject`]). Proces
//! k objektu pristupuje jen pres handle ve sve tabulce ([`HandleTable`]),
//! ktery nese prava ([`Rights`]). Kazdy objekt ma signaly ([`Signals`]),
//! na ktere lze cekat ([`wait`]).

pub mod event;
pub mod handle;
pub mod process;
pub mod wait;

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::ops::{BitAnd, BitOr, Not};

pub use handle::{Handle, HandleTable};

use crate::sched::{self, Thread};
use crate::sync::SpinLock;

/// Bitova maska stavu objektu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signals(pub u32);

impl Signals {
    pub const NONE: Self = Self(0);
    /// Event je signalizovany.
    pub const SIGNALED: Self = Self(1 << 0);
    /// Vlakno nebo proces skoncil.
    pub const TERMINATED: Self = Self(1 << 1);

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl BitOr for Signals {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitAnd for Signals {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

impl Not for Signals {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0)
    }
}

/// Prava handlu. Pri duplikaci lze prava jen ubirat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rights(u32);

#[allow(
    dead_code,
    reason = "cast prav pouziji az syscally a objekty pameti (milnik 4)"
)]
impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    /// Smi se vytvorit dalsi handle na stejny objekt.
    pub const DUPLICATE: Self = Self(1 << 4);
    /// Handle smi byt predan jinemu procesu.
    pub const TRANSFER: Self = Self(1 << 5);
    /// Smi se cekat na signaly objektu.
    pub const WAIT: Self = Self(1 << 6);
    /// Smi se menit uzivatelske signaly objektu.
    pub const SIGNAL: Self = Self(1 << 7);
    /// Smi se ridit objekt (napr. ukoncit proces).
    pub const MANAGE: Self = Self(1 << 8);

    pub const BASIC: Self = Self(Self::DUPLICATE.0 | Self::TRANSFER.0 | Self::WAIT.0);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Rights {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

#[allow(dead_code, reason = "API pro syscally (milnik 4)")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Event,
    Thread,
    Process,
}

/// Chyby operaci s objekty a handly. Pozdeji se z nich stanou navratove
/// kody syscallu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Handle neexistuje (nebo uz byl zavren).
    BadHandle,
    /// Handle nema potrebna prava.
    AccessDenied,
    /// Objekt je jineho typu, nez operace ocekava.
    WrongType,
    /// Vyprsel cas cekani.
    TimedOut,
    /// Nedostatek pameti.
    NoMemory,
    /// Vycerpany limit (napr. pocet handlu).
    NoResources,
}

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
