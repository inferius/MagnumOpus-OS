//! Planovac.
//!
//! - Kazde CPU ma vlastni frontu (`schedcore::RunQueue`): nejdriv Realtime
//!   vlakna s rozpoctem (EDF), pak spravedlive tridy Interactive, Normal
//!   a Background s vahami 15 : 4 : 1 (vazeny virtualni cas trid), uvnitr
//!   tridy EEVDF s kvantem podle tridy. Nic nehladovi: Background ma na
//!   vytizenem CPU aspon 1/20 casu mimo Realtime. Navrh a srovnani:
//!   `docs/cs/planovac.md`.
//! - Ucinna trida vlakna = pozadovana, omezena stropem jobu, docasne
//!   zvednuta zpravou od vyssi tridy (dedeni pres IPC), viz `qos`.
//! - Tickless: casovac se nastavuje jen na konec kvanta, obnovu Realtime
//!   rozpoctu nebo nejblizsi probuzeni; necinne CPU spi bez preruseni.
//! - Rozkladani zateze podle tridy a kapacity jadra (`balance`, `topo`):
//!   Interactive a Realtime na vykonna jadra, Background na usporna;
//!   necinne CPU si praci samo bere (stealing).
//!
//! Stavy vlakna viz `thread::state`. Probuzeni pres porovnani `wait_seq`
//! a CAS na stavu, takze se neztrati ani nezdvoji; planne probuzeni je
//! mozne a cekajici kod ho musi snest (podminku overi znovu).

mod balance;
mod debug;
mod percpu;
mod qos;
pub mod suspend;
mod thread;
pub mod topo;
mod user;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::Ordering;

pub use debug::dump_state;
pub use percpu::{MAX_CPUS, all_cpus, cpu_count, this_cpu, try_this_cpu};
pub use qos::{QosLoan, boost, boost_current, ipc_class, rt_admitted, set_thread_qos, thread_info};
pub use thread::{Qos, Thread};
pub use user::{
    create_user_thread, on_return_to_user, request_kill, start_user_thread, start_user_thread_in_kernel,
    user_exception, user_fault, user_page_fault,
};

use crate::arch::context::{self, Context, ThreadContext};
use crate::arch::{self, interrupts};
use crate::mm::KernelStack;
use crate::sync::SpinLockGuard;
use percpu::{IDLE_CLASS, PerCpu, RunQueue, TimerEntry};
use thread::{cpu_flag, state};

/// Zaregistruje aktualni CPU v planovaci; prave bezici kontext se stane
/// jeho idle vlaknem.
pub fn init_cpu(index: usize, hw_id: u32) -> &'static PerCpu {
    let name = alloc::format!("idle-{index}");
    // Idle vlakno nikdy neni ve fronte; trida se nepouzije.
    let idle = Arc::new(Thread::new(
        &name,
        Qos::Background,
        None,
        Context::default(),
        None,
        index,
        None,
    ));
    idle.set_state(state::RUNNING);
    idle.cpu_flags.store(cpu_flag::ON_CPU, Ordering::Relaxed);
    let cpu = PerCpu::install(index, hw_id, idle);
    topo::init_this_cpu(cpu);
    cpu
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

fn new_kernel_thread(
    name: &str,
    qos: Qos,
    f: impl FnOnce() + Send + 'static,
) -> Option<Arc<Thread>> {
    let stack = KernelStack::new()?;
    // SAFETY: zasobnik je novy a patri jen tomuto vlaknu.
    let ctx = unsafe { Context::new(stack.top(), thread_start) };
    Some(Arc::new(Thread::new(
        name,
        qos,
        Some(stack),
        ctx,
        Some(Box::new(f)),
        this_cpu().index,
        None,
    )))
}

/// Vytvori a spusti vlakno jadra.
pub fn spawn(name: &str, qos: Qos, f: impl FnOnce() + Send + 'static) -> Arc<Thread> {
    let t = new_kernel_thread(name, qos, f).expect("nedostatek pameti pro zasobnik vlakna");
    t.started.store(true, Ordering::Relaxed);
    enqueue(t.clone(), Enq::New);
    t
}

/// Vytvori a spusti vlakno jadra tridy Realtime s rozpoctem `budget` ns
/// v kazde `period` ns (testy, ovladace v jadre).
pub fn spawn_realtime(
    name: &str,
    budget: u64,
    period: u64,
    f: impl FnOnce() + Send + 'static,
) -> Result<Arc<Thread>, crate::error::Error> {
    let t = new_kernel_thread(name, Qos::Normal, f).ok_or(crate::error::Error::NoMemory)?;
    set_thread_qos(&t, Qos::Realtime, Some((budget, period)))?;
    t.started.store(true, Ordering::Relaxed);
    enqueue(t.clone(), Enq::New);
    Ok(t)
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
        // Nejdriv pryc z adresniho prostoru procesu: po `thread_exited`
        // posledniho vlakna ho `Process::finish` hned zrusi. Priznak plati
        // drive nez prepnuti, takze ani preruseni mezi nimi koren procesu
        // znovu nenahraje.
        me.user_gone.store(true, Ordering::Release);
        let irq = interrupts::disable();
        // SAFETY: vlastni CPU, preruseni zakazana; koren jadra obsahuje
        // celou horni polovinu (bezici kod a zasobnik), spodni polovinu
        // vlakno uz nepotrebuje.
        unsafe { arch::switch_user_space(None) };
        interrupts::restore(irq);
        process.thread_exited();
    }
    qos::on_exit(&me);
    me.set_state(state::DEAD);
    me.mark_terminated();
    drop(me);
    schedule();
    unreachable!("mrtve vlakno bylo znovu naplanovano");
}

/// Ulozena baze TLS vlakna (u bezici aarch64 muze byt starsi nez registr).
pub fn user_thread_pointer(t: &Thread) -> u64 {
    t.user_tp.load(Ordering::Relaxed)
}

/// Nastavi registr ukazatele vlakna (TLS) aktualniho vlakna.
pub fn set_user_thread_pointer(value: u64) {
    let irq = interrupts::disable();
    current().user_tp.store(value, Ordering::Relaxed);
    // SAFETY: vlastni CPU, preruseni zakazana, hodnotu zkontroloval syscall.
    unsafe { arch::set_thread_pointer(value) };
    interrupts::restore(irq);
}

/// Dobrovolne prenecha CPU (vlakno se zaradi zpet podle sve tridy).
pub fn yield_now() {
    schedule();
}

/// Uspi aktualni vlakno (po `Thread::prepare_wait`), dokud ho nekdo neprobudi.
pub fn block() {
    // Pozastavene vlakno, ktere usina v syscallu, uz stoji (`SUSPENDED`).
    suspend::note_blocking(&current());
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
                    // Vlakno, ktere prave usnulo, muze jeste dobihat na svem
                    // CPU (`ON_CPU`). Do fronty ho smi dat az to CPU po
                    // prepnuti: jinak by ho jine CPU vybralo a cekalo na nej
                    // se zakazanymi prerusenimi, a pokud by to CPU cekalo na
                    // vlakno od nej, obe by stala navzdy. Bezi-li jeste,
                    // nechame mu `WAKE_PENDING` a zaradi ho `finish_switch`.
                    // Rozhodnuti je CAS na jednom slove spolu s `ON_CPU`:
                    // bud uvidime ON_CPU shozene (prepnuti skoncilo,
                    // zaradime sami), nebo `finish_switch` uvidi nas priznak.
                    let mut f = t.cpu_flags.load(Ordering::Acquire);
                    loop {
                        if f & cpu_flag::ON_CPU == 0 {
                            enqueue(t.clone(), Enq::Wake);
                            break;
                        }
                        match t.cpu_flags.compare_exchange_weak(
                            f,
                            f | cpu_flag::WAKE_PENDING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        ) {
                            Ok(_) => break,
                            Err(now) => f = now,
                        }
                    }
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

/// Rozbehnuty (jeste nepripsany) cas CPU procesu `pid` na vsech CPU, kde
/// prave bezi jeho vlakno. `Process::cpu_ns` se pricita az pri prepnuti;
/// soucet obou je aktualni cas CPU (odhad bez zamku, muze o par us
/// zaostavat nebo se zdvojit s prave probihajicim prepnutim).
pub fn running_ns(pid: u64) -> u64 {
    if pid == 0 {
        return 0;
    }
    let now = arch::now_ns();
    all_cpus()
        .filter(|c| c.current_pid.load(Ordering::Relaxed) == pid)
        .map(|c| now.saturating_sub(c.run_start.load(Ordering::Relaxed)))
        .sum()
}

/// Pocet naplanovanych probuzeni na vsech CPU (pro testy a ladeni).
pub fn pending_timers() -> usize {
    all_cpus().map(|c| c.timers.lock().len()).sum()
}

/// Jak vlakno prichazi do fronty.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Enq {
    /// Nove vlakno (umisteni podle CPU rodice).
    New,
    /// Probuzene ze spanku (bez nasetreneho kreditu, preempce podle tridy).
    Wake,
}

fn enqueue(t: Arc<Thread>, how: Enq) {
    let now = arch::now_ns();
    let ent = qos::prepare_ent(&t, now);
    let last = match how {
        Enq::New => percpu::try_this_cpu().map_or(0, |c| c.index),
        Enq::Wake => t.last_cpu.load(Ordering::Relaxed),
    };
    let target = balance::select_cpu(ent.class, last);
    enqueue_on(target, t, ent, now, how == Enq::Wake);
}

fn enqueue_on(target: &'static PerCpu, t: Arc<Thread>, ent: schedcore::Ent, now: u64, wake: bool) {
    let realtime = ent.class == Qos::Realtime;
    let preempt = {
        let mut rq = target.rq.lock();
        let p = rq.enqueue(t, ent, now, wake);
        target.sync(&rq);
        p
    };
    // Realtime na cizim CPU: at si prenastavi casovac obnovy rozpoctu.
    let remote = percpu::try_this_cpu().is_none_or(|c| c.index != target.index);
    if preempt || (realtime && remote) {
        arch::send_reschedule(target.hw_id);
    }
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
    let now = arch::now_ns();
    let mut rq = cpu.rq.lock();

    match (reason, prev.state()) {
        _ if prev_is_idle => {}
        (_, state::RUNNING) => {
            prev.set_state(state::RUNNABLE);
            if reason == Reason::Voluntary {
                rq.update_curr(now);
                rq.yield_curr();
            }
            rq = requeue(cpu, rq, prev.clone(), now);
        }
        (Reason::Preempt, _) => rq = requeue(cpu, rq, prev.clone(), now),
        (Reason::Voluntary, state::BLOCKING) => {
            // Stav fronty ulozit drive, nez vlakno usne: probouzejici CPU
            // ho hned potrebuje (zbytek Realtime rozpoctu, zpozdeni).
            qos::store_ent(&prev, rq.take_curr(now));
            if !prev.transition(state::BLOCKING, state::BLOCKED) {
                // Mezitim probuzeno (wake prepnul na RUNNING): bezime dal.
                rq.resume(qos::take_ent(&prev), now);
                cpu.sync(&rq);
                drop(rq);
                interrupts::restore(irq);
                return;
            }
        }
        _ => {
            // DEAD (dobrovolne z `exit`): z fronty pryc a po prepnuti uvolnit
            // zasobnik jadra. Preempce DEAD vlakna (pred jeho `schedule`)
            // jde vetvi `Preempt` a zasobnik zustane - vlakno jeste pobezi.
            rq.take_curr(now);
            // SAFETY: vlastni CPU, preruseni zakazana.
            unsafe { cpu.set_exited() };
        }
    }

    let mut picked = rq.pick(now);
    if picked.is_none()
        && let Some((t, ent)) = balance::steal(cpu)
    {
        rq.enqueue(t, ent, now, false);
        picked = rq.pick(now);
    }
    cpu.sync(&rq);
    let class_now = rq.curr_class();
    drop(rq);
    let (next, slice) = picked.unwrap_or_else(|| (cpu.idle().clone(), u64::MAX));
    let next_is_idle = Arc::ptr_eq(&next, cpu.idle());
    // Cas CPU procesu (a necinnosti): od posledniho prepnuti na tomto CPU.
    let ran = now.saturating_sub(cpu.run_start.swap(now, Ordering::Relaxed));
    if prev_is_idle {
        cpu.idle_ns.fetch_add(ran, Ordering::Relaxed);
    } else if let Some(proc) = &prev.process {
        proc.cpu_ns.fetch_add(ran, Ordering::Relaxed);
    }
    cpu.slice_end.store(
        if next_is_idle {
            u64::MAX
        } else {
            now.saturating_add(slice)
        },
        Ordering::Relaxed,
    );
    cpu.is_idle.store(next_is_idle, Ordering::Relaxed);
    cpu.current_pid
        .store(next.process.as_ref().map_or(0, |p| p.id), Ordering::Relaxed);
    cpu.current_qos
        .store(class_now.map_or(IDLE_CLASS, |c| c as u8), Ordering::Relaxed);
    cpu.current_tid.store(next.id, Ordering::Relaxed);
    cpu.switches.fetch_add(1, Ordering::Relaxed);
    // Jen RUNNABLE -> RUNNING; preempcne vystridane vlakno si drzi svuj stav
    // (napr. BLOCKING), aby se probuzeni ani konec neztratily.
    next.transition(state::RUNNABLE, state::RUNNING);

    if Arc::ptr_eq(&next, &prev) {
        program_timer(cpu);
        interrupts::restore(irq);
        return;
    }

    // Jine CPU mozna jeste nedokoncilo prepnuti pryc z `next`.
    while next.on_cpu() {
        core::hint::spin_loop();
    }
    // `last_cpu` pred ON_CPU: kdo uvidi ON_CPU, posle IPI sem (viz
    // `on_cpu_after_update`). RMW (ne zapis), aby poradi vuci nemu urcovalo
    // jedno slovo; SeqCst, aby se zmeny stavu vlakna (ukonceni) cetly az po nem.
    next.last_cpu.store(cpu.index, Ordering::Relaxed);
    next.cpu_flags.fetch_or(cpu_flag::ON_CPU, Ordering::SeqCst);

    // Adresni prostor a zasobnik jadra pro vstup z userspace noveho vlakna.
    // Vlakna jadra nemaji spodni polovinu (`None`), aby zadne CPU nedrzelo
    // aktivni tabulky procesu, ktery uz mozna zanikl.
    // Konci-li vlakno (`user_gone`), koren procesu uz nepouzit: proces ho
    // muze prave rusit (`Process::finish`).
    let root = match &next.process {
        Some(p) if !next.user_gone.load(Ordering::Acquire) => p.address_space().root(),
        _ => None,
    };
    // SAFETY: vlastni CPU, preruseni zakazana; root je platna korenova
    // tabulka se sdilenou polovinou jadra (bezici kod a zasobnik jsou v ni)
    // a proces ji drzi, dokud ma ziva vlakna (vlakno bez `user_gone`
    // znamena `live > 0`, takze `teardown` jeste neprobehl).
    unsafe {
        arch::switch_user_space(root);
        if let Some(top) = next.kernel_stack_top() {
            arch::set_kernel_stack(&cpu.arch, top);
        }
    }

    // Ukazatel vlakna userspace (TLS): vzdy hodnota noveho vlakna, aby
    // nic neprosaklo mezi vlakny ani procesy.
    // SAFETY: vlastni CPU, preruseni zakazana; hodnotu nastavil jen
    // `THREAD_SET_POINTER` (kontrola adresy userspace) nebo je 0.
    unsafe { arch::switch_thread_pointer(&prev.user_tp, next.user_tp.load(Ordering::Relaxed)) };

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

/// Vrati vystridane vlakno do fronty. Pokud ma toto CPU dalsi praci nebo
/// je vlakno na spatnem typu jadra a vhodne CPU je necinne, presune ho
/// tam - ale az po dokonceni prepnuti (`finish_switch`): dokud vlakno bezi
/// tady, nesmi byt v cizi fronte (cizi CPU by na nej cekalo se zakazanymi
/// prerusenimi; dve CPU, ktera si vymeni vlakna, by cekala navzajem).
/// Zmenena trida (nastaveni, zvednuti, strop jobu) = nove zarazeni misto
/// navratu na puvodni misto.
fn requeue<'a>(
    cpu: &'static PerCpu,
    mut rq: SpinLockGuard<'a, RunQueue>,
    t: Arc<Thread>,
    now: u64,
) -> SpinLockGuard<'a, RunQueue> {
    let class = qos::current_class(&t, now);
    // Necinne vhodnejsi CPU, jinak mene zatizene (vyvazovani vytizenych).
    let target = if !rq.is_empty() || balance::misfit(cpu, class) {
        balance::idle_target(cpu, class).or_else(|| {
            if rq.is_empty() {
                None
            } else {
                cpu.sync(&rq);
                balance::push_target(cpu, class)
            }
        })
    } else {
        None
    };
    if let Some(target) = target
        && let Some(ent) = rq.take_curr(now)
    {
        let ent = qos::refresh_ent(&t, ent, now);
        // SAFETY: vlastni CPU, preruseni zakazana (uvnitr `switch_away`).
        unsafe { cpu.set_push(target.index, ent) };
        return rq;
    }
    if qos::needs_requeue(&t, rq.curr_requested_class(), now) {
        if let Some(ent) = rq.take_curr(now) {
            let ent = qos::refresh_ent(&t, ent, now);
            rq.enqueue(t, ent, now, false);
        }
    } else {
        rq.put_curr(t, now);
    }
    rq
}

/// Dokonceni prepnuti na novem vlakne: predchozi vlakno uz smi bezet jinde.
fn finish_switch() {
    let cpu = this_cpu();
    // SAFETY: vlastni CPU, preruseni zakazana (jsme uvnitr schedule nebo na
    // zacatku noveho vlakna pred povolenim preruseni).
    let prev = unsafe { cpu.take_prev() };
    // SAFETY: jako vyse.
    let push = unsafe { cpu.take_push() };
    // SAFETY: jako vyse.
    let exited = unsafe { cpu.take_exited() };
    if let Some(prev) = prev {
        // Shodi ON_CPU a zaroven vezme pripadne probuzeni (jedna operace,
        // viz `wake`).
        let flags = prev.cpu_flags.swap(0, Ordering::AcqRel);
        // Odlozeny presun na jine CPU (`requeue`): vlakno uz nebezi.
        if let Some((target, ent)) = push {
            let target = percpu::cpu(target).unwrap_or(cpu);
            enqueue_on(target, prev.clone(), ent, arch::now_ns(), false);
        }
        // Mrtve vlakno: kontext uz je ulozeny a nikdy se nenahraje, na
        // zasobniku nic nebezi (bezime na zasobniku `next`). Zasobnik
        // a jeho cast rozpoctu jobu jdou zpet hned; objekt vlakna (signaly,
        // kod konce) zije dal, dokud ho nekdo drzi.
        if exited {
            prev.release_stack();
        }
        // Probuzeni behem prepnuti (`wake`).
        if flags & cpu_flag::WAKE_PENDING != 0 {
            enqueue(prev.clone(), Enq::Wake);
        }
        // Pokud bylo DEAD a tohle byl posledni odkaz, uvolni se i jeho
        // zasobnik - uz na nem nebezime.
        drop(prev);
    }
}

/// Nastavi casovac CPU na nejblizsi udalost: konec kvanta, obnova
/// Realtime rozpoctu nebo probuzeni. `cpu` musi byt aktualni CPU
/// a preruseni musi byt zakazana.
fn program_timer(cpu: &PerCpu) {
    debug_assert!(!interrupts::are_enabled());
    debug_assert!(core::ptr::eq(cpu, this_cpu()));
    let next_timer = cpu.timers.lock().peek().map(|e| e.deadline);
    let slice_end = cpu
        .slice_end
        .load(Ordering::Relaxed)
        .min(cpu.replenish_at.load(Ordering::Relaxed));
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
    let tick = {
        let mut rq = cpu.rq.lock();
        let t = rq.tick(now);
        cpu.sync(&rq);
        t
    };
    if now >= cpu.slice_end.load(Ordering::Relaxed) || tick {
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
        let has_work = !cpu.rq.lock().is_empty() || balance::stealable(cpu);
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
