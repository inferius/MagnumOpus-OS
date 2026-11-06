//! Vlakno jadra.

use alloc::boxed::Box;
use alloc::string::String;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use crate::arch::context::Context;
use crate::mm::KernelStack;
use crate::object::{KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;

/// Trida kvality sluzby: rika, co vlakno dela, ne jak se ma planovat.
/// Mezi tridami plati prisna priorita (nizsi cislo vyhrava), uvnitr tridy
/// round-robin s casovym kvantem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[allow(dead_code, reason = "tridy pouziji sluzby a aplikace (milnik 4+)")]
#[repr(u8)]
pub enum Qos {
    Realtime = 0,
    Interactive = 1,
    Normal = 2,
    Background = 3,
    /// Jen idle vlakno CPU; nikdy neni ve fronte.
    Idle = 4,
}

pub const QOS_CLASSES: usize = 4;

/// Stavy vlakna.
pub(super) mod state {
    /// Bezi na nekterem CPU.
    pub const RUNNING: u8 = 0;
    /// Ve fronte, ceka na CPU.
    pub const RUNNABLE: u8 = 1;
    /// Chysta se usnout (zaregistrovane u objektu), jeste bezi.
    pub const BLOCKING: u8 = 2;
    /// Spi, dokud ho nekdo neprobudi.
    pub const BLOCKED: u8 = 3;
    /// Skoncilo.
    pub const DEAD: u8 = 4;
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Thread {
    core: ObjectCore,
    #[allow(dead_code, reason = "identifikace vlakna pro syscally a ladeni")]
    pub id: u64,
    pub name: String,
    pub qos: Qos,
    pub(super) state: AtomicU8,
    /// True, dokud nejake CPU nedokoncilo prepnuti pryc z tohoto vlakna
    /// (kontext jeste neni ulozeny, nesmi ho spustit jine CPU).
    pub(super) on_cpu: AtomicBool,
    /// Poradove cislo aktualniho cekani; probuzeni se starym cislem se ignoruje.
    wait_seq: AtomicU64,
    pub(super) last_cpu: AtomicUsize,
    pub(super) context: UnsafeCell<Context>,
    _stack: Option<KernelStack>,
    pub(super) entry: SpinLock<Option<Box<dyn FnOnce() + Send>>>,
}

// SAFETY: `context` cte a zapisuje jen planovac na CPU, ktere vlakno prave
// prepina (se zakazanymi prerusenimi); `on_cpu` zarucuje, ze to v jednu
// chvili dela jen jedno CPU. Ostatni pole jsou atomicka nebo pod zamkem.
unsafe impl Sync for Thread {}
// SAFETY: viz vyse.
unsafe impl Send for Thread {}

impl Thread {
    pub(super) fn new(
        name: &str,
        qos: Qos,
        stack: Option<KernelStack>,
        context: Context,
        entry: Option<Box<dyn FnOnce() + Send>>,
        cpu: usize,
    ) -> Self {
        Self {
            core: ObjectCore::new(Signals::NONE),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: String::from(name),
            qos,
            state: AtomicU8::new(state::RUNNABLE),
            on_cpu: AtomicBool::new(false),
            wait_seq: AtomicU64::new(0),
            last_cpu: AtomicUsize::new(cpu),
            context: UnsafeCell::new(context),
            _stack: stack,
            entry: SpinLock::new(entry),
        }
    }

    pub(super) fn state(&self) -> u8 {
        self.state.load(Ordering::Acquire)
    }

    pub(super) fn set_state(&self, s: u8) {
        self.state.store(s, Ordering::Release);
    }

    pub(super) fn transition(&self, from: u8, to: u8) -> bool {
        self.state
            .compare_exchange(from, to, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// Zacatek cekani (vola vlakno samo): nove poradove cislo a stav
    /// BLOCKING. Vlakno pak bud zavola `sched::block`, nebo `cancel_wait`.
    pub fn prepare_wait(&self) -> u64 {
        let seq = self.wait_seq.fetch_add(1, Ordering::AcqRel) + 1;
        self.set_state(state::BLOCKING);
        seq
    }

    /// Zruseni cekani bez usnuti.
    pub fn cancel_wait(&self) {
        self.set_state(state::RUNNING);
    }

    pub(super) fn wait_seq(&self) -> u64 {
        self.wait_seq.load(Ordering::Acquire)
    }

    pub(super) fn mark_terminated(&self) {
        self.core.update(Signals::NONE, Signals::TERMINATED);
    }
}

impl KernelObject for Thread {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Thread
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::MANAGE
    }
}
