//! Doruceni vyjimek z userspace na kanaly vyjimek (`EXCEPTION_CHANNEL`).
//!
//! Vyjimka CPU v userspace (po neuspesnem doplneni stranky) se nabidne
//! obsluham v poradi: debugger procesu (prvni sance), kanal vlakna, kanal
//! procesu, debugger (druha sance). Vlakno obsluhy (`HANDLER_THREAD`)
//! preskoci kanaly vlakna a procesu - necekalo by samo na sebe. Kazde
//! obsluze jadro posle zpravu (`abi::exception::Report`) s handlem vlakna
//! a tokenem vyjimky a vlakno ceka, nez obsluha rozhodne
//! (`EXCEPTION_RESOLVE`) nebo token zavre (= dalsi obsluha). Mezitim vlakno
//! stoji a jeho kontext jde cist a zapsat (`THREAD_STATE`); zapis se pouzije
//! pri navratu do userspace.
//!
//! Kill vzdy vyhraje: cekani skonci `Canceled` a vlakno skonci pri navratu.
//! Obsluha, ktera zemre, zavre kanal i tokeny: dalsi obsluha nebo konec
//! procesu.

use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::{AtomicU32, Ordering};

use abi::exception::{Report, action, stage};

use super::channel::{Channel, Message};
use super::{Error, KernelObject, ObjectCore, ObjectType, Rights, Signals};
use crate::sched::{self, Thread};

/// Spolecny stav jedne nabidky vyjimky: rozhodnuti a signal pro cekajici
/// vlakno.
struct Pending {
    core: ObjectCore,
    action: AtomicU32,
}

impl Pending {
    /// Rozhodnuti (prvni vyhrava); signalizuje cekajici vlakno.
    fn decide(&self, a: u32) -> bool {
        let won = self.action.compare_exchange(0, a, Ordering::AcqRel, Ordering::Acquire).is_ok();
        self.core.update(Signals::NONE, Signals::SIGNALED);
        won
    }
}

impl KernelObject for Pending {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Event
    }

    fn default_rights(&self) -> Rights {
        Rights::NONE
    }
}

/// Token vyjimky (handle ve zprave). Zanik bez rozhodnuti = dalsi obsluha.
pub struct ExceptionToken {
    core: ObjectCore,
    pending: Arc<Pending>,
}

impl ExceptionToken {
    /// `EXCEPTION_RESOLVE`: rozhodnuti obsluhy. Vlakno uz rozhodnuto (nebo
    /// skoncilo) = `BadState`.
    pub fn resolve(&self, a: u32) -> Result<(), Error> {
        if !(action::RESUME..=action::KILL).contains(&a) {
            return Err(Error::InvalidArgs);
        }
        if self.pending.decide(a) { Ok(()) } else { Err(Error::BadState) }
    }
}

impl Drop for ExceptionToken {
    fn drop(&mut self) {
        self.pending.decide(action::NEXT);
    }
}

impl KernelObject for ExceptionToken {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::ExceptionToken
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC
    }
}

/// Jak vyjimka dopadla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Obsluha ji vyresila: navrat do userspace (s pripadne zmenenym
    /// kontextem), nebo vlakno konci (kill behem cekani).
    Resume,
    /// Nikdo ji nevyresil nebo obsluha rozhodla KILL: konec procesu.
    Unhandled,
}

/// Nabidne vyjimku aktualniho vlakna obsluham. `report` bez `stage`, `pid`
/// a `tid` (doplni se). Vola se s povolenymi prerusenimi.
pub fn deliver(mut report: Report) -> Outcome {
    let me = sched::current();
    let Some(process) = me.process.clone() else {
        return Outcome::Unhandled;
    };
    report.pid = process.id;
    report.tid = me.id;
    let handler = sched::suspend::is_handler(&me);
    let order = [
        (process.exception_channel(true), stage::DEBUGGER_FIRST),
        (if handler { None } else { sched::suspend::exception_channel(&me) }, stage::THREAD),
        (if handler { None } else { process.exception_channel(false) }, stage::PROCESS),
        (process.exception_channel(true), stage::DEBUGGER_SECOND),
    ];
    for (channel, st) in order {
        let Some(channel) = channel else { continue };
        report.stage = st;
        match offer(&me, &channel, &report) {
            Some(action::RESUME) => return Outcome::Resume,
            Some(action::KILL) => return Outcome::Unhandled,
            // Kill behem cekani: navrat, vlakno skonci v `on_return_to_user`.
            None => return Outcome::Resume,
            _ => {}
        }
    }
    Outcome::Unhandled
}

/// Posle zpravu jedne obsluze a pocka na rozhodnuti. `None` = cekani
/// preruseno (kill); obsluha bez ziveho kanalu = NEXT.
fn offer(me: &Arc<Thread>, channel: &Arc<Channel>, report: &Report) -> Option<u32> {
    let pending = Arc::new(Pending { core: ObjectCore::new(Signals::NONE), action: AtomicU32::new(0) });
    let token = Arc::new(ExceptionToken { core: ObjectCore::new(Signals::NONE), pending: pending.clone() });
    let thread_rights = Rights::BASIC | Rights::READ | Rights::WRITE | Rights::MANAGE;
    let msg = Message::new(
        report.encode().to_vec(),
        vec![
            (me.clone() as Arc<dyn KernelObject>, thread_rights),
            (token as Arc<dyn KernelObject>, Rights::BASIC),
        ],
    );
    if channel.write(msg).is_err() {
        // Obsluha nebezi nebo nestiha (plna fronta): dalsi obsluha.
        return Some(action::NEXT);
    }
    sched::suspend::set_in_exception(me, true);
    let r = super::wait::wait_one(&*pending, Signals::SIGNALED, None);
    sched::suspend::set_in_exception(me, false);
    match r {
        Ok(_) => Some(pending.action.load(Ordering::Acquire)),
        Err(_) => {
            // Kill: token (u obsluhy) uz nic nerozhodne.
            pending.decide(action::NEXT);
            None
        }
    }
}
