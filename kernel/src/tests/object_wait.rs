//! Testy `wait_many`: signal prijde v kazdem miste mezi zacatkem cekani
//! a usnutim (pres hacek `wait::test_hook`), z jineho vlakna i z cekajiciho
//! vlakna samotneho, s terminem i bez nej. Po kazdem cekani nesmi u objektu
//! zustat registrace.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;

use super::join;
use crate::arch;
use crate::object::event::Event;
use crate::object::wait::test_hook::{self, Point};
use crate::object::wait::{self, deadline_after};
use crate::object::{KernelObject, Signals};
use crate::sched::{self, Qos};

pub const TESTS: &[(&str, fn())] = &[
    (
        "wait_many: signal z jineho vlakna behem registrace",
        signal_from_other,
    ),
    (
        "wait_many: signal od sebe sama behem registrace",
        signal_from_self,
    ),
    (
        "wait_many: signal z jineho CPU tesne po usnuti 400x",
        signal_racing_sleep,
    ),
];

/// Nejdel, co smi cekani se signalem trvat (termin je mnohem dal: kdyby
/// se probuzeni ztratilo, vlakno by spalo az do terminu).
const PROMPT: Duration = Duration::from_millis(500);
const FAR: Duration = Duration::from_secs(3);

/// Mista, kde hacek posle signal, pro dve polozky.
const POINTS: [Point; 3] = [
    Point::Registered(0),
    Point::Registered(1),
    Point::BeforeBlock,
];

/// Pomocne vlakno: na pokyn `cmd` (1 + index) da signal udalosti a nastavi
/// `done`. `cmd` = usize::MAX ukonci vlakno.
struct Helper {
    cmd: Arc<AtomicUsize>,
    done: Arc<AtomicBool>,
    thread: Arc<sched::Thread>,
}

const STOP: usize = usize::MAX;

impl Helper {
    fn spawn(events: [Arc<Event>; 2]) -> Self {
        let cmd = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let (c, d) = (cmd.clone(), done.clone());
        let thread = sched::spawn("signal-helper", Qos::Normal, move || {
            loop {
                match c.swap(0, Ordering::AcqRel) {
                    0 => sched::yield_now(),
                    STOP => return,
                    n => {
                        events[n - 1].signal();
                        d.store(true, Ordering::Release);
                    }
                }
            }
        });
        Self { cmd, done, thread }
    }

    /// Posle signal udalosti `index` a pocka, az ho pomocnik da. Nesmi
    /// spat (bezi uvnitr `wait_many`, jine cekani by zmenilo `wait_seq`),
    /// jen se toci s moznosti preempce.
    fn signal_and_spin(&self, index: usize) {
        self.done.store(false, Ordering::Release);
        self.cmd.store(index + 1, Ordering::Release);
        let start = arch::now_ns();
        while !self.done.load(Ordering::Acquire) {
            assert!(
                arch::now_ns() - start < 2_000_000_000,
                "pomocne vlakno nedalo signal do 2 s"
            );
            core::hint::spin_loop();
        }
    }

    fn stop(self) {
        self.cmd.store(STOP, Ordering::Release);
        join(&self.thread, Duration::from_secs(2), "signal-helper");
    }
}

fn new_events() -> [Arc<Event>; 2] {
    [Event::new(), Event::new()]
}

/// Zachrana pro cekani bez terminu: kdyz se probuzeni ztrati, vlakno by
/// spalo navzdy a test by visel. Hlidac po `FAR` od zacatku cekani da
/// signal treti udalosti (cekani bez casovace zustane bez casovace)
/// a test pak spadne se zpravou.
struct Rescuer {
    event: Arc<Event>,
    /// Zacatek hlidaneho cekani (ns), 0 = nic se nehlida.
    armed: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Arc<sched::Thread>,
}

impl Rescuer {
    fn spawn() -> Self {
        let event = Event::new();
        let armed = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (e, a, s) = (event.clone(), armed.clone(), stop.clone());
        let thread = sched::spawn("wait-rescuer", Qos::Normal, move || {
            while !s.load(Ordering::Acquire) {
                let since = a.load(Ordering::Acquire);
                if since != 0 && arch::now_ns() - since > FAR.as_nanos() as u64 {
                    e.signal();
                }
                wait::sleep(Duration::from_millis(10));
            }
        });
        Self {
            event,
            armed,
            stop,
            thread,
        }
    }

    fn stop(self) {
        self.stop.store(true, Ordering::Release);
        join(&self.thread, Duration::from_secs(2), "wait-rescuer");
    }
}

/// Jedno cekani na obe udalosti; vrati index, ktery `wait_many` hlasil,
/// a overi, ze nezustaly registrace a cekani netrvalo do terminu.
/// Bez terminu hlida cekani `rescuer` (treti polozka).
fn wait_both(events: &[Arc<Event>; 2], rescuer: Option<&Rescuer>, what: &str) -> usize {
    let start = arch::now_ns();
    let r = match rescuer {
        Some(rs) => {
            rs.event.reset();
            rs.armed.store(start, Ordering::Release);
            let r = wait::wait_many(
                &[
                    (&*events[0], Signals::SIGNALED),
                    (&*events[1], Signals::SIGNALED),
                    (&*rs.event, Signals::SIGNALED),
                ],
                None,
            );
            rs.armed.store(0, Ordering::Release);
            r
        }
        None => wait::wait_many(
            &[
                (&*events[0], Signals::SIGNALED),
                (&*events[1], Signals::SIGNALED),
            ],
            Some(deadline_after(FAR)),
        ),
    };
    let took = arch::now_ns() - start;
    let (index, signals) = r.unwrap_or_else(|e| {
        sched::dump_state(&[sched::current()]);
        panic!("{what}: wait_many vratil {e:?} po {} ms", took / 1_000_000)
    });
    if index == 2 {
        sched::dump_state(&[sched::current()]);
        panic!("{what}: ztracene probuzeni, cekani bez terminu vzbudil az hlidac");
    }
    assert!(
        signals.intersects(Signals::SIGNALED),
        "{what}: signaly {signals:?}"
    );
    assert!(
        took < PROMPT.as_nanos() as u64,
        "{what}: probuzeni trvalo {} ms (ztracene probuzeni, vzbudil az termin?)",
        took / 1_000_000
    );
    for e in events {
        assert_eq!(
            e.core().thread_observers(),
            0,
            "{what}: registrace zustala u objektu"
        );
    }
    index
}

fn signal_from_other() {
    let events = new_events();
    let rescuer = Rescuer::spawn();
    let helper = Arc::new(Helper::spawn(events.clone()));
    let me = sched::current();
    for with_deadline in [false, true] {
        for point in POINTS {
            for target in 0..2 {
                for e in &events {
                    e.reset();
                }
                let h = helper.clone();
                let fired = Arc::new(AtomicBool::new(false));
                let f = fired.clone();
                test_hook::set(
                    me.id,
                    Some(Arc::new(move |p| {
                        if p == point && !f.swap(true, Ordering::AcqRel) {
                            h.signal_and_spin(target);
                        }
                    })),
                );
                let what = alloc::format!(
                    "jine vlakno, {point:?}, udalost {target}, termin {with_deadline}"
                );
                let index = wait_both(&events, (!with_deadline).then_some(&rescuer), &what);
                test_hook::set(0, None);
                // Registered(1) nenastane, kdyz udalost 0 uz signal ma
                // (cekani skonci pri kontrole); pak se signal neposle.
                if fired.load(Ordering::Acquire) {
                    assert_eq!(index, target, "{what}: spatny index");
                }
            }
        }
    }
    test_hook::set(0, None);
    match Arc::try_unwrap(helper) {
        Ok(h) => h.stop(),
        Err(_) => panic!("pomocne vlakno je stale sdilene"),
    }
    rescuer.stop();
}

fn signal_from_self() {
    let events = new_events();
    let rescuer = Rescuer::spawn();
    let me = sched::current();
    for with_deadline in [false, true] {
        for point in POINTS {
            for target in 0..2 {
                for e in &events {
                    e.reset();
                }
                let ev = events.clone();
                let fired = Arc::new(AtomicBool::new(false));
                let f = fired.clone();
                test_hook::set(
                    me.id,
                    Some(Arc::new(move |p| {
                        if p == point && !f.swap(true, Ordering::AcqRel) {
                            // Vlastni registrace: `wake` uvidi BLOCKING.
                            ev[target].signal();
                        }
                    })),
                );
                let what = alloc::format!(
                    "samo sebe, {point:?}, udalost {target}, termin {with_deadline}"
                );
                let index = wait_both(&events, (!with_deadline).then_some(&rescuer), &what);
                test_hook::set(0, None);
                if fired.load(Ordering::Acquire) {
                    assert_eq!(index, target, "{what}: spatny index");
                }
            }
        }
    }
    rescuer.stop();
}

/// Signal z jineho vlakna s promenlivym zpozdenim po tom, co cekajici
/// prosel `BeforeBlock`: trefuje se do usinani (BLOCKING -> BLOCKED,
/// `ON_CPU`, `WAKE_PENDING`) a do probuzeni casovacem.
fn signal_racing_sleep() {
    const ROUNDS: usize = 400;
    let events = new_events();
    let rescuer = Rescuer::spawn();
    let ev = events.clone();
    let go = Arc::new(AtomicUsize::new(0));
    let g = go.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let helper = sched::spawn("signal-racer", Qos::Normal, move || {
        let mut round = 0usize;
        while !s.load(Ordering::Acquire) {
            let n = g.load(Ordering::Acquire);
            if n == round {
                core::hint::spin_loop();
                continue;
            }
            round = n;
            // 0..~40 us podle kola: ruzne faze usinani na druhem CPU.
            let delay = (round as u64 * 7919) % 40_000;
            let t0 = arch::now_ns();
            while arch::now_ns() - t0 < delay {
                core::hint::spin_loop();
            }
            ev[round % 2].signal();
        }
    });
    let me = sched::current();
    let g = go.clone();
    test_hook::set(
        me.id,
        Some(Arc::new(move |p| {
            if p == Point::BeforeBlock {
                g.fetch_add(1, Ordering::AcqRel);
            }
        })),
    );
    let mut indices = Vec::with_capacity(ROUNDS);
    for round in 0..ROUNDS {
        for e in &events {
            e.reset();
        }
        let what = alloc::format!("kolo {round}");
        indices.push(wait_both(
            &events,
            (round % 3 == 0).then_some(&rescuer),
            &what,
        ));
        // Pockat, az pomocnik signal opravdu da (jinak by prisel do
        // dalsiho kola); signaly z tohoto kola pak smaze reset.
        let start = arch::now_ns();
        while !events
            .iter()
            .any(|e| e.core().signals().intersects(Signals::SIGNALED))
        {
            assert!(
                arch::now_ns() - start < 2_000_000_000,
                "{what}: pomocnik nedal signal"
            );
            core::hint::spin_loop();
        }
    }
    test_hook::set(0, None);
    stop.store(true, Ordering::Release);
    join(&helper, Duration::from_secs(2), "signal-racer");
    rescuer.stop();
    assert_eq!(indices.len(), ROUNDS);
}
