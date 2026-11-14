//! Pozastaveni vlakna procesu a jeho kontext (`THREAD_SUSPEND`,
//! `THREAD_STATE`).
//!
//! **Pozastaveni** = pocitadlo tokenu. Dokud je nenulove, vlakno se
//! zastavi na hranici navratu do userspace (`park`): usne a dostane signal
//! `SUSPENDED`. Vlakno, ktere prave spi v syscallu, se pocita jako
//! pozastavene hned (jeho uzivatelsky kontext je ulozeny a nemeni se); kdyz
//! se probudi, zastavi se pred navratem. Bezici vlakno se pres IPI dostane
//! do jadra a zastavi se tam. Kill vzdy vyhraje: zastavene vlakno se
//! probudi a skonci.
//!
//! **Kontext:** cteni jde jen u vlakna, ktere stoji v jadre (zaparkovane
//! nebo spici, neni na CPU) - obecne registry z ramce na vrcholu zasobniku
//! jadra, FPU/SIMD z `Context`. Zapis se neprovadi do ciziho vlakna primo
//! (s jeho FPU by se to minulo pri prepnuti): ulozi se jako cekajici a
//! vlakno ho samo pouzije tesne pred navratem do userspace (registry do
//! ramce, FPU/SIMD primo do registru). Cteni vidi cekajici zapis.
//!
//! **Synchronizace mezi CPU:** kazde rozhodnuti dvou stran jde pres zamek
//! `Thread::stop` nebo RMW na `cpu_flags` (`kernel/CLAUDE.md`, zadny vzor
//! "zapis svuj priznak, precti cizi"):
//! - pozastaveni x usinani: `suspend` zvysi pocitadlo a cte stav vlakna pod
//!   zamkem, usinajici vlakno nastavi BLOCKING pred `note_blocking`, ktery
//!   cte pocitadlo pod tymz zamkem - `SUSPENDED` nastavi aspon jeden z nich;
//! - kill x parkovani: `park` kontroluje kill a zapisuje `parked` pod
//!   zamkem, `request_kill` po nastaveni killu probudi zaparkovane vlakno
//!   pres `kill_wake` (taky pod zamkem) - kill se neztrati;
//! - IPI bezicimu vlaknu: `on_cpu_after_update` (RMW), jako `request_kill`.
//!
//! Soubeh s navratem ze syscallu: spici vlakno muze po probuzeni zapsat
//! navratovou hodnotu (rax / x0) do ramce, nez se zastavi; cekajici zapis
//! debuggeru ji prepise.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::Ordering;
use core::time::Duration;

use super::thread::state;
use super::{Thread, block, current, percpu, wake};
use crate::arch::{self, interrupts, ustate};
use crate::error::Error;
use crate::object::channel::Channel;
use crate::object::{KernelObject, ObjectCore, ObjectType, Rights, Signals};

/// Stav pozastaveni vlakna (pod zamkem `Thread::stop`).
#[derive(Default)]
pub(super) struct Stop {
    /// Zive tokeny pozastaveni.
    suspends: u32,
    /// Vlakno stoji v `park` (spi nebo usina).
    parked: bool,
    /// Stoji pred prvnim vstupem do userspace: kontext jeste neexistuje.
    before_start: bool,
    /// Poradove cislo cekani v `park` (pro probuzeni pri obnoveni).
    park_seq: u64,
    /// Cekajici zapis obecnych registru (overeny `ustate::check_general`).
    general: Option<Vec<u8>>,
    /// Cekajici zapis FPU/SIMD (overeny `ustate::check_fp`).
    fp: Option<Vec<u8>>,
    /// Vlakno stoji ve vyjimce (ceka na obsluhu): kontext jde cist
    /// a zapsat jako u pozastaveneho.
    in_exception: bool,
    /// Kanal vyjimek vlakna (`EXCEPTION_CHANNEL`, konec jadra).
    exc_channel: Option<Arc<Channel>>,
    /// Vlakno obsluhy: jeho vyjimky preskoci kanaly vlakna a procesu.
    handler: bool,
    /// Alternativni zasobnik pro obsluhu ve vlaknu (zacatek, delka).
    alt_stack: (u64, u64),
    /// Vyjimky prave obsluhovane ve vlaknu (vlastni kopie jadra pro NEXT;
    /// delka = hloubka vnoreni).
    in_thread: Vec<abi::exception::Report>,
}

/// Alternativni zasobnik vlakna (zacatek, delka; delka 0 = zadny).
pub fn alt_stack(t: &Thread) -> (u64, u64) {
    t.stop.lock().alt_stack
}

pub fn set_alt_stack(t: &Thread, base: u64, size: u64) {
    t.stop.lock().alt_stack = (base, size);
}

/// Zacina obsluha ve vlaknu: ulozi kopii zpravy. `false` = hloubka
/// vnoreni dosahla `abi::exception::handler::MAX_DEPTH`.
pub fn in_thread_push(t: &Thread, r: abi::exception::Report) -> Result<bool, Error> {
    let mut s = t.stop.lock();
    if s.in_thread.len() >= abi::exception::handler::MAX_DEPTH as usize {
        return Ok(false);
    }
    s.in_thread.try_reserve(1).map_err(|_| Error::NoMemory)?;
    s.in_thread.push(r);
    Ok(true)
}

/// Konci obsluha ve vlaknu: vraci kopii zpravy (`None` = zadna nebezi).
pub fn in_thread_pop(t: &Thread) -> Option<abi::exception::Report> {
    t.stop.lock().in_thread.pop()
}

/// Bezi ve vlaknu obsluha (hloubka > 0)?
pub fn in_thread_depth(t: &Thread) -> usize {
    t.stop.lock().in_thread.len()
}

/// Vlakno stoji ve vyjimce (cteni a zapis kontextu povoleny).
pub fn set_in_exception(t: &Thread, on: bool) {
    t.stop.lock().in_exception = on;
}

/// Kanal vyjimek vlakna (zivy), pokud nejaky je.
pub fn exception_channel(t: &Thread) -> Option<Arc<Channel>> {
    t.stop.lock().exc_channel.clone().filter(|c| !c.peer_closed())
}

/// Nastavi kanal vyjimek vlakna; zivy kanal uz existuje = `AlreadyExists`.
pub fn set_exception_channel(t: &Thread, ch: Arc<Channel>) -> Result<(), Error> {
    user_thread(t)?;
    let mut s = t.stop.lock();
    if s.exc_channel.as_ref().is_some_and(|c| !c.peer_closed()) {
        return Err(Error::AlreadyExists);
    }
    s.exc_channel = Some(ch);
    Ok(())
}

/// Oznaci vlakno jako vlakno obsluhy vyjimek.
pub fn mark_handler(t: &Thread) -> Result<(), Error> {
    user_thread(t)?;
    t.stop.lock().handler = true;
    Ok(())
}

/// Je vlakno vlaknem obsluhy?
pub fn is_handler(t: &Thread) -> bool {
    t.stop.lock().handler
}

fn user_thread(t: &Thread) -> Result<(), Error> {
    if t.process.is_none() {
        return Err(Error::WrongType);
    }
    Ok(())
}

/// Usina nebo spi vlakno v jadre (uzivatelsky kontext ulozeny a pred
/// navratem projde `park`)?
fn sleeping(t: &Thread) -> bool {
    matches!(t.state(), state::BLOCKED | state::BLOCKING)
}

/// Stoji vlakno v jadre mimo CPU (kontext jde cist)?
fn asleep(t: &Thread) -> bool {
    sleeping(t) && !t.on_cpu()
}

/// Pozastavi vlakno (jeden token navic).
pub fn suspend(t: &Arc<Thread>) -> Result<(), Error> {
    user_thread(t)?;
    if t.state() == state::DEAD {
        return Err(Error::BadState);
    }
    let running = {
        let mut s = t.stop.lock();
        s.suspends = s.suspends.checked_add(1).ok_or(Error::NoResources)?;
        if s.suspends != 1 {
            return Ok(());
        }
        // Stav se cte pod zamkem: usinajici vlakno nastavi BLOCKING pred
        // `note_blocking` (cte pocitadlo pod timto zamkem), takze bud ho
        // uvidime tady, nebo ono uvidi nas token.
        if sleeping(t) {
            // Spi v syscallu: pozastavene hned (zastavi se pred navratem).
            t.core().update(Signals::NONE, Signals::SUSPENDED);
            false
        } else {
            true
        }
    };
    // Bezi: preruseni ho dostane do jadra, pri navratu se zastavi. RMW na
    // `cpu_flags`: bud uvidime ON_CPU, nebo vlakno pri navratu do
    // userspace uvidi token (cte ho pod zamkem az po spusteni).
    if running
        && t.on_cpu_after_update()
        && let Some(cpu) = percpu::cpu(t.last_cpu.load(Ordering::Relaxed))
    {
        arch::send_reschedule(cpu.hw_id);
    }
    Ok(())
}

/// Vlakno usina v jadre (`sched::block`, po `prepare_wait`): je-li
/// pozastavene, uz stoji - signal `SUSPENDED`. Doplnuje `suspend`, ktery
/// usinajici vlakno na CPU nemusi stihnout videt.
pub(super) fn note_blocking(me: &Arc<Thread>) {
    if me.process.is_some() && me.stop.lock().suspends != 0 {
        me.core().update(Signals::NONE, Signals::SUSPENDED);
    }
}

/// Po nastaveni killu (`request_kill`): probudi vlakno zaparkovane
/// v `park`. Pod zamkem `stop`: `park` kontroluje kill a zapisuje `parked`
/// pod nim, takze bud kill uvidi `park`, nebo tady uvidime `parked`.
pub(super) fn kill_wake(t: &Arc<Thread>) {
    if t.process.is_none() {
        return;
    }
    let seq = {
        let s = t.stop.lock();
        s.parked.then_some(s.park_seq)
    };
    if let Some(seq) = seq {
        wake(t, seq);
    }
}

/// Obnovi vlakno (token zanikl); posledni token ho rozbehne.
pub fn resume(t: &Arc<Thread>) {
    let wake_seq = {
        let mut s = t.stop.lock();
        s.suspends = s.suspends.saturating_sub(1);
        if s.suspends != 0 {
            return;
        }
        t.core().update(Signals::SUSPENDED, Signals::NONE);
        s.parked.then_some(s.park_seq)
    };
    if let Some(seq) = wake_seq {
        wake(t, seq);
    }
}

/// Hranice navratu do userspace (a prvniho vstupu do nej): pozastavene
/// vlakno tu usne, dokud ho posledni token neobnovi nebo nedostane kill;
/// pak pouzije cekajici zapis kontextu. Kill resi volajici (po navratu).
/// Vraci se se zakazanymi prerusenimi.
pub(super) fn park() {
    park_inner(false);
}

/// Jako [`park`] pred prvnim vstupem vlakna do userspace (kontext jeste
/// neni; cteni a zapis vraci `BadState`).
pub(super) fn park_before_start() {
    park_inner(true);
}

fn park_inner(before_start: bool) {
    let me = current();
    loop {
        let mut s = me.stop.lock();
        if s.suspends == 0 {
            let (general, fp) = (s.general.take(), s.fp.take());
            drop(s);
            if !before_start {
                apply(&me, general, fp);
            }
            return;
        }
        let seq = me.prepare_wait();
        // Kill pod zamkem `stop` spolu se zapisem `parked`: `request_kill`
        // po nastaveni killu bere tento zamek (`kill_wake`) a cekani probudi.
        if me.kill_requested() {
            me.cancel_wait();
            return;
        }
        s.parked = true;
        s.before_start = before_start;
        s.park_seq = seq;
        drop(s);
        me.core().update(Signals::NONE, Signals::SUSPENDED);
        interrupts::enable();
        block();
        interrupts::disable();
        {
            let mut s = me.stop.lock();
            s.parked = false;
            s.before_start = false;
        }
        if me.kill_requested() {
            return;
        }
    }
}

/// Cekajici zapis kontextu do vlastniho vlakna (pred navratem).
fn apply(me: &Arc<Thread>, general: Option<Vec<u8>>, fp: Option<Vec<u8>>) {
    if let Some(g) = general
        && let Some(top) = me.kernel_stack_top()
    {
        // SAFETY: vlastni ramec, zakazana preruseni, overeno pri zapisu.
        let tp = unsafe { ustate::write_general(top, &g) };
        me.user_tp.store(tp, Ordering::Relaxed);
        // SAFETY: vlastni CPU, zakazana preruseni, `tp` v userspace.
        unsafe { arch::set_thread_pointer(tp) };
    }
    if let Some(f) = fp {
        // SAFETY: vlastni vlakno pred navratem, overeno pri zapisu.
        unsafe { ustate::load_fp(&f) };
    }
}

/// Velikost stavu druhu `kind` (`abi::thread::kind`).
pub fn state_size(kind: u32) -> Result<usize, Error> {
    match kind {
        abi::thread::kind::GENERAL => Ok(ustate::GENERAL_SIZE),
        abi::thread::kind::FP => Ok(ustate::FP_SIZE),
        _ => Err(Error::InvalidArgs),
    }
}

/// Precte kontext pozastaveneho vlakna. Pocka (nejvys ~100 ms), nez
/// pozastavene vlakno opravdu stoji v jadre.
pub fn read_state(t: &Arc<Thread>, kind: u32, out: &mut [u8]) -> Result<(), Error> {
    user_thread(t)?;
    if out.len() != state_size(kind)? {
        return Err(Error::InvalidArgs);
    }
    for _ in 0..1000 {
        {
            let s = t.stop.lock();
            if (s.suspends == 0 && !s.in_exception) || s.before_start {
                return Err(Error::BadState);
            }
            let pending = if kind == abi::thread::kind::GENERAL { &s.general } else { &s.fp };
            if let Some(p) = pending {
                out.copy_from_slice(p);
                return Ok(());
            }
            if (s.parked || s.in_exception || asleep(t)) && !t.on_cpu() {
                let seq = t.wait_seq();
                copy_state(t, kind, out)?;
                // Stale stoji (zadne nove cekani, neni na CPU): kopie plati.
                if t.wait_seq() == seq && !t.on_cpu() {
                    return Ok(());
                }
            }
        }
        crate::object::wait::sleep(Duration::from_micros(100));
    }
    Err(Error::BadState)
}

fn copy_state(t: &Thread, kind: u32, out: &mut [u8]) -> Result<(), Error> {
    if kind == abi::thread::kind::GENERAL {
        let top = t.kernel_stack_top().ok_or(Error::WrongType)?;
        // SAFETY: vlakno stoji v jadre (overeno volajicim pod `stop`), jeho
        // uzivatelsky ramec na vrcholu zasobniku se nemeni.
        unsafe { ustate::read_general(top, t.user_tp.load(Ordering::Relaxed), out) };
    } else {
        // SAFETY: vlakno neni na CPU (overeno volajicim), `switch` ho
        // nemeni; FPU stav jadro mezi prepnutimi nepouziva.
        let ctx = unsafe { &*t.context.get() };
        ustate::read_fp(ctx, out);
    }
    Ok(())
}

/// Zapise kontext pozastaveneho vlakna (jako cekajici, vlakno ho pouzije
/// pred navratem do userspace). Neplatny kontext = `InvalidArgs`.
pub fn write_state(t: &Arc<Thread>, kind: u32, data: &[u8]) -> Result<(), Error> {
    user_thread(t)?;
    if data.len() != state_size(kind)? {
        return Err(Error::InvalidArgs);
    }
    let ok = if kind == abi::thread::kind::GENERAL { ustate::check_general(data) } else { ustate::check_fp(data) };
    if !ok {
        return Err(Error::InvalidArgs);
    }
    let mut buf = Vec::new();
    buf.try_reserve_exact(data.len()).map_err(|_| Error::NoMemory)?;
    buf.extend_from_slice(data);
    let mut s = t.stop.lock();
    if (s.suspends == 0 && !s.in_exception) || s.before_start {
        return Err(Error::BadState);
    }
    if kind == abi::thread::kind::GENERAL {
        s.general = Some(buf);
    } else {
        s.fp = Some(buf);
    }
    Ok(())
}

/// Token pozastaveni (`THREAD_SUSPEND`): zanik = obnoveni vlakna.
pub struct SuspendToken {
    core: ObjectCore,
    thread: Arc<Thread>,
}

impl SuspendToken {
    /// Pozastavi vlakno a vrati jeho token.
    pub fn new(thread: Arc<Thread>) -> Result<Arc<Self>, Error> {
        suspend(&thread)?;
        Ok(Arc::new(Self { core: ObjectCore::new(Signals::NONE), thread }))
    }
}

impl Drop for SuspendToken {
    fn drop(&mut self) {
        resume(&self.thread);
    }
}

impl KernelObject for SuspendToken {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::SuspendToken
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC
    }
}
