//! Testy plotu (`object/fence.rs`): signal v kazdem miste mezi kontrolou
//! a usnutim (hacek `wait::test_hook`), soubeh s usinanim z jineho CPU,
//! trvala chyba a opusteni pri zaniku posledni obalky producenta. Po kazdem
//! cekani nesmi u plotu zustat registrace vlakna.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use core::time::Duration;

use super::join;
use crate::arch;
use crate::object::fence::{self, Fence, FenceSignaler};
use crate::object::wait::test_hook::{self, Point};
use crate::object::wait::{self as owait, deadline_after};
use crate::object::{Error, KernelObject, Rights, Signals};
use crate::sched::{self, Qos};

pub const TESTS: &[(&str, fn())] = &[
    ("plot: signal od sebe sama v kazdem miste cekani", signal_from_self),
    ("plot: signal z jineho CPU tesne po usnuti 300x", signal_racing_sleep),
    ("plot: trvala chyba a index polozky", sticky_error),
    ("plot: opusteni po zaniku posledni obalky SIGNAL", abandoned),
];

const PROMPT: Duration = Duration::from_millis(500);
const FAR: Duration = Duration::from_secs(3);

const POINTS: [Point; 3] = [Point::Registered(0), Point::Registered(1), Point::BeforeBlock];

/// Jedno cekani; overi rychle probuzeni a uklid registraci.
fn wait_checked(items: &[(Arc<Fence>, u64)], all: bool, what: &str) -> usize {
    let start = arch::now_ns();
    let r = fence::wait(items, all, Some(deadline_after(FAR)));
    let took = arch::now_ns() - start;
    let index = r.unwrap_or_else(|(e, i)| {
        sched::dump_state(&[sched::current()]);
        panic!("{what}: wait vratil {e:?} (polozka {i:?}) po {} ms", took / 1_000_000)
    });
    assert!(
        took < PROMPT.as_nanos() as u64,
        "{what}: probuzeni trvalo {} ms (ztracene probuzeni?)",
        took / 1_000_000
    );
    for (f, _) in items {
        assert_eq!(f.thread_waiters(), 0, "{what}: registrace zustala u plotu");
    }
    index
}

fn signal_from_self() {
    let me = sched::current();
    for all in [false, true] {
        for point in POINTS {
            for target in 0..2usize {
                let fences = [Fence::new(0), Fence::new(0)];
                if all {
                    // U "vsech" je druha polozka splnena predem.
                    fences[1 - target].signal(2, false).unwrap();
                }
                let fs = fences.clone();
                let fired = Arc::new(AtomicBool::new(false));
                let f = fired.clone();
                test_hook::set(
                    me.id,
                    Some(Arc::new(move |p| {
                        if p == point && !f.swap(true, Ordering::AcqRel) {
                            fs[target].signal(2, false).unwrap();
                        }
                    })),
                );
                let what = alloc::format!("{point:?}, plot {target}, vsechny {all}");
                let items = [(fences[0].clone(), 2), (fences[1].clone(), 2)];
                let index = wait_checked(&items, all, &what);
                test_hook::set(0, None);
                if fired.load(Ordering::Acquire) && !all {
                    assert_eq!(index, target, "{what}: spatny index");
                }
                // Hacek signalizuje jen cilovy plot (u "vsech" je druhy
                // signalizovany predem).
                assert!(fired.load(Ordering::Acquire), "{what}: hacek neprobehl");
                assert!(fences[target].core().signals().intersects(Signals::SIGNALED), "{what}: SIGNALED");
            }
        }
    }
}

/// Pomocnik posune osu s promenlivym zpozdenim po `BeforeBlock`.
fn signal_racing_sleep() {
    const ROUNDS: u64 = 300;
    let f = Fence::new(0);
    let go = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let (ff, g, s) = (f.clone(), go.clone(), stop.clone());
    let helper = sched::spawn("fence-racer", Qos::Normal, move || {
        let mut round = 0u64;
        while !s.load(Ordering::Acquire) {
            let n = g.load(Ordering::Acquire);
            if n == round {
                core::hint::spin_loop();
                continue;
            }
            round = n;
            let delay = (round * 7919) % 40_000;
            let t0 = arch::now_ns();
            while arch::now_ns() - t0 < delay {
                core::hint::spin_loop();
            }
            ff.signal(round, false).unwrap();
        }
    });
    let me = sched::current();
    let g = go.clone();
    let round = Arc::new(AtomicU64::new(0));
    let r = round.clone();
    test_hook::set(
        me.id,
        Some(Arc::new(move |p| {
            if p == Point::BeforeBlock {
                g.store(r.load(Ordering::Acquire), Ordering::Release);
            }
        })),
    );
    for n in 1..=ROUNDS {
        round.store(n, Ordering::Release);
        let what = alloc::format!("kolo {n}");
        // Pomocnik posune osu az po BeforeBlock tohoto kola, takze cekani
        // vzdy usina.
        wait_checked(&[(f.clone(), n)], false, &what);
        assert_eq!(f.info().value, n, "{what}: hodnota po cekani");
    }
    test_hook::set(0, None);
    stop.store(true, Ordering::Release);
    join(&helper, Duration::from_secs(2), "fence-racer");
}

fn sticky_error() {
    let a = Fence::new(0);
    let b = Fence::new(0);
    a.signal(3, false).unwrap();
    b.signal(5, true).unwrap();
    let items = [(a.clone(), 10), (b.clone(), 1)];
    assert_eq!(fence::wait(&items, false, None), Err((Error::FenceFailed, Some(1))));
    assert_eq!(fence::wait(&[(a.clone(), 3)], false, None), Ok(0));
    assert_eq!(b.signal(6, false), Err(Error::BadState));
    // Termin u nedosazene hodnoty.
    let r = fence::wait(&[(a.clone(), 4)], true, Some(deadline_after(Duration::from_millis(20))));
    assert_eq!(r, Err((Error::TimedOut, None)));
    assert_eq!(a.thread_waiters(), 0);
    let i = b.info();
    assert_eq!((i.value, i.failed_at, i.status), (0, 5, Error::FenceFailed.code()));
}

fn abandoned() {
    let f = Fence::new(0);
    let producer: Arc<dyn KernelObject> = FenceSignaler::new(f.clone());
    // Handle bez SIGNAL drzi holy plot, duplikat s SIGNAL tutez obalku.
    let consumer = fence::bind(producer.clone(), Rights::BASIC);
    assert!(fence::fence_of(consumer.clone()).is_ok());
    let dup = fence::bind(producer.clone(), Rights::BASIC | Rights::SIGNAL);
    assert!(Arc::ptr_eq(&dup, &producer));
    drop(dup);
    f.signal(1, false).unwrap();
    // Cekajici vlakno na hodnotu 2: probudi ho az zanik producenta.
    let result = Arc::new(AtomicU64::new(0));
    let (ff, res) = (f.clone(), result.clone());
    let waiter = sched::spawn("fence-waiter", Qos::Normal, move || {
        let r = fence::wait(&[(ff, 2)], false, Some(deadline_after(FAR)));
        res.store(
            match r {
                Err((e, _)) => (-(e.code() as i64)) as u64,
                Ok(_) => 1,
            },
            Ordering::Release,
        );
    });
    owait::sleep(Duration::from_millis(20));
    assert_eq!(result.load(Ordering::Acquire), 0, "cekani skoncilo pred opustenim");
    drop(producer);
    join(&waiter, PROMPT, "fence-waiter");
    assert_eq!(result.load(Ordering::Acquire), (-(Error::FenceAbandoned.code() as i64)) as u64);
    // Dosazena hodnota zustava, nedosazena skonci opustenim.
    assert_eq!(fence::wait(&[(f.clone(), 1)], false, None), Ok(0));
    assert_eq!(fence::wait(&[(f.clone(), 2)], false, None), Err((Error::FenceAbandoned, Some(0))));
    assert_eq!(f.info().status, Error::FenceAbandoned.code());
    drop(consumer);
}
