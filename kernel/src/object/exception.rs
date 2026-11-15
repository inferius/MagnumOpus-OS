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

use crate::arch::{interrupts, ustate};
use crate::log;

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
    deliver_from(&me, &process, report, stage::DEBUGGER_FIRST)
}

/// Doruci preruseni vlakna (`THREAD_INTERRUPT`) obsluze ve vlakne; bez
/// obsluhy nebo bez mista na zasobniku se preruseni zahodi (kanaly
/// vyjimek ho nedostavaji - neni to chyba programu). Vola se s
/// povolenymi prerusenimi pred navratem do userspace.
pub fn deliver_interrupt(code: u32) {
    let me = sched::current();
    let Some(process) = me.process.clone() else { return };
    let report = Report {
        kind: abi::exception::kind::INTERRUPT,
        stage: stage::IN_THREAD,
        pid: process.id,
        tid: me.id,
        code: u64::from(code),
        ..Default::default()
    };
    if !in_thread(&me, &process, &report) {
        log::debug!("proces {} ({}): preruseni {code} bez obsluhy ve vlaknu zahozeno", process.id, process.name);
    }
}

/// Poradi obsluh od stupne `from`: debugger (prvni sance), obsluha ve
/// vlaknu, kanal vlakna, kanal procesu, debugger (druha sance).
fn deliver_from(
    me: &Arc<Thread>,
    process: &Arc<super::process::Process>,
    mut report: Report,
    from: u32,
) -> Outcome {
    let handler = sched::suspend::is_handler(me);
    let order = [
        (stage::DEBUGGER_FIRST, process.exception_channel(true)),
        (stage::IN_THREAD, None),
        (stage::THREAD, if handler { None } else { sched::suspend::exception_channel(me) }),
        (stage::PROCESS, if handler { None } else { process.exception_channel(false) }),
        (stage::DEBUGGER_SECOND, process.exception_channel(true)),
    ];
    let start = order.iter().position(|(s, _)| *s == from).unwrap_or(0);
    for (st, channel) in order.into_iter().skip(start) {
        report.stage = st;
        if st == stage::IN_THREAD {
            if !handler && in_thread(me, process, &report) {
                return Outcome::Resume;
            }
            continue;
        }
        let Some(channel) = channel else { continue };
        match offer(me, &channel, &report) {
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
    // Ve vyjimce uz pred odeslanim: obsluha na jinem CPU muze zpravu precist
    // a volat THREAD_STATE driv, nez toto vlakno usne (jinak BAD_STATE;
    // cteni pocka, az vlakno opravdu stoji mimo CPU).
    sched::suspend::set_in_exception(me, true);
    if channel.write(msg).is_err() {
        sched::suspend::set_in_exception(me, false);
        // Obsluha nebezi nebo nestiha (plna fronta): dalsi obsluha.
        return Some(action::NEXT);
    }
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

/// Doruceni primo ve vlaknu: ramec vyjimky na zasobnik (alternativni,
/// je-li nastaveny a vlakno na nem jeste neni), skok na vstup obsluhy.
/// `false` = neni obsluha, hloubka vnoreni vycerpana nebo se ramec
/// nevesel (ochranna oblast pri preteceni zasobniku) - vyjimka jde dal
/// v poradi, nikdy do rekurze.
fn in_thread(me: &Arc<Thread>, process: &super::process::Process, report: &Report) -> bool {
    let entry = process.exception_entry();
    if entry == 0 {
        return false;
    }
    let Some(top) = me.kernel_stack_top() else { return false };
    match sched::suspend::in_thread_push(me, *report) {
        Ok(true) => {}
        _ => {
            log::warn!("proces {} ({}): vyjimka v obsluze nad limit vnoreni, jde dal", process.id, process.name);
            return false;
        }
    }
    let mut general = vec![0u8; ustate::GENERAL_SIZE];
    // SAFETY: vlastni ramec (vlakno je v jadre kvuli vyjimce).
    unsafe { ustate::read_general(top, sched::user_thread_pointer(me), sched::user_second_pointer(me), &mut general) };
    let frame_size = ustate::frame_size();
    let mut frame = vec![0u8; frame_size];
    frame[..Report::SIZE].copy_from_slice(&report.encode());
    let g = abi::exception::frame::GENERAL;
    frame[g..g + general.len()].copy_from_slice(&general);
    // SAFETY: vlastni vlakno, registry FPU patri jemu.
    unsafe { ustate::save_fp(&mut frame[ustate::FRAME_FP..ustate::FRAME_FP + ustate::fp_size()]) };
    let (sp, _) = ustate::general_sp_ret(&general);
    let (alt, alt_len) = sched::suspend::alt_stack(me);
    let base = if alt_len != 0 && !(alt..alt + alt_len).contains(&sp) {
        alt + alt_len
    } else {
        // x86_64: cervena zona 128 B pod SP patri prerusene funkci.
        sp.saturating_sub(128)
    };
    let at = base.saturating_sub(frame_size as u64 + 64) & !63;
    let space = process.address_space();
    let mut call = general.clone();
    let ret_slot = ustate::prepare_handler_call(&mut call, entry, at);
    let written = space.copy_to_user(at, &frame).is_ok()
        && ret_slot.is_none_or(|s| space.copy_to_user(s, &0u64.to_le_bytes()).is_ok())
        && ustate::check_general(&call);
    if !written {
        sched::suspend::in_thread_pop(me);
        log::warn!(
            "proces {} ({}): ramec vyjimky se nevesel na zasobnik ({at:#x}), obsluha ve vlaknu preskocena",
            process.id,
            process.name
        );
        return false;
    }
    let irq = interrupts::disable();
    // SAFETY: vlastni ramec, zakazana preruseni, `call` prosel kontrolou.
    unsafe { ustate::write_general(top, &call) };
    interrupts::restore(irq);
    true
}

/// `EXCEPTION_HANDLER` RETURN: navrat z obsluhy ve vlaknu. Kontext z ramce
/// projde stejnou kontrolou jako `THREAD_STATE`; druh a adresu vyjimky pro
/// NEXT bere jadro ze sve kopie. Vraci hodnotu rax / x0 obnoveneho
/// kontextu (syscall ji zapise jako navratovou hodnotu).
pub fn handler_return(frame_ptr: u64, disposition: u32) -> Result<u64, Error> {
    let me = sched::current();
    let process = me.process.clone().ok_or(Error::WrongType)?;
    if disposition != action::RESUME && disposition != action::NEXT {
        return Err(Error::InvalidArgs);
    }
    if sched::suspend::in_thread_depth(&me) == 0 {
        return Err(Error::BadState);
    }
    let mut frame = vec![0u8; ustate::frame_size()];
    process.address_space().copy_from_user(frame_ptr, &mut frame)?;
    let g = abi::exception::frame::GENERAL;
    let general = &frame[g..g + ustate::GENERAL_SIZE];
    let fp = &frame[ustate::FRAME_FP..ustate::FRAME_FP + ustate::fp_size()];
    if !ustate::check_general(general) || !ustate::check_fp(fp) {
        return Err(Error::InvalidArgs);
    }
    let report = sched::suspend::in_thread_pop(&me).ok_or(Error::BadState)?;
    let top = me.kernel_stack_top().ok_or(Error::WrongType)?;
    let irq = interrupts::disable();
    // SAFETY: vlastni ramec syscallu, zakazana preruseni, overeno vyse.
    // Baze TLS a druhy ukazatel se neobnovuji (behem obsluhy se nemeni,
    // na aarch64 meni TLS program sam).
    unsafe {
        ustate::write_general(top, general);
        ustate::load_fp(fp);
    }
    interrupts::restore(irq);
    let (_, ret) = ustate::general_sp_ret(general);
    // Preruseni vlakna nejde dal (kanaly vyjimek ho nedostavaji): NEXT =
    // jen pokracovat.
    if disposition == action::NEXT
        && report.kind != abi::exception::kind::INTERRUPT
        && deliver_from(&me, &process, report, stage::THREAD) == Outcome::Unhandled
    {
        log::error!(
            "proces {} ({}): neosetrena vyjimka {} na {:#x} (obsluha ve vlaknu ji odmitla)",
            process.id,
            process.name,
            report.kind,
            report.pc
        );
        process.terminate(abi::process::EXIT_FAULT_BASE - report.kind as i64);
    }
    Ok(ret)
}
