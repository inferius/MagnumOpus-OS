//! Rychle testy zakladnich subsystemu, spoustene pri startu.
//!
//! Kazdy test pri selhani panikuje. S feature `qemu-test` kernel po testech
//! ukonci QEMU s vysledkem (pouziva `cargo xtask test`).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use core::time::Duration;

use crate::arch;
use crate::kprintln;
use crate::mm::{self, PhysAddr, VirtAddr, phys};
use crate::object::event::Event;
use crate::object::process::Process;
use crate::object::wait::{self, deadline_after};
use crate::object::{Error, Handle, HandleTable, KernelObject, Rights, Signals};
use crate::sched::{self, Qos, Thread};

pub fn run() {
    let tests: &[(&str, fn())] = &[
        ("vyjimka breakpoint se vrati", breakpoint),
        ("buddy: alokace a uvolneni", buddy),
        ("heap: Box, Vec, String", heap_small),
        ("heap: velka alokace", heap_large),
        ("strankovani: identita od loaderu je pryc", no_identity_map),
        ("strankovani: prima mapa", direct_map),
        ("cas: sleep 20 ms", sleep_timing),
        ("vlakna: 16 vlaken na vice CPU", threads_on_all_cpus),
        ("vlakna: preempce (vic vlaken nez CPU)", preemption),
        ("objekty: event a wait_many, timeout", events_wait_many),
        ("objekty: ping-pong mezi vlakny 2000x", ping_pong),
        ("handly: prava, typy, generace", handles),
        ("procesy: adresni prostor, uvolneni", processes),
    ];
    for (name, test) in tests {
        test();
        kprintln!("  [ok] {name}");
    }
}

fn breakpoint() {
    arch::breakpoint();
}

fn buddy() {
    let (free_before, _) = phys::stats();
    let a = phys::alloc_pages(0).expect("alokace 1 stranky");
    let b = phys::alloc_pages(3).expect("alokace 8 stranek");
    assert_eq!(
        b.as_u64() % (8 * 4096),
        0,
        "blok neni zarovnany na svou velikost"
    );
    assert_ne!(a, b);
    assert_eq!(phys::stats().0, free_before - 9);
    phys::free_pages(a, 0);
    phys::free_pages(b, 3);
    assert_eq!(
        phys::stats().0,
        free_before,
        "uvolneni nevratilo vsechny stranky"
    );
}

fn heap_small() {
    let boxed = Box::new(0xDEAD_BEEF_u64);
    assert_eq!(*boxed, 0xDEAD_BEEF);
    let v: Vec<u32> = (0..1000).collect();
    assert_eq!(v.iter().sum::<u32>(), 499_500);
    let mut s = String::from("Příliš ");
    s.push_str("žluťoučký kůň");
    assert_eq!(s.chars().count(), 20);
}

fn heap_large() {
    let (free_before, _) = phys::stats();
    let v = alloc::vec![0xA5u8; 1024 * 1024];
    assert!(v.iter().all(|&x| x == 0xA5));
    drop(v);
    assert_eq!(phys::stats().0, free_before, "velka alokace se neuvolnila");
}

fn no_identity_map() {
    // Kod jadra lezi fyzicky nekde v RAM; na jeho fyzicke adrese uz nesmi
    // byt nic namapovano (identita byla jen pro prechod z loaderu).
    let code = mm::translate(VirtAddr::new(no_identity_map as *const () as u64))
        .expect("kod neni namapovany");
    assert!(mm::translate(VirtAddr::new(code.as_u64())).is_none());
}

fn direct_map() {
    let frame = phys::alloc_frame_zeroed().expect("alokace stranky");
    let virt = frame.to_virt();
    assert_eq!(mm::translate(virt), Some(frame));
    // SAFETY: prave alokovana stranka, pristup pres primou mapu.
    unsafe { virt.as_mut_ptr::<u64>().write(42) };
    assert_eq!(PhysAddr::from_direct_map(virt), frame);
    phys::free_pages(frame, 0);
}

fn join(t: &Arc<Thread>, timeout: Duration, what: &str) {
    wait::wait_one(&**t, Signals::TERMINATED, Some(deadline_after(timeout)))
        .unwrap_or_else(|e| panic!("{what}: vlakno {} neskoncilo ({e:?})", t.name));
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

fn events_wait_many() {
    let e1 = Event::new();
    let e2 = Event::new();
    let e2c = e2.clone();
    let signaler = sched::spawn("signaler", Qos::Normal, move || {
        wait::sleep(Duration::from_millis(10));
        e2c.signal();
    });
    let (index, signals) = wait::wait_many(
        &[(&*e1, Signals::SIGNALED), (&*e2, Signals::SIGNALED)],
        Some(deadline_after(Duration::from_secs(2))),
    )
    .expect("wait_many vyprsel");
    assert_eq!(index, 1);
    assert!(signals.intersects(Signals::SIGNALED));
    e2.reset();
    assert_eq!(e2.core().signals(), Signals::NONE);

    let r = wait::wait_one(
        &*e1,
        Signals::SIGNALED,
        Some(deadline_after(Duration::from_millis(10))),
    );
    assert_eq!(r, Err(Error::TimedOut));
    join(&signaler, Duration::from_secs(1), "signaler");
}

fn ping_pong() {
    const ROUNDS: usize = 2000;
    let ping = Event::new();
    let pong = Event::new();
    let (p, q) = (ping.clone(), pong.clone());
    let partner = sched::spawn("pong", Qos::Normal, move || {
        for _ in 0..ROUNDS {
            wait::wait_one(&*p, Signals::SIGNALED, None).unwrap();
            p.reset();
            q.signal();
        }
    });
    for i in 0..ROUNDS {
        ping.signal();
        wait::wait_one(
            &*pong,
            Signals::SIGNALED,
            Some(deadline_after(Duration::from_secs(5))),
        )
        .unwrap_or_else(|_| panic!("ping-pong se zasekl v kole {i}"));
        pong.reset();
    }
    join(&partner, Duration::from_secs(1), "pong");
}

fn handles() {
    let mut table = HandleTable::new();
    let event = Event::new();
    let h = table.insert(event.clone(), event.default_rights()).unwrap();
    assert!(table.get_as::<Event>(h, Rights::SIGNAL).is_ok());

    // Prava lze jen ubirat.
    let weak = table
        .duplicate(h, Rights::WAIT | Rights::DUPLICATE)
        .unwrap();
    assert_eq!(
        table.get(weak, Rights::SIGNAL).err(),
        Some(Error::AccessDenied)
    );
    assert_eq!(
        table.duplicate(weak, Rights::WAIT | Rights::SIGNAL).err(),
        Some(Error::AccessDenied)
    );
    let no_dup = table.duplicate(h, Rights::WAIT).unwrap();
    assert_eq!(
        table.duplicate(no_dup, Rights::WAIT).err(),
        Some(Error::AccessDenied)
    );

    // Kontrola typu.
    assert_eq!(
        table.get_as::<Process>(h, Rights::NONE).err(),
        Some(Error::WrongType)
    );

    // Zavreny handle je neplatny i po znovupouziti slotu (generace).
    table.close(h).unwrap();
    assert_eq!(table.get(h, Rights::NONE).err(), Some(Error::BadHandle));
    let h2 = table.insert(event.clone(), Rights::WAIT).unwrap();
    assert_ne!(h, h2);
    assert_eq!(table.get(h, Rights::NONE).err(), Some(Error::BadHandle));
    assert_eq!(
        table.get(Handle::INVALID, Rights::NONE).err(),
        Some(Error::BadHandle)
    );
    assert_eq!(table.close(h).err(), Some(Error::BadHandle));
}

fn processes() {
    let p = Process::new("test").expect("vytvoreni procesu");
    // Jadro je ve vsech adresnich prostorech stejne, spodni polovina prazdna.
    let kernel_addr = VirtAddr::new(processes as *const () as u64);
    assert_eq!(
        p.address_space().translate(kernel_addr),
        mm::translate(kernel_addr)
    );
    assert!(
        p.address_space()
            .translate(VirtAddr::new(0x40_0000))
            .is_none()
    );

    let event = Event::new();
    let h = p.handles().insert(event, Rights::WAIT).unwrap();
    assert!(p.handles().get_as::<Event>(h, Rights::WAIT).is_ok());
    assert!(p.id > 0 && p.name == "test");
    drop(p);

    // Opakovane vytvoreni a zruseni procesu nesmi ztracet pamet.
    let before = phys::stats().0;
    for _ in 0..100 {
        drop(Process::new("tmp").unwrap());
    }
    let after = phys::stats().0;
    assert!(
        after + 2 >= before,
        "procesy ztraci pamet: {before} -> {after} volnych stranek"
    );
}
