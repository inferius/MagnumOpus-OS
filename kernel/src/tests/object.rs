//! Testy objektu, cekani, handlu a procesu.

use core::time::Duration;

use super::join;
use crate::mm::{self, VirtAddr, phys};
use crate::object::event::Event;
use crate::object::job::{Charge, Job, Pool};
use crate::object::process::Process;
use crate::object::wait::{self, deadline_after};
use crate::object::{Error, Handle, HandleTable, KernelObject, Rights, Signals};
use crate::sched::{self, Qos};

pub const TESTS: &[(&str, fn())] = &[
    ("event a wait_many, timeout", events_wait_many),
    ("ping-pong mezi vlakny 2000x", ping_pong),
    ("handly: prava, typy, generace", handles),
    ("procesy: adresni prostor, uvolneni", processes),
    (
        "handly: vycerpane generace se nepouziji",
        handle_generation_exhausted,
    ),
    (
        "casovace: zrusene cekani neopousti zaznam",
        timers_cancelled,
    ),
    ("joby: limity predku, vraceni strzeni", jobs),
    ("handly: tabulka nad 4 MiB slotu", handles_many),
];

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
    let mut table = HandleTable::new(Charge::none());
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
    let p = Process::new("test", Job::new_root()).expect("vytvoreni procesu");
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
        drop(Process::new("tmp", Job::new_root()).unwrap());
    }
    let after = phys::stats().0;
    assert!(
        after + 2 >= before,
        "procesy ztraci pamet: {before} -> {after} volnych stranek"
    );
}

fn handle_generation_exhausted() {
    let mut table = HandleTable::new(Charge::none());
    let event = Event::new();
    let first = table.insert(event.clone(), Rights::WAIT).unwrap();
    table.close(first).unwrap();
    // Slot se znovu pouzije s vyssi generaci, az generace dojdou, vyradi se.
    // Puvodni handle nesmi nikdy znovu platit.
    for _ in 0..5000 {
        let h = table.insert(event.clone(), Rights::WAIT).unwrap();
        assert_ne!(h, first, "stary handle po preteceni generace znovu plati");
        table.close(h).unwrap();
        assert_eq!(table.get(first, Rights::NONE).err(), Some(Error::BadHandle));
    }
}

fn timers_cancelled() {
    // Cekani s dlouhym timeoutem, ktere skonci driv, nesmi nechat zaznam
    // casovace (drzel by vlakno a zbytecne budil CPU).
    let before = sched::pending_timers();
    let event = Event::new();
    for _ in 0..50 {
        let e = event.clone();
        let t = sched::spawn("signal", Qos::Normal, move || e.signal());
        wait::wait_one(
            &*event,
            Signals::SIGNALED,
            Some(deadline_after(Duration::from_secs(3600))),
        )
        .expect("event nedosel");
        event.reset();
        join(&t, Duration::from_secs(1), "signal");
    }
    assert_eq!(
        sched::pending_timers(),
        before,
        "zrusene casovace zustaly ve fronte"
    );
}

fn jobs() {
    let root = Job::new_root();
    let a = Job::new_child(&root).unwrap();
    let b = Job::new_child(&a).unwrap();
    let base = root.info().kernel.used;
    a.set_limit(Pool::User, 1000);

    // Strzeni v potomkovi se pocita i predkum.
    let c1 = b.charge(Pool::User, 600).unwrap();
    assert_eq!(a.info().user.used, 600);
    assert_eq!(root.info().user.used, 600);
    assert_eq!(b.info().user.available, 400);

    // Limit predka plati i pres vlastni limit potomka.
    b.set_limit(Pool::User, 1 << 40);
    assert_eq!(b.charge(Pool::User, 500).err(), Some(Error::NoResources));
    // Neuspech nic nenechal strzene (ani u potomka na ceste).
    assert_eq!(b.info().user.used, 600);
    assert_eq!(root.info().user.used, 600);

    // Rozpocty jsou oddelene.
    let k = b.charge(Pool::Kernel, 5000).unwrap();
    drop(c1);
    assert_eq!(root.info().user.used, 0);
    drop(k);
    assert_eq!(root.info().kernel.used, base);

    // Grow/shrink drzitele.
    let mut c = a.charge(Pool::User, 100).unwrap();
    c.grow(800).unwrap();
    assert_eq!(c.grow(200).err(), Some(Error::NoResources));
    c.shrink(500);
    assert_eq!(a.info().user.used, 400);
    drop(c);
    assert_eq!(a.info().user.used, 0);

    // Limit pod spotrebou: dalsi strzeni selze, uvolneni projde.
    let c = a.charge(Pool::User, 900).unwrap();
    a.set_limit(Pool::User, 100);
    assert_eq!(a.charge(Pool::User, 1).err(), Some(Error::NoResources));
    drop(c);
    assert!(a.charge(Pool::User, 100).is_ok());
}

fn handles_many() {
    // Vic slotu, nez by se veslo do jednoho bloku heapu (4 MiB): drive by
    // tabulka v obycejnem `Vec` pri rustu selhala.
    let job = Job::new_root();
    let mut table = HandleTable::new(job.charge(Pool::Kernel, 0).unwrap());
    let event = Event::new();
    let n = 200_000;
    let mut last = Handle::INVALID;
    for _ in 0..n {
        last = table.insert(event.clone(), Rights::WAIT).unwrap();
    }
    assert_eq!(table.slot_count(), n);
    assert!(table.get(last, Rights::WAIT).is_ok());
    assert!(job.info().kernel.used >= n as u64 * 16);
    drop(table);
    assert_eq!(job.info().kernel.used, 0);
}
