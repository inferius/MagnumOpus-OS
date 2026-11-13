//! Vlakno jadra.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use crate::arch::context::Context;
use crate::mm::KernelStack;
use crate::object::process::Process;
use crate::object::{KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sync::SpinLock;

/// Trida kvality sluzby: rika, co vlakno dela, ne jak se ma planovat.
/// Mezi tridami vazene spravedlive deleni (viz `schedcore`), Realtime
/// s pevnym rozpoctem.
pub use schedcore::Class as Qos;

/// Planovaci stav vlakna (pod zamkem `Thread::sched`; poradi zamku:
/// fronta CPU -> stav vlakna, nikdy obracene).
pub(super) struct SchedState {
    /// Pozadovana trida (pred stropem jobu a zvednutim).
    pub base: Qos,
    /// Realtime rozpocet a perioda (ns), jen u `base == Realtime`.
    pub rt: Option<(u64, u64)>,
    /// Prijate vyuziti Realtime (promile), vraci se pri konci nebo zmene.
    pub rt_permille: u64,
    /// Docasne zvednuti zpravou: trida a do kdy plati (ns).
    pub boost: Option<(Qos, u64)>,
    /// Stav ve fronte, kdyz vlakno ve fronte neni (spi, jeste nebezelo).
    pub ent: Option<schedcore::Ent>,
    /// Trida nebo parametry se zmenily: pri dalsim vystridani se vlakno
    /// zaradi znovu (ne jen vrati na sve misto).
    pub dirty: bool,
}

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

impl Drop for SchedState {
    fn drop(&mut self) {
        super::qos::release(self.rt_permille);
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub struct Thread {
    core: ObjectCore,
    #[allow(dead_code, reason = "identifikace vlakna pro syscally a ladeni")]
    pub id: u64,
    pub name: String,
    pub(super) sched: SpinLock<SchedState>,
    pub(super) state: AtomicU8,
    /// True, dokud nejake CPU nedokoncilo prepnuti pryc z tohoto vlakna
    /// (kontext jeste neni ulozeny, nesmi ho spustit jine CPU).
    pub(super) on_cpu: AtomicBool,
    /// Poradove cislo aktualniho cekani; probuzeni se starym cislem se ignoruje.
    wait_seq: AtomicU64,
    pub(super) last_cpu: AtomicUsize,
    pub(super) context: UnsafeCell<Context>,
    stack: Option<KernelStack>,
    pub(super) entry: SpinLock<Option<Box<dyn FnOnce() + Send>>>,
    /// Proces, do ktereho vlakno patri (`None` = vlakno jadra).
    pub process: Option<Arc<Process>>,
    /// Pozadavek na ukonceni (kill procesu): vlakno skonci pri navratu do
    /// userspace nebo z cekani v jadre.
    kill: AtomicBool,
    /// Vlakno uz bylo spusteno (vlakna procesu se vytvari a spousti zvlast).
    pub(super) started: AtomicBool,
    /// Registr ukazatele vlakna userspace (TLS; `THREAD_SET_POINTER`).
    /// Planovac ho nahraje pri kazdem prepnuti na vlakno.
    pub(super) user_tp: AtomicU64,
    /// Probuzeni behem dobihajiciho prepnuti: vlakno uz je RUNNABLE, ale
    /// jeho CPU ho jeste neprepnulo pryc (`on_cpu`). Do fronty ho zaradi
    /// to CPU v `finish_switch` (viz `sched::wake`).
    pub(super) wake_pending: AtomicBool,
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
        process: Option<Arc<Process>>,
    ) -> Self {
        Self {
            core: ObjectCore::new(Signals::NONE),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            name: String::from(name),
            sched: SpinLock::new(SchedState {
                base: qos,
                rt: None,
                rt_permille: 0,
                boost: None,
                ent: None,
                dirty: false,
            }),
            state: AtomicU8::new(state::RUNNABLE),
            on_cpu: AtomicBool::new(false),
            wait_seq: AtomicU64::new(0),
            last_cpu: AtomicUsize::new(cpu),
            context: UnsafeCell::new(context),
            stack,
            entry: SpinLock::new(entry),
            process,
            kill: AtomicBool::new(false),
            started: AtomicBool::new(false),
            user_tp: AtomicU64::new(0),
            wake_pending: AtomicBool::new(false),
        }
    }

    /// Pozadovana trida vlakna.
    pub fn qos(&self) -> Qos {
        self.sched.lock().base
    }

    /// Vrchol zasobniku jadra (`None` u idle vlaken na zasobniku od startu).
    pub fn kernel_stack_top(&self) -> Option<u64> {
        self.stack.as_ref().map(KernelStack::top)
    }

    pub fn kill_requested(&self) -> bool {
        self.kill.load(Ordering::Acquire)
    }

    pub(super) fn set_kill(&self) {
        self.kill.store(true, Ordering::Release);
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

    /// Ceka vlakno prave s poradovym cislem `seq`?
    pub fn wait_seq_is(&self, seq: u64) -> bool {
        self.wait_seq() == seq
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
