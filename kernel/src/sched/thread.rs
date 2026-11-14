//! Vlakno jadra.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

use crate::arch::context::Context;
use crate::mm::KernelStack;
use crate::object::job::Charge;
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

/// Priznaky `Thread::cpu_flags`.
pub(super) mod cpu_flag {
    /// Nektere CPU jeste nedokoncilo prepnuti pryc z vlakna: kontext neni
    /// ulozeny, vlakno nesmi spustit jine CPU ani byt v cizi fronte.
    pub const ON_CPU: u8 = 1;
    /// Probuzeni behem dobihajiciho prepnuti: vlakno uz je RUNNABLE, do
    /// fronty ho zaradi jeho CPU v `finish_switch` (viz `sched::wake`).
    /// Nastavuje se jen spolu s `ON_CPU`.
    pub const WAKE_PENDING: u8 = 2;
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
    /// Priznaky prepinani (`cpu_flag`). Obe informace (vlakno jeste bezi /
    /// ceka na nej probuzeni) jsou v jednom slove a meni se jen atomickymi
    /// operacemi na nem: dvojice samostatnych priznaku (zapis jednoho, cteni
    /// druheho) potrebuje usporadani zapis -> cteni mezi adresami, ktere
    /// QEMU TCG na x86 hostiteli u `stlr` + `ldar` nedodrzi (ztracene
    /// probuzeni, viz `sched::wake`).
    pub(super) cpu_flags: AtomicU8,
    /// Poradove cislo aktualniho cekani; probuzeni se starym cislem se ignoruje.
    wait_seq: AtomicU64,
    pub(super) last_cpu: AtomicUsize,
    pub(super) context: UnsafeCell<Context>,
    /// Vrchol zasobniku jadra (`None` = idle vlakno na zasobniku od startu).
    /// Pevny i po uvolneni zasobniku - mrtve vlakno se uz nespusti.
    stack_top: Option<u64>,
    /// Zasobnik jadra a jeho strzeni z rozpoctu jobu. Uvolni se hned po
    /// poslednim prepnuti pryc z mrtveho vlakna (`finish_switch`), ne az
    /// se zahodi posledni odkaz (handle, fronta futexu): objekt vlakna se
    /// signaly a kodem konce muze zit dlouho, 64 KiB zasobniku ne.
    stack: SpinLock<Option<(KernelStack, Charge)>>,
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
    /// Vlakno konci (`sched::exit`): planovac mu uz nenahraje adresni
    /// prostor procesu (bezi s korenem jadra), aby ho proces mohl zrusit
    /// hned po konci posledniho vlakna.
    pub(super) user_gone: AtomicBool,
    /// Pozastaveni (`THREAD_SUSPEND`) a cekajici zapis kontextu
    /// (`suspend.rs`).
    pub(super) stop: SpinLock<super::suspend::Stop>,
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
            cpu_flags: AtomicU8::new(0),
            wait_seq: AtomicU64::new(0),
            last_cpu: AtomicUsize::new(cpu),
            context: UnsafeCell::new(context),
            stack_top: stack.as_ref().map(KernelStack::top),
            stack: SpinLock::new(stack.map(|s| (s, Charge::none()))),
            entry: SpinLock::new(entry),
            process,
            kill: AtomicBool::new(false),
            started: AtomicBool::new(false),
            user_tp: AtomicU64::new(0),
            user_gone: AtomicBool::new(false),
            stop: SpinLock::new(super::suspend::Stop::default()),
        }
    }

    /// Pozadovana trida vlakna.
    pub fn qos(&self) -> Qos {
        self.sched.lock().base
    }

    /// Vrchol zasobniku jadra (`None` u idle vlaken na zasobniku od startu).
    pub fn kernel_stack_top(&self) -> Option<u64> {
        self.stack_top
    }

    /// Strzeni rozpoctu jobu za zasobnik jadra (vrati se s uvolnenim
    /// zasobniku, ne az se zrusi objekt vlakna).
    pub(super) fn set_stack_charge(&self, charge: Charge) {
        if let Some((_, c)) = &mut *self.stack.lock() {
            *c = charge;
        }
    }

    /// Ma vlakno jeste zasobnik jadra (pro testy)? `false` = zasobnik
    /// i jeho rozpocet jobu uz jsou vracene.
    pub fn has_kernel_stack(&self) -> bool {
        self.stack.lock().is_some()
    }

    /// Vrati rozpocet jobu za zasobnik jadra hned pri konci vlakna
    /// (`sched::exit`, pred `thread_exited` a signalem TERMINATED): kdo
    /// uvidi konec vlakna nebo procesu, vidi rozpocet zpet. Zasobnik sam
    /// (pamet) se uvolni az v `finish_switch`, kdy uz na nem nic nebezi -
    /// do te doby je o 64 KiB vic, nez ukazuje rozpocet (mikrosekundy).
    pub(super) fn return_stack_charge(&self) {
        let charge = self.stack.lock().as_mut().map(|(_, c)| core::mem::take(c));
        drop(charge);
    }

    /// Uvolni zasobnik jadra mrtveho vlakna. Volat jen, kdyz na nem uz
    /// nic nebezi: vlakno je DEAD a jeho CPU dokoncilo prepnuti pryc.
    ///
    /// Rozpocet jobu se vraci jeste pod zamkem: kdo uvidi "bez zasobniku"
    /// ([`Thread::has_kernel_stack`]), uvidi i vraceny rozpocet. Pamet
    /// zasobniku se uvolni az po zamku (vraceni rozpoctu jsou jen atomicke
    /// odecty, uvolneni stranek muze trvat dele).
    pub(super) fn release_stack(&self) {
        let mut slot = self.stack.lock();
        let stack = slot.take().map(|(stack, charge)| {
            drop(charge);
            stack
        });
        drop(slot);
        drop(stack);
    }

    pub fn kill_requested(&self) -> bool {
        self.kill.load(Ordering::Acquire)
    }

    pub(super) fn set_kill(&self) {
        self.kill.store(true, Ordering::Release);
    }

    /// Bezi vlakno na nekterem CPU? Pro toho, kdo prave zmenil stav vlakna
    /// (pozadavek na ukonceni, trida) a chce ho prerusit, kdyz bezi.
    ///
    /// Cteni je atomicka operace RMW na `cpu_flags` (ne obycejne cteni):
    /// CPU, ktere vlakno spousti, nastavuje ON_CPU take RMW a zmenu cte az
    /// po nem. Operace RMW na jednom slove maji jedno poradi, takze bud
    /// uvidime ON_CPU (a posleme IPI), nebo vlakno uvidi zmenu. Obycejne
    /// cteni po zapisu jineho priznaku by pod QEMU TCG na x86 hostiteli
    /// mohlo predbehnout zapis (`stlr` -> `ldar`) a IPI by se neposlalo
    /// (ukonceni vlakna az pri pristim vstupu do jadra).
    pub(super) fn on_cpu_after_update(&self) -> bool {
        self.cpu_flags.fetch_or(0, Ordering::SeqCst) & cpu_flag::ON_CPU != 0
    }

    /// Bezi vlakno jeste na nekterem CPU (`cpu_flag::ON_CPU`)?
    pub(super) fn on_cpu(&self) -> bool {
        self.cpu_flags.load(Ordering::Acquire) & cpu_flag::ON_CPU != 0
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

    /// `MANAGE` = pozastavit, `READ` / `WRITE` = cist / zapsat kontext
    /// (`THREAD_STATE`).
    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::MANAGE | Rights::READ | Rights::WRITE
    }
}
