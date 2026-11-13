//! Tridy QoS vlaken: ucinna trida (strop jobu, zvednuti pres IPC),
//! nastaveni tridy a Realtime rozpoctu, prijeti Realtime vlaken.
//!
//! Ucinna trida = nizsi z (pozadovana, strop jobu procesu a predku), pak
//! vyssi z (to, platne zvednuti zpravou). Zvednuti strop nerespektuje -
//! je to pujcka od odesilatele, ktery tu tridu skutecne ma (viz
//! `docs/cs/planovac.md`). Realtime se pres IPC pujcuje jako Interactive.

use alloc::sync::Arc;
use core::sync::atomic::Ordering;

use schedcore::Ent;
use schedcore::class::BOOST_WINDOW_NS;
use schedcore::rt::{Admission, Rt};

use super::thread::{Qos, SchedState};
use super::{Thread, percpu};
use crate::arch;
use crate::error::Error;

/// Prijate Realtime vyuziti vsech vlaken (promile kapacity CPU).
static ADMISSION: Admission = Admission::new();

/// Strop tridy vlakna z jobu jeho procesu (vlakna jadra bez stropu).
fn cap(t: &Thread) -> Qos {
    t.process.as_ref().map_or(Qos::Realtime, |p| p.job().qos_cap())
}

/// Ucinna trida; vyprsele zvednuti smaze.
/// Vlastni trida vlakna bez zvednuti (pozadovana, omezena stropem jobu).
fn own(t: &Thread, st: &SchedState) -> Qos {
    let class = st.base.min(cap(t));
    // Realtime bez parametru (napr. strop jobu snizil a zase zvedl) se
    // planuje jako Interactive; parametry se doplni pri nastaveni.
    if class == Qos::Realtime && st.rt.is_none() {
        Qos::Interactive
    } else {
        class
    }
}

fn effective(t: &Thread, st: &mut SchedState, now: u64) -> Qos {
    let class = own(t, st);
    match st.boost {
        Some((b, until)) if now < until => class.max(b),
        Some(_) => {
            st.boost = None;
            class
        }
        None => class,
    }
}

/// Stav pro zarazeni do fronty (vola `enqueue` pred zamcenim fronty).
pub(super) fn prepare_ent(t: &Thread, now: u64) -> Ent {
    let mut st = t.sched.lock();
    let ent = st.ent.take().unwrap_or(Ent::new(st.base));
    st.dirty = false;
    refresh(t, &mut st, ent, now)
}

/// Doplni do `ent` aktualni tridu a Realtime rozpocet.
fn refresh(t: &Thread, st: &mut SchedState, mut ent: Ent, now: u64) -> Ent {
    ent.class = effective(t, st, now);
    if ent.class == Qos::Realtime {
        let (budget, period) = st.rt.expect("Realtime bez parametru");
        if ent.rt.is_none_or(|r| r.budget != budget || r.period != period) {
            ent.rt = Some(Rt::new(budget, period));
        }
    } else {
        ent.rt = None;
    }
    ent
}

/// Prepocita stav vlakna, ktere se vraci do fronty (`ent` z `take_curr`).
pub(super) fn refresh_ent(t: &Thread, ent: Ent, now: u64) -> Ent {
    let mut st = t.sched.lock();
    st.dirty = false;
    refresh(t, &mut st, ent, now)
}

/// Ma se vystridane vlakno zaradit znovu (zmena tridy), nebo jen vratit
/// na sve misto? `queued` = trida, se kterou je ve fronte.
pub(super) fn needs_requeue(t: &Thread, queued: Option<Qos>, now: u64) -> bool {
    let mut st = t.sched.lock();
    let class = effective(t, &mut st, now);
    st.dirty || queued != Some(class)
}

/// Trida, kterou ma vlakno ted (pro umisteni pri presunu).
pub(super) fn current_class(t: &Thread, now: u64) -> Qos {
    let mut st = t.sched.lock();
    effective(t, &mut st, now)
}

pub(super) fn store_ent(t: &Thread, ent: Option<Ent>) {
    t.sched.lock().ent = ent;
}

pub(super) fn take_ent(t: &Thread) -> Ent {
    let mut st = t.sched.lock();
    let ent = st.ent.take().unwrap_or(Ent::new(st.base));
    ent
}

/// Vlakno konci: vrati prijate Realtime vyuziti.
pub(super) fn on_exit(t: &Thread) {
    let mut st = t.sched.lock();
    ADMISSION.release(core::mem::take(&mut st.rt_permille));
}

/// Vrati prijate vyuziti zanikajiciho vlakna, ktere nikdy neskoncilo
/// pres `exit` (nespustene).
pub(super) fn release(permille: u64) {
    ADMISSION.release(permille);
}

/// Nastavi tridu vlakna (a Realtime rozpocet a periodu v ns). Nad strop
/// jobu = `AccessDenied`; Realtime nad kapacitu = `NoResources`.
pub fn set_thread_qos(t: &Arc<Thread>, class: Qos, rt: Option<(u64, u64)>) -> Result<(), Error> {
    if class.above(cap(t)) {
        return Err(Error::AccessDenied);
    }
    let rt = if class == Qos::Realtime {
        Some(rt.ok_or(Error::InvalidArgs)?)
    } else {
        None
    };
    let permille = rt.map_or(0, |(b, p)| Rt::new(b, p).permille());
    {
        let mut st = t.sched.lock();
        if permille > 0 && !ADMISSION.admit(permille, percpu::cpu_count().max(1)) {
            return Err(Error::NoResources);
        }
        ADMISSION.release(core::mem::replace(&mut st.rt_permille, permille));
        st.base = class;
        st.rt = rt;
        st.dirty = true;
        if let Some(ent) = &mut st.ent {
            ent.class = class;
            ent.rt = None;
        }
    }
    kick(t);
    Ok(())
}

/// Bezi-li vlakno na jinem CPU, preplanuje ho (zmena tridy hned).
fn kick(t: &Thread) {
    if t.on_cpu.load(Ordering::Acquire)
        && let Some(cpu) = percpu::cpu(t.last_cpu.load(Ordering::Relaxed))
    {
        arch::send_reschedule(cpu.hw_id);
    }
}

/// Pujcka tridy, kterou nese zprava: trida a konec platnosti (ns).
///
/// Konec urcuje vlakno, ktere tridu skutecne ma (`now + BOOST_WINDOW_NS`
/// pri odeslani). Vlakno, ktere tridu jen pujcenou preposila dal, preda
/// puvodni konec - pujcka se predavanim neprodluzuje, takze dva procesy
/// si ji ping-pongem neudrzi navzdy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QosLoan {
    pub class: Qos,
    pub until: u64,
}

/// Docasne zvedne vlakno pujckou ze zpravy (probuzeni cekajiciho). Jen
/// zvedne - nizsi trida ani drivejsi konec platne zvednuti nezmeni.
pub fn boost(t: &Thread, loan: QosLoan) {
    let class = ipc_form(loan.class);
    let now = arch::now_ns();
    if loan.until <= now {
        return;
    }
    let mut st = t.sched.lock();
    let keep = st
        .boost
        .is_some_and(|(b, u)| u > now && (b.above(class) || (b == class && u >= loan.until)));
    if !keep {
        st.boost = Some((class, loan.until));
        st.dirty = true;
    }
}

/// Aktualni vlakno precetlo zpravu s pujckou `loan` (`None` = od jadra):
/// zvednuti se nastavi na pujcku (i snizi predchozi - obsluhuje se nova
/// zprava), s jejim koncem platnosti.
pub fn boost_current(loan: Option<QosLoan>) {
    let Some(loan) = loan else { return };
    let me = super::current();
    let now = arch::now_ns();
    let mut st = me.sched.lock();
    st.boost = (loan.until > now).then_some((ipc_form(loan.class), loan.until));
    st.dirty = true;
}

/// Realtime se pres IPC pujcuje jako Interactive (rozpocet je vlastnost
/// konkretniho vlakna).
fn ipc_form(class: Qos) -> Qos {
    if class == Qos::Realtime { Qos::Interactive } else { class }
}

/// Pujcka, kterou nese zprava od aktualniho vlakna: vlastni trida s novym
/// oknem, nebo - je-li vlakno zvednute vys - preposlana pujcka
/// s puvodnim koncem platnosti.
pub fn ipc_class() -> Option<QosLoan> {
    let me = super::current();
    let now = arch::now_ns();
    let mut st = me.sched.lock();
    let own = ipc_form(own(&me, &st));
    let loan = match st.boost {
        Some((b, until)) if now < until && b.above(own) => QosLoan { class: b, until },
        Some((_, until)) if now >= until => {
            st.boost = None;
            QosLoan { class: own, until: now + BOOST_WINDOW_NS }
        }
        _ => QosLoan { class: own, until: now + BOOST_WINDOW_NS },
    };
    Some(loan)
}

/// Udaje o tride vlakna (`THREAD_GET`).
pub fn thread_info(t: &Thread) -> abi::sched::QosInfo {
    let now = arch::now_ns();
    let mut st = t.sched.lock();
    let effective = effective(t, &mut st, now);
    let (budget, period) = st.rt.unwrap_or((0, 0));
    abi::sched::QosInfo {
        base: st.base as u8,
        effective: effective as u8,
        job_max: cap(t) as u8,
        _reserved: 0,
        budget_us: (budget / 1000) as u32,
        period_us: (period / 1000) as u32,
        _reserved2: 0,
    }
}

/// Prijate Realtime vyuziti (promile; pro testy).
pub fn rt_admitted() -> u64 {
    ADMISSION.used()
}
