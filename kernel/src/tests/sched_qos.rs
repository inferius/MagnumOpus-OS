//! Testy trid QoS planovace v QEMU: latence pod zatezi, zaruceny podil
//! Background, dedeni pres IPC, Realtime rozpocet, strop jobu
//! a mikrobenchmark prepnuti kontextu.
//!
//! QEMU neumi hybridni CPU: tady se overuje homogenni cesta. Hybridni
//! umisteni overuji jen simulace v `libs/schedcore` (`sim_tests.rs`).
//!
//! Namerena cisla se loguji (`planovac:`) a jsou v `docs/cs/planovac.md`.

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use core::time::Duration;

use super::join;
use crate::arch;
use crate::log;
use crate::object::channel::{Channel, Message};
use crate::object::event::Event;
use crate::object::job::Job;
use crate::object::port::Port;
use crate::object::wait::{self, deadline_after};
use crate::object::{Error, KernelObject, Signals};
use crate::sched::{self, Qos, Thread};
use crate::sync::SpinLock;

pub const TESTS: &[(&str, fn())] = &[
    ("qos: latence vstup -> snimek pod zatezi pozadi", input_to_frame_latency),
    ("qos: Background dostane zaruceny podil", background_share),
    ("qos: dedeni tridy pres kanal a port", ipc_inheritance),
    ("qos: zvednuti vyprsi", boost_expires),
    ("qos: Realtime prijeti a latence periody", realtime),
    ("qos: strop tridy z jobu", job_cap),
    ("qos: prepnuti kontextu (mikrobenchmark)", switch_benchmark),
];

const US: u64 = 1_000;
const MS: u64 = 1_000_000;

/// Zatez: `n` smycek dane tridy, kazda pocita svuj cas behu (soucet
/// kratkych mezer mezi dvema ctenimi hodin = bezela, delsi = byla vystridana).
struct Load {
    stop: Arc<AtomicBool>,
    threads: Vec<Arc<Thread>>,
    ran: Arc<AtomicU64>,
}

fn load(class: Qos, n: usize) -> Load {
    let stop = Arc::new(AtomicBool::new(false));
    let ran = Arc::new(AtomicU64::new(0));
    let threads = (0..n)
        .map(|_| {
            let (stop, ran) = (stop.clone(), ran.clone());
            sched::spawn("zatez", class, move || {
                let mut mine = 0;
                let mut last = arch::now_ns();
                while !stop.load(Ordering::Relaxed) {
                    let t = arch::now_ns();
                    let d = t - last;
                    if d < 100 * US {
                        mine += d;
                    }
                    last = t;
                }
                ran.fetch_add(mine, Ordering::Relaxed);
            })
        })
        .collect();
    Load { stop, threads, ran }
}

impl Load {
    fn finish(self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        for t in &self.threads {
            join(t, Duration::from_secs(10), "zatez");
        }
        self.ran.load(Ordering::Relaxed)
    }
}

fn busy(ns: u64) {
    let end = arch::now_ns() + ns;
    while arch::now_ns() < end {
        core::hint::spin_loop();
    }
}

fn wait_reset(e: &Event) {
    wait::wait_one(e, Signals::SIGNALED, Some(deadline_after(Duration::from_secs(5)))).expect("udalost neprisla");
    e.reset();
}

fn stats(v: &mut [u64]) -> (u64, u64) {
    v.sort_unstable();
    (v[v.len() / 2], v[v.len() - 1])
}

/// "Ovladac" (Interactive) -> "inputd" -> "compd" vykresli snimek (200 us),
/// na vsech CPU bezi smycky Background a Normal.
fn input_to_frame_latency() {
    const ROUNDS: usize = 40;
    let n = sched::cpu_count();
    let bg = load(Qos::Background, 2 * n);
    let normal = load(Qos::Normal, n);
    let (irq, to_comp, frame) = (Event::new(), Event::new(), Event::new());
    let (i1, c1) = (irq.clone(), to_comp.clone());
    let inputd = sched::spawn("inputd", Qos::Interactive, move || {
        for _ in 0..ROUNDS {
            wait_reset(&i1);
            c1.signal();
        }
    });
    let (c2, f2) = (to_comp.clone(), frame.clone());
    let compd = sched::spawn("compd", Qos::Interactive, move || {
        for _ in 0..ROUNDS {
            wait_reset(&c2);
            busy(200 * US);
            f2.signal();
        }
    });
    let lat = Arc::new(SpinLock::new(Vec::new()));
    let l = lat.clone();
    let driver = sched::spawn("ovladac", Qos::Interactive, move || {
        for _ in 0..ROUNDS {
            wait::sleep(Duration::from_millis(3));
            let t0 = arch::now_ns();
            irq.signal();
            wait_reset(&frame);
            l.lock().push(arch::now_ns() - t0);
        }
    });
    for t in [&driver, &inputd, &compd] {
        join(t, Duration::from_secs(20), "cesta vstupu");
    }
    bg.finish();
    normal.finish();
    let mut v = core::mem::take(&mut *lat.lock());
    let (median, max) = stats(&mut v);
    log::info!(
        "planovac: latence vstup -> snimek ({n} CPU, {} smycek pozadi): median {} us, max {} us",
        3 * n,
        median / US,
        max / US
    );
    // TCG je pomale a nepresne; snimek sam trva 200 us.
    assert!(median < 3 * MS, "median latence {median} ns");
    assert!(max < 40 * MS, "max latence {max} ns");
}

/// Na kazde CPU pripada jedna smycka Interactive a jedna Background:
/// Background musi dostat aspon 4 % (zaruka 1/16 pri vahach 15 : 1).
fn background_share() {
    let n = sched::cpu_count();
    let inter = load(Qos::Interactive, n);
    let bg = load(Qos::Background, n);
    wait::sleep(Duration::from_millis(600));
    let (b, i) = (bg.finish(), inter.finish());
    let permille = b * 1000 / (b + i).max(1);
    log::info!("planovac: podil Background pod zatezi Interactive: {permille} promile");
    assert!(permille >= 40, "Background dostal jen {permille} promile");
    assert!(permille <= 300, "Background predbiha Interactive: {permille} promile");
}

/// Normal obsluha probuzena zpravou od Interactive bezi jako Interactive;
/// port si tridu zapamatuje u paketu.
fn ipc_inheritance() {
    let (client_end, server_end) = Channel::create();
    let at_wake = Arc::new(AtomicU8::new(0xFF));
    let after_read = Arc::new(AtomicU8::new(0xFF));
    let (w, r) = (at_wake.clone(), after_read.clone());
    let s_end = server_end.clone();
    let server = sched::spawn("obsluha", Qos::Normal, move || {
        wait::wait_one(&*s_end, Signals::READABLE, Some(deadline_after(Duration::from_secs(5)))).unwrap();
        let me = sched::current();
        w.store(sched::thread_info(&me).effective, Ordering::Relaxed);
        let msg = s_end.read(64, 0).unwrap();
        sched::boost_current(msg.qos);
        r.store(sched::thread_info(&me).effective, Ordering::Relaxed);
        assert_eq!(me.qos(), Qos::Normal, "zakladni trida se nemeni");
        s_end.write(Message::new(vec![2], Vec::new())).unwrap();
    });
    // Obsluha musi cekat drive, nez prijde zprava.
    wait::sleep(Duration::from_millis(20));
    let c_end = client_end.clone();
    let client = sched::spawn("klient", Qos::Interactive, move || {
        let mut m = Message::new(vec![1], Vec::new());
        m.qos = sched::ipc_class();
        assert_eq!(m.qos, Some(Qos::Interactive));
        c_end.write(m).unwrap();
        wait::wait_one(&*c_end, Signals::READABLE, Some(deadline_after(Duration::from_secs(5)))).unwrap();
    });
    join(&client, Duration::from_secs(5), "klient");
    join(&server, Duration::from_secs(5), "obsluha");
    assert_eq!(at_wake.load(Ordering::Relaxed), Qos::Interactive as u8, "probuzeni nezvedlo tridu");
    assert_eq!(after_read.load(Ordering::Relaxed), Qos::Interactive as u8, "cteni nezvedlo tridu");

    // Port: paket nese tridu odesilatele.
    let port = Port::new(None);
    let (a, b) = Channel::create();
    b.core().observe_async(&port, 7, Signals::READABLE).unwrap();
    let writer = sched::spawn("zapis", Qos::Interactive, move || {
        let mut m = Message::new(vec![1], Vec::new());
        m.qos = sched::ipc_class();
        a.write(m).unwrap();
    });
    join(&writer, Duration::from_secs(5), "zapis");
    let p = port.pop().expect("port bez paketu");
    assert_eq!(p.key, 7);
    assert_eq!(p.boost, Some(Qos::Interactive));
    // Realtime se pujcuje jako Interactive, zprava od jadra nic nenese.
    assert_eq!(Message::new(Vec::new(), Vec::new()).qos, None);
}

/// Zvednuti plati jen omezenou dobu (`BOOST_WINDOW_NS`).
fn boost_expires() {
    let stop = Arc::new(Event::new());
    let s = stop.clone();
    let t = sched::spawn("spici", Qos::Background, move || wait_reset(&s));
    sched::boost(&t, Qos::Interactive);
    assert_eq!(sched::thread_info(&t).effective, Qos::Interactive as u8);
    // Nizsi trida platne zvednuti nesnizi.
    sched::boost(&t, Qos::Normal);
    assert_eq!(sched::thread_info(&t).effective, Qos::Interactive as u8);
    wait::sleep(Duration::from_nanos(schedcore::class::BOOST_WINDOW_NS + 10 * MS));
    assert_eq!(sched::thread_info(&t).effective, Qos::Background as u8);
    stop.signal();
    join(&t, Duration::from_secs(5), "spici");
}

/// Prijeti: kazde CPU unese nejvys 50 % Realtime. Periodicke Realtime
/// vlakno se pod plnou zatezi Interactive budi vcas.
fn realtime() {
    let n = sched::cpu_count();
    let base = sched::rt_admitted();
    let gate = Arc::new(Event::new());
    // Vlakna s 50 % (5 ms / 10 ms): vejde se jich presne `n`.
    let mut held = Vec::new();
    let mut refused = None;
    for _ in 0..=n {
        let g = gate.clone();
        match sched::spawn_realtime("rt-kapacita", 5 * MS, 10 * MS, move || wait_reset(&g)) {
            Ok(t) => held.push(t),
            Err(e) => {
                refused = Some(e);
                break;
            }
        }
    }
    if base == 0 {
        assert_eq!(held.len(), n, "prijato {} vlaken po 50 % na {n} CPU", held.len());
        assert_eq!(refused, Some(Error::NoResources));
    }
    for _ in &held {
        gate.signal();
        wait::sleep(Duration::from_millis(5));
    }
    for t in &held {
        join(t, Duration::from_secs(5), "rt-kapacita");
    }
    assert_eq!(sched::rt_admitted(), base, "konec vlakna nevratil kapacitu");

    // Perioda 2 ms pod zatezi Interactive na vsech CPU.
    let inter = load(Qos::Interactive, 2 * n);
    let lat = Arc::new(SpinLock::new(Vec::new()));
    let l = lat.clone();
    let audio = sched::spawn_realtime("zvuk", MS, 4 * MS, move || {
        let mut next = arch::now_ns() + 2 * MS;
        for _ in 0..100 {
            wait::sleep(Duration::from_nanos(next.saturating_sub(arch::now_ns())));
            l.lock().push(arch::now_ns().saturating_sub(next));
            busy(300 * US);
            next += 2 * MS;
        }
    })
    .expect("Realtime vlakno neprijato");
    join(&audio, Duration::from_secs(20), "zvuk");
    inter.finish();
    let mut v = core::mem::take(&mut *lat.lock());
    let (median, max) = stats(&mut v);
    log::info!("planovac: zpozdeni Realtime periody pod zatezi: median {} us, max {} us", median / US, max / US);
    assert!(median < 2 * MS, "median zpozdeni periody {median} ns");
}

fn job_cap() {
    let root = Job::new_root();
    let child = Job::new_child(&root).unwrap();
    let grandchild = Job::new_child(&child).unwrap();
    assert_eq!(grandchild.qos_cap(), Qos::Realtime);
    child.set_qos_max(Qos::Normal).unwrap();
    assert_eq!(grandchild.qos_cap(), Qos::Normal);
    // Potomek si nad strop predka nepomuze.
    assert_eq!(grandchild.set_qos_max(Qos::Interactive), Err(Error::AccessDenied));
    grandchild.set_qos_max(Qos::Background).unwrap();
    assert_eq!(grandchild.qos_cap(), Qos::Background);
    assert_eq!(child.qos_cap(), Qos::Normal);
}

/// Ping-pong pres dve udalosti: prumerna doba jednoho prepnuti (pul cesty
/// tam a zpet, vcetne probuzeni a cekani).
fn switch_benchmark() {
    const ROUNDS: u64 = 3000;
    let ping = Event::new();
    let pong = Event::new();
    let (p, q) = (ping.clone(), pong.clone());
    let partner = sched::spawn("pong", Qos::Normal, move || {
        for _ in 0..ROUNDS {
            wait_reset(&p);
            q.signal();
        }
    });
    let start = arch::now_ns();
    for _ in 0..ROUNDS {
        ping.signal();
        wait_reset(&pong);
    }
    let per_switch = (arch::now_ns() - start) / (2 * ROUNDS);
    join(&partner, Duration::from_secs(5), "pong");
    log::info!("planovac: prepnuti kontextu (ping-pong pres udalost): {per_switch} ns");
}
