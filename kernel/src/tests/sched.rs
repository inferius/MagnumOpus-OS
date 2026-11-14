//! Testy planovace, vlaken a casu.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;

use super::join;
use crate::arch;
use crate::object::wait;
use crate::mm::KERNEL_STACK_SIZE;
use crate::object::event::Event;
use crate::object::job::Job;
use crate::object::Signals;
use crate::object::process::Process;
use crate::sched::{self, Qos};

pub const TESTS: &[(&str, fn())] = &[
    ("cas: sleep 20 ms", sleep_timing),
    ("vlakna: 16 vlaken na vice CPU", threads_on_all_cpus),
    ("vlakna: preempce (vic vlaken nez CPU)", preemption),
    ("vlakna: mrtve vlakno vrati zasobnik jadra a rozpocet jobu", dead_thread_frees_stack),
];

/// Ceka, az CPU, na kterem vlakno skoncilo, dokonci prepnuti pryc z nej
/// (signal TERMINATED prichazi uz pred poslednim `schedule`).
fn wait_stack_released(t: &sched::Thread) {
    let deadline = arch::now_ns() + 5_000_000_000;
    while t.has_kernel_stack() {
        assert!(arch::now_ns() < deadline, "vlakno {} po konci drzi zasobnik jadra", t.name);
        wait::sleep(Duration::from_millis(1));
    }
}

/// Zasobnik jadra (64 KiB) a jeho cast rozpoctu jobu se vraci hned po konci
/// vlakna, i kdyz objekt vlakna (handle) zije dal; objekt se vrati az se
/// zahodi posledni odkaz.
fn dead_thread_frees_stack() {
    let t = sched::spawn("kratke", Qos::Normal, || {});
    join(&t, Duration::from_secs(5), "kratke");
    wait_stack_released(&t);

    // Prvni vlakno drzi proces nazivu (konec posledniho vlakna by uklidil
    // i adresni prostor procesu a cisla rozpoctu by se smichala).
    let job = Job::new_root();
    let p = Process::new("test-zasobnik", job.clone()).unwrap();
    let go = Event::new();
    let holder = sched::create_user_thread(&p, "drzi").unwrap();
    let g = go.clone();
    sched::start_user_thread_in_kernel(&holder, true, move || {
        let _ = wait::wait_one(&*g, Signals::SIGNALED, Some(wait::deadline_after(Duration::from_secs(10))));
    })
    .unwrap();

    let before = job.info().kernel.used;
    let th = sched::create_user_thread(&p, "zasobnik").unwrap();
    let alive = job.info().kernel.used;
    assert!(alive >= before + KERNEL_STACK_SIZE, "vlakno nestrhlo zasobnik: {before} -> {alive}");
    sched::start_user_thread_in_kernel(&th, false, || {}).unwrap();
    join(&th, Duration::from_secs(5), "zasobnik");
    wait_stack_released(&th);
    // Handle (`th`) drzi objekt vlakna: zpet je presne zasobnik.
    assert_eq!(alive - job.info().kernel.used, KERNEL_STACK_SIZE);
    drop(th);
    assert_eq!(job.info().kernel.used, before, "objekt vlakna nevratil rozpocet");

    go.signal();
    join(&holder, Duration::from_secs(5), "drzi");
    wait_stack_released(&holder);
}

fn sleep_timing() {
    let start = arch::now_ns();
    wait::sleep(Duration::from_millis(20));
    let elapsed = arch::now_ns() - start;
    assert!(elapsed >= 20_000_000, "spanek skoncil driv: {elapsed} ns");
    assert!(
        elapsed < 500_000_000,
        "spanek trval prilis dlouho: {elapsed} ns"
    );
}

fn threads_on_all_cpus() {
    const N: usize = 16;
    let done = Arc::new(AtomicUsize::new(0));
    let cpus = Arc::new(AtomicU64::new(0));
    let threads: Vec<_> = (0..N)
        .map(|_| {
            let (done, cpus) = (done.clone(), cpus.clone());
            sched::spawn("worker", Qos::Normal, move || {
                // Aktivni prace 2 ms, aby se vlakna prekryvala a planovac je
                // musel rozlozit (kratka vlakna by stihlo jedine CPU).
                let end = arch::now_ns() + 2_000_000;
                while arch::now_ns() < end {
                    cpus.fetch_or(1 << sched::this_cpu().index, Ordering::Relaxed);
                }
                wait::sleep(Duration::from_millis(1));
                done.fetch_add(1, Ordering::Relaxed);
            })
        })
        .collect();
    for t in &threads {
        join(t, Duration::from_secs(5), "worker");
    }
    assert_eq!(done.load(Ordering::Relaxed), N);
    if sched::cpu_count() > 1 {
        let used = cpus.load(Ordering::Relaxed).count_ones();
        assert!(used > 1, "vsechna vlakna bezela na jednom CPU");
    }
}

fn preemption() {
    // O jedno vlakno vic nez CPU, kazde aktivne ceka na ostatni: projde
    // jen pokud planovac vlakna strida i bez jejich spoluprace.
    let n = sched::cpu_count() + 1;
    let arrived = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..n)
        .map(|_| {
            let arrived = arrived.clone();
            sched::spawn("spinner", Qos::Normal, move || {
                arrived.fetch_add(1, Ordering::SeqCst);
                while arrived.load(Ordering::SeqCst) < n {
                    core::hint::spin_loop();
                }
            })
        })
        .collect();
    for t in &threads {
        join(t, Duration::from_secs(10), "preempce");
    }
}
