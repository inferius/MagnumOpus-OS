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
use crate::sched::{self, Qos, QosLoan, Thread};
use crate::sync::SpinLock;

pub const TESTS: &[(&str, fn())] = &[
    ("qos: latence vstup -> snimek pod zatezi pozadi", input_to_frame_latency),
    ("qos: Background dostane zaruceny podil", background_share),
    ("qos: dedeni tridy pres kanal a port", ipc_inheritance),
    ("qos: zvednuti vyprsi", boost_expires),
    ("qos: pujcka se ping-pongem neprodluzuje", loan_not_renewed_by_ping_pong),
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

/// Cesta "ovladac" (Interactive) -> "inputd" -> "compd" vykresli snimek
/// (200 us); vraci latence `rounds` udalosti.
fn input_path(rounds: usize) -> Vec<u64> {
    let (irq, to_comp, frame) = (Event::new(), Event::new(), Event::new());
    let (i1, c1) = (irq.clone(), to_comp.clone());
    let inputd = sched::spawn("inputd", Qos::Interactive, move || {
        for _ in 0..rounds {
            wait_reset(&i1);
            c1.signal();
        }
    });
    let (c2, f2) = (to_comp.clone(), frame.clone());
    let compd = sched::spawn("compd", Qos::Interactive, move || {
        for _ in 0..rounds {
            wait_reset(&c2);
            busy(200 * US);
            f2.signal();
        }
    });
    let lat = Arc::new(SpinLock::new(Vec::new()));
    let l = lat.clone();
    let driver = sched::spawn("ovladac", Qos::Interactive, move || {
        for _ in 0..rounds {
            wait::sleep(Duration::from_millis(3));
            let t0 = arch::now_ns();
            irq.signal();
            wait_reset(&frame);
            l.lock().push(arch::now_ns() - t0);
        }
    });
    for t in [&driver, &inputd, &compd] {
        join(t, Duration::from_secs(30), "cesta vstupu");
    }
    core::mem::take(&mut *lat.lock())
}

/// Latence vstupu pod plnou zatezi Background a Normal na vsech CPU.
///
/// Pod QEMU TCG zavisi absolutni cas na rychlosti a vytizeni hostitele
/// (soubezne QEMU), proto se porovnava s referencnim behem bez zateze ve
/// stejnem testu a s rezervou na emulaci. Test hlida hrubou chybu (cesta
/// vstupu ceka na kvanta Normal/Background = desitky ms az sekundy), ne
/// presne cislo; cisla se loguji do dokumentace. Maximum se jen loguje.
fn input_to_frame_latency() {
    const ROUNDS: usize = 40;
    let n = sched::cpu_count();
    let mut base = input_path(ROUNDS);
    let (base_median, base_max) = stats(&mut base);
    let bg = load(Qos::Background, 2 * n);
    let normal = load(Qos::Normal, n);
    let mut v = input_path(ROUNDS);
    bg.finish();
    normal.finish();
    let (median, max) = stats(&mut v);
    log::info!(
        "planovac: latence vstup -> snimek ({n} CPU, {} smycek pozadi): median {} us, max {} us (bez zateze {} / {} us)",
        3 * n,
        median / US,
        max / US,
        base_median / US,
        base_max / US
    );
    let limit = 5 * base_median + 15 * MS;
    assert!(median < limit, "median latence {median} ns pod zatezi, bez zateze {base_median} ns");
}

/// Na kazde CPU pripada jedna smycka Interactive a jedna Background.
/// Zaruka je 1/16 (62 promile); pod QEMU TCG meri smycky svuj beh jen
/// priblizne (vytizeny hostitel vlakno vCPU prerusuje), proto test hlida
/// hladoveni (stary planovac: 0 promile) a opacny extrem, ne presnou
/// zaruku. Cislo se loguje.
fn background_share() {
    let n = sched::cpu_count();
    let inter = load(Qos::Interactive, n);
    let bg = load(Qos::Background, n);
    wait::sleep(Duration::from_millis(1500));
    let (b, i) = (bg.finish(), inter.finish());
    let permille = b * 1000 / (b + i).max(1);
    log::info!("planovac: podil Background pod zatezi Interactive: {permille} promile");
    assert!(permille >= 15, "Background hladovi: {permille} promile");
    assert!(permille <= 400, "Background predbiha Interactive: {permille} promile");
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
        assert_eq!(m.qos.map(|l| l.class), Some(Qos::Interactive));
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
    assert_eq!(p.boost.map(|l| l.class), Some(Qos::Interactive));
    // Realtime se pujcuje jako Interactive, zprava od jadra nic nenese.
    assert_eq!(Message::new(Vec::new(), Vec::new()).qos, None);
}

/// Zvednuti plati jen omezenou dobu (`BOOST_WINDOW_NS`).
fn boost_expires() {
    let stop = Arc::new(Event::new());
    let s = stop.clone();
    let t = sched::spawn("spici", Qos::Background, move || wait_reset(&s));
    let until = arch::now_ns() + schedcore::class::BOOST_WINDOW_NS;
    sched::boost(&t, QosLoan { class: Qos::Interactive, until });
    assert_eq!(sched::thread_info(&t).effective, Qos::Interactive as u8);
    // Nizsi trida platne zvednuti nesnizi.
    sched::boost(&t, QosLoan { class: Qos::Normal, until: until + 100 * MS });
    assert_eq!(sched::thread_info(&t).effective, Qos::Interactive as u8);
    wait::sleep(Duration::from_nanos(schedcore::class::BOOST_WINDOW_NS + 10 * MS));
    assert_eq!(sched::thread_info(&t).effective, Qos::Background as u8);
    stop.signal();
    join(&t, Duration::from_secs(5), "spici");
}

/// Dva procesy Normal si po jedne zprave od Interactive posilaji zpravy
/// tam a zpet (preposilaji pujcku). Pujcka se predavanim neprodluzuje:
/// po `BOOST_WINDOW_NS` od zpravy Interactive musi byt oba zpet Normal,
/// i kdyz si porad posilaji.
fn loan_not_renewed_by_ping_pong() {
    let (a_end, b_end) = Channel::create();
    let (start_tx, start_rx) = Channel::create();
    let stop = Arc::new(AtomicBool::new(false));
    let late = Arc::new(AtomicU8::new(0xFF));
    // Prectene zpravy, preposlani se vzdy pujckou odesilatele.
    fn pass(end: &Channel) -> bool {
        match end.read(64, 0) {
            Ok(m) => {
                sched::boost_current(m.qos);
                let mut out = Message::new(vec![0], Vec::new());
                out.qos = sched::ipc_class();
                end.write(out).is_ok()
            }
            Err(_) => false,
        }
    }
    let (s1, l1) = (stop.clone(), late.clone());
    let a = sched::spawn("ping", Qos::Normal, move || {
        // Prvni zprava od Interactive prijde po `start`.
        wait::wait_one(&*start_rx, Signals::READABLE, Some(deadline_after(Duration::from_secs(5)))).unwrap();
        let first = start_rx.read(64, 0).unwrap();
        let t0 = first.qos.expect("zprava bez pujcky").until;
        sched::boost_current(first.qos);
        let mut m = Message::new(vec![0], Vec::new());
        m.qos = sched::ipc_class();
        a_end.write(m).unwrap();
        while !s1.load(Ordering::Relaxed) {
            if wait::wait_one(&*a_end, Signals::READABLE, Some(deadline_after(Duration::from_millis(200)))).is_err() {
                break;
            }
            pass(&a_end);
            // Po konci puvodni pujcky (s rezervou) uz musi byt Normal.
            if arch::now_ns() > t0 + 20 * MS {
                l1.store(sched::thread_info(&sched::current()).effective, Ordering::Relaxed);
            }
        }
    });
    let s2 = stop.clone();
    let b = sched::spawn("pong", Qos::Normal, move || {
        while !s2.load(Ordering::Relaxed) {
            if wait::wait_one(&*b_end, Signals::READABLE, Some(deadline_after(Duration::from_millis(200)))).is_err() {
                break;
            }
            pass(&b_end);
        }
    });
    let sender = sched::spawn("interactive", Qos::Interactive, move || {
        let mut m = Message::new(vec![1], Vec::new());
        m.qos = sched::ipc_class();
        start_tx.write(m).unwrap();
    });
    join(&sender, Duration::from_secs(5), "interactive");
    wait::sleep(Duration::from_nanos(schedcore::class::BOOST_WINDOW_NS + 60 * MS));
    stop.store(true, Ordering::Relaxed);
    join(&a, Duration::from_secs(5), "ping");
    join(&b, Duration::from_secs(5), "pong");
    assert_eq!(
        late.load(Ordering::Relaxed),
        Qos::Normal as u8,
        "pujcka Interactive prezila ping-pong po konci okna"
    );
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
    // Pod TCG zavisi na hostiteli; hruba chyba (Realtime ceka za
    // Interactive) dava zpozdeni v radu kvant a vic.
    assert!(median < 10 * MS, "median zpozdeni periody {median} ns");
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
