//! Planovac.
//!
//! - Kazde CPU ma vlastni frontu (jedna na tridu QoS) a vlastni casovace.
//! - Mezi tridami QoS prisna priorita, uvnitr tridy round-robin s kvantem
//!   `TIME_SLICE_NS`. TODO: ferovejsi algoritmus (EEVDF), aby Background
//!   nehladovelo.
//! - Tickless: casovac se nastavuje jen na konec kvanta nebo nejblizsi
//!   probuzeni; necinne CPU spi v `hlt` bez preruseni casovace.
//! - Rozkladani zateze: nova a probuzena vlakna jdou prednostne na necinne
//!   CPU, preempcne vystridane vlakno se presune na necinne CPU, pokud ma
//!   vlastni CPU dalsi praci, a necinne CPU si praci samo bere (stealing).
//!
//! Stavy vlakna viz `thread::state`. Probuzeni pres porovnani `wait_seq`
//! a CAS na stavu, takze se neztrati ani nezdvoji; planne probuzeni je
//! mozne a cekajici kod ho musi snest (podminku overi znovu).

mod percpu;
mod thread;
mod user;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

pub use percpu::{MAX_CPUS, all_cpus, cpu_count, this_cpu, try_this_cpu};
pub use thread::{Qos, Thread};
pub use user::{
    create_user_thread, on_return_to_user, request_kill, start_user_thread, user_fault,
};

use crate::arch::context::{self, Context, ThreadContext};
use crate::arch::{self, interrupts};
use crate::mm::KernelStack;
use percpu::{PerCpu, TimerEntry};
use thread::state;

/// Delka casoveho kvanta.
pub const TIME_SLICE_NS: u64 = 10_000_000;

/// Zaregistruje aktualni CPU v planovaci; prave bezici kontext se stane
/// jeho idle vlaknem.
pub fn init_cpu(index: usize, hw_id: u32) -> &'static PerCpu {
    let name = alloc::format!("idle-{index}");
    let idle = Arc::new(Thread::new(
        &name,
        Qos::Idle,
        None,
        Context::default(),
        None,
        index,
        None,
    ));
    idle.set_state(state::RUNNING);
    idle.on_cpu.store(true, Ordering::Relaxed);
    PerCpu::install(index, hw_id, idle)
}

/// Index aktualniho CPU, pokud uz je CPU zaregistrovane v planovaci.
pub fn try_current_cpu_index() -> Option<usize> {
    percpu::try_this_cpu().map(|c| c.index)
}

/// Aktualni vlakno.
pub fn current() -> Arc<Thread> {
    let irq = interrupts::disable();
    let t = this_cpu().current().clone();
    interrupts::restore(irq);
    t
}

/// Vytvori a spusti vlakno jadra.
pub fn spawn(name: &str, qos: Qos, f: impl FnOnce() + Send + 'static) -> Arc<Thread> {
    assert!(qos != Qos::Idle);
    let stack = KernelStack::new().expect("nedostatek pameti pro zasobnik vlakna");
    // SAFETY: zasobnik je novy a patri jen tomuto vlaknu.
    let ctx = unsafe { Context::new(stack.top(), thread_start) };
    let t = Arc::new(Thread::new(
        name,
        qos,
        Some(stack),
        ctx,
        Some(Box::new(f)),
        this_cpu().index,
        None,
    ));
    t.started.store(true, Ordering::Relaxed);
    enqueue(t.clone(), true);
    t
}

/// Vstupni bod kazdeho noveho vlakna (prvni `switch` sem skoci).
extern "C" fn thread_start() -> ! {
    finish_switch();
    interrupts::enable();
    let f = current()
        .entry
        .lock()
        .take()
        .expect("vlakno bez vstupni funkce");
    f();
    exit()
}

/// Ukonci aktualni vlakno.
pub fn exit() -> ! {
    let me = current();
    if let Some(process) = &me.process {
        process.thread_exited();
    }
    me.set_state(state::DEAD);
    me.mark_terminated();
    drop(me);
    schedule();
    unreachable!("mrtve vlakno bylo znovu naplanovano");
}

/// Dobrovolne prenecha CPU dalsimu vlaknu stejne nebo vyssi tridy.
pub fn yield_now() {
    schedule();
}

/// Uspi aktualni vlakno (po `Thread::prepare_wait`), dokud ho nekdo neprobudi.
pub fn block() {
    schedule();
}

/// Probudi vlakno, pokud stale ceka s poradovym cislem `seq`.
pub fn wake(t: &Arc<Thread>, seq: u64) {
    if t.wait_seq() != seq {
        return;
    }
    loop {
        match t.state() {
            // Jeste neusnulo: zrusime usnuti, vlakno pobezi dal.
            state::BLOCKING => {
                if t.transition(state::BLOCKING, state::RUNNING) {
                    return;
                }
            }
            state::BLOCKED => {
                if t.transition(state::BLOCKED, state::RUNNABLE) {
                    enqueue(t.clone(), false);
                    return;
                }
            }
            _ => return,
        }
    }
}

/// Identifikace naplanovaneho probuzeni (pro zruseni).
pub struct TimerId {
    cpu: usize,
    thread: *const Thread,
    seq: u64,
}

/// Naplanuje probuzeni vlakna `t` (cekani `seq`) v case `deadline`.
/// Kdyz cekani skonci driv, je treba probuzeni zrusit (`cancel_timer`),
/// jinak by zaznam drzel vlakno a zbytecne budil CPU az do terminu.
pub fn add_timer(deadline: u64, t: &Arc<Thread>, seq: u64) -> TimerId {
    // Bez preruseni, aby vlakno mezi vyberem CPU a naprogramovanim casovace
    // nemohlo byt preempcne presunuto jinam (casovac by se nastavil na
    // jinem CPU, nez ve kterem je zaznam, a probuzeni by se ztratilo).
    let irq = interrupts::disable();
    let cpu = this_cpu();
    cpu.timers.lock().push(TimerEntry {
        deadline,
        seq,
        thread: t.clone(),
    });
    program_timer(cpu);
    interrupts::restore(irq);
    TimerId {
        cpu: cpu.index,
        thread: Arc::as_ptr(t),
        seq,
    }
}

/// Zrusi probuzeni z `add_timer` (pokud jeste nenastalo). Casovac CPU se
/// neprenastavuje - pripadne preruseni na prazdnou frontu je neskodne.
pub fn cancel_timer(id: TimerId) {
    if let Some(cpu) = percpu::cpu(id.cpu) {
        cpu.timers
            .lock()
            .retain(|e| !(Arc::as_ptr(&e.thread) == id.thread && e.seq == id.seq));
    }
}

/// Pocet naplanovanych probuzeni na vsech CPU (pro testy a ladeni).
pub fn pending_timers() -> usize {
    all_cpus().map(|c| c.timers.lock().len()).sum()
}

/// Vybere CPU pro vlakno pripravene k behu: jeho posledni CPU, pokud je
/// necinne, jinak libovolne necinne, jinak posledni (kvuli cache).
fn select_cpu(t: &Thread) -> &'static PerCpu {
    let last = percpu::cpu(t.last_cpu.load(Ordering::Relaxed)).unwrap_or_else(this_cpu);
    if last.is_idle.load(Ordering::Relaxed) {
        return last;
    }
    all_cpus()
        .find(|c| c.is_idle.load(Ordering::Relaxed))
        .unwrap_or(last)
}

fn enqueue(t: Arc<Thread>, new: bool) {
    let target = if new && !this_cpu().is_idle.load(Ordering::Relaxed) {
        // Nove vlakno: kdekoli je volno, jinak u rodice.
        all_cpus()
            .find(|c| c.is_idle.load(Ordering::Relaxed))
            .unwrap_or_else(this_cpu)
    } else {
        select_cpu(&t)
    };
    enqueue_on(target, t);
}

fn enqueue_on(target: &'static PerCpu, t: Arc<Thread>) {
    let qos = t.qos as u8;
    target.rq.lock().push(t);
    // Preempce, pokud cil je necinny nebo bezi neco s nizsi prioritou.
    if target.is_idle.load(Ordering::Relaxed) || qos < target.current_qos.load(Ordering::Relaxed) {
        arch::send_reschedule(target.hw_id);
    }
}

fn pick_next(cpu: &PerCpu) -> Option<Arc<Thread>> {
    if let Some(t) = cpu.rq.lock().pop() {
        return Some(t);
    }
    // Stealing: try_lock, aby dve CPU kradouci si navzajem nedeadlockovala.
    for other in all_cpus().filter(|c| c.index != cpu.index) {
        if let Some(t) = other.rq.try_lock().and_then(|mut rq| rq.pop()) {
            return Some(t);
        }
    }
    None
}

/// Proc se prepina.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reason {
    /// Vlakno samo usina, konci nebo prenechava CPU.
    Voluntary,
    /// Preempce z obsluhy preruseni: vlakno muze byt kdekoli, treba
    /// uprostred pripravy na cekani (BLOCKING, jeste nema zaregistrovane
    /// probuzeni) nebo v `exit` pred signalizaci konce. Takove vlakno se
    /// nesmi uspat ani zahodit, jen se vrati do fronty a stav se nemeni.
    Preempt,
}

/// Prepne na dalsi vlakno podle stavu aktualniho.
fn schedule() {
    switch_away(Reason::Voluntary);
}

fn switch_away(reason: Reason) {
    let irq = interrupts::disable();
    let cpu = this_cpu();
    let prev = cpu.current().clone();
    let prev_is_idle = Arc::ptr_eq(&prev, cpu.idle());

    match (reason, prev.state()) {
        _ if prev_is_idle => {}
        (_, state::RUNNING) => {
            prev.set_state(state::RUNNABLE);
            requeue(cpu, prev.clone());
        }
        (Reason::Preempt, _) => requeue(cpu, prev.clone()),
        // Mezitim probuzeno (wake prepnul na RUNNING): bezime dal.
        (Reason::Voluntary, state::BLOCKING)
            if !prev.transition(state::BLOCKING, state::BLOCKED) =>
        {
            interrupts::restore(irq);
            return;
        }
        _ => {} // DEAD, nebo uspesne BLOCKING -> BLOCKED
    }

    let next = pick_next(cpu).unwrap_or_else(|| cpu.idle().clone());
    let next_is_idle = Arc::ptr_eq(&next, cpu.idle());
    cpu.slice_end.store(
        if next_is_idle {
            u64::MAX
        } else {
            arch::now_ns() + TIME_SLICE_NS
        },
        Ordering::Relaxed,
    );
    cpu.is_idle.store(next_is_idle, Ordering::Relaxed);
    cpu.current_qos.store(next.qos as u8, Ordering::Relaxed);
    // Jen RUNNABLE -> RUNNING; preempcne vystridane vlakno si drzi svuj stav
    // (napr. BLOCKING), aby se probuzeni ani konec neztratily.
    next.transition(state::RUNNABLE, state::RUNNING);

    if Arc::ptr_eq(&next, &prev) {
        program_timer(cpu);
        interrupts::restore(irq);
        return;
    }

    // Jine CPU mozna jeste nedokoncilo prepnuti pryc z `next`.
    while next.on_cpu.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    next.on_cpu.store(true, Ordering::Relaxed);
    next.last_cpu.store(cpu.index, Ordering::Relaxed);

    // Adresni prostor a zasobnik jadra pro vstup z userspace noveho vlakna.
    // Vlakna jadra nemaji spodni polovinu (`None`), aby zadne CPU nedrzelo
    // aktivni tabulky procesu, ktery uz mozna zanikl.
    let root = next.process.as_ref().map(|p| p.address_space().root());
    // SAFETY: vlastni CPU, preruseni zakazana; root je platna korenova
    // tabulka se sdilenou polovinou jadra (bezici kod a zasobnik jsou v ni)
    // a proces ji drzi, dokud ma vlakna.
    unsafe {
        arch::switch_user_space(root);
        if let Some(top) = next.kernel_stack_top() {
            arch::set_kernel_stack(&cpu.arch, top);
        }
    }

    let from = prev.context.get();
    let to = next.context.get() as *const Context;
    // SAFETY: vlastni CPU, preruseni zakazana. `prev` zustava drzeny v
    // `prev`, `next` v `current`, takze `from` a `to` zustanou platne.
    unsafe {
        cpu.set_prev(prev);
        cpu.set_current(next);
    }
    program_timer(cpu);
    // SAFETY: `to` je kontext vlakna, ktere nebezi na zadnem CPU (on_cpu
    // jsme prave ziskali), preruseni jsou zakazana.
    unsafe { context::switch(from, to) };

    // Sem se vlakno vrati, az ho nekdo znovu naplanuje.
    finish_switch();
    interrupts::restore(irq);
}

/// Vrati preempcne vystridane vlakno do fronty. Pokud ma toto CPU dalsi
/// praci a nektere jine je necinne, presune ho tam.
fn requeue(cpu: &'static PerCpu, t: Arc<Thread>) {
    let mut rq = cpu.rq.lock();
    if !rq.is_empty() {
        let idle = all_cpus().find(|c| c.index != cpu.index && c.is_idle.load(Ordering::Relaxed));
        if let Some(target) = idle {
            drop(rq);
            enqueue_on(target, t);
            return;
        }
    }
    rq.push(t);
}

/// Dokonceni prepnuti na novem vlakne: predchozi vlakno uz smi bezet jinde.
fn finish_switch() {
    let cpu = this_cpu();
    // SAFETY: vlastni CPU, preruseni zakazana (jsme uvnitr schedule nebo na
    // zacatku noveho vlakna pred povolenim preruseni).
    let prev = unsafe { cpu.take_prev() };
    if let Some(prev) = prev {
        prev.on_cpu.store(false, Ordering::Release);
        // Pokud bylo DEAD a tohle byl posledni odkaz, uvolni se i jeho
        // zasobnik - uz na nem nebezime.
        drop(prev);
    }
}

/// Nastavi casovac CPU na nejblizsi udalost: konec kvanta nebo probuzeni.
/// `cpu` musi byt aktualni CPU a preruseni musi byt zakazana.
fn program_timer(cpu: &PerCpu) {
    debug_assert!(!interrupts::are_enabled());
    debug_assert!(core::ptr::eq(cpu, this_cpu()));
    let next_timer = cpu.timers.lock().peek().map(|e| e.deadline);
    let slice_end = cpu.slice_end.load(Ordering::Relaxed);
    let deadline = match next_timer {
        Some(t) => Some(t.min(slice_end)),
        None if slice_end != u64::MAX => Some(slice_end),
        None => None,
    };
    let now = arch::now_ns();
    arch::set_timer(deadline.map(|d| d.saturating_sub(now).max(1_000)));
}

/// Obsluha preruseni casovace (volana z obsluhy preruseni).
pub fn on_timer_interrupt() {
    let cpu = this_cpu();
    let now = arch::now_ns();
    loop {
        let entry = {
            let mut timers = cpu.timers.lock();
            match timers.peek() {
                Some(e) if e.deadline <= now => timers.pop(),
                _ => None,
            }
        };
        match entry {
            Some(e) => wake(&e.thread, e.seq),
            None => break,
        }
    }
    if now >= cpu.slice_end.load(Ordering::Relaxed) {
        cpu.need_resched.store(true, Ordering::Relaxed);
    }
    program_timer(cpu);
}

/// Obsluha IPI "preplanuj".
pub fn on_reschedule_ipi() {
    this_cpu().need_resched.store(true, Ordering::Relaxed);
}

/// Volano na konci obsluhy preruseni: pripadne prepne vlakno.
pub fn preempt_if_needed() {
    if this_cpu().need_resched.swap(false, Ordering::Relaxed) {
        switch_away(Reason::Preempt);
    }
}

/// Smycka idle vlakna CPU: hleda praci, jinak spi do dalsiho preruseni.
pub fn idle_loop() -> ! {
    loop {
        interrupts::disable();
        let cpu = this_cpu();
        let has_work = !cpu.rq.lock().is_empty()
            || all_cpus()
                .any(|c| c.index != cpu.index && c.rq.try_lock().is_some_and(|rq| !rq.is_empty()));
        if has_work {
            schedule();
            interrupts::enable();
        } else {
            interrupts::enable_and_halt();
        }
    }
}

/// Zastavi vsechna ostatni CPU (pri panice).
///
/// Nesaha na radic preruseni, dokud aktualni CPU neni zaregistrovane: panika muze
/// nastat jeste pred inicializaci radice preruseni a dalsi vyjimka by prekryla puvodni
/// chybu. Pred registraci BSP zadna jina CPU nebezi.
pub fn halt_other_cpus() {
    let Some(me) = percpu::try_this_cpu() else {
        return;
    };
    for cpu in all_cpus().filter(|c| c.index != me.index) {
        arch::send_halt(cpu.hw_id);
    }
}
