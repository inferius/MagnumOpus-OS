//! Port (cekani na libovolne mnozstvi objektu) a nahodna cisla jadra.

use alloc::vec::Vec;

use libos::abi::limits::PROCESS_MAX_PORT_PENDING;
use libos::abi::syscall::{PORT_MAX_PENDING, RANDOM_MAX, random_flags};
use libos::hw::Resource;
use libos::{Channel, Error, Event, Port, Rights, Signals, Startup, sys, time};

use super::{Test, TestResult, devices, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("port: paket po signalu, registrace jednorazova", port_one_shot),
    ("port: objekt uz signalizovany -> paket hned", port_already),
    ("port: 500 objektu najednou (vic nez WAIT_MANY_MAX)", port_many),
    ("port: kanal ke cteni a zavreny kanal", port_channel),
    ("port: neplatne argumenty", port_invalid),
    ("port: zanikly objekt vrati rezervaci (bez uniku)", port_no_leak),
    ("port: kvota registraci na proces pres vsechny porty", port_process_quota),
    ("RANDOM_GET: data, nasazeni, limity", random_get),
    ("RANDOM_ADD: jen s Resource MANAGE", random_add),
    ("virtio-rng: ovladac spusten", virtio_rng_started),
];

fn no_wait() -> Option<u64> {
    Some(0)
}

fn port_one_shot(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    let ev = Event::create().map_err(|e| fail("event", e))?;
    port.wait_async(ev.handle(), 42, Signals::SIGNALED)
        .map_err(|e| fail("wait_async", e))?;
    check_eq!(port.wait(no_wait()).err(), Some(Error::TimedOut), "bez signalu nic");
    ev.signal().map_err(|e| fail("signal", e))?;
    let p = port.wait(no_wait()).map_err(|e| fail("port_wait", e))?;
    check_eq!(p.key, 42, "klic");
    check!(Signals(p.signals).contains(Signals::SIGNALED), "signaly v paketu");
    // Jednorazove: dalsi paket az po nove registraci.
    ev.reset().map_err(|e| fail("reset", e))?;
    ev.signal().map_err(|e| fail("signal", e))?;
    check_eq!(port.wait(no_wait()).err(), Some(Error::TimedOut), "bez nove registrace");
    // Cekani s terminem.
    let start = time::now();
    check_eq!(
        port.wait(Some(time::deadline_after(core::time::Duration::from_millis(20)))).err(),
        Some(Error::TimedOut),
        "termin"
    );
    check!(time::now() - start >= 15_000_000, "cekalo se do terminu");
    Ok(())
}

fn port_already(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    let ev = Event::create().map_err(|e| fail("event", e))?;
    ev.signal().map_err(|e| fail("signal", e))?;
    port.wait_async(ev.handle(), 7, Signals::SIGNALED)
        .map_err(|e| fail("wait_async", e))?;
    check_eq!(port.wait(no_wait()).map(|p| p.key).ok(), Some(7), "paket hned");
    Ok(())
}

fn port_many(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    let mut events = Vec::new();
    for i in 0..500u64 {
        let ev = Event::create().map_err(|e| fail("event", e))?;
        port.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("wait_async", e))?;
        events.push(ev);
    }
    // Signalizovat od konce, pakety prijdou v poradi signalu.
    for ev in events.iter().rev() {
        ev.signal().map_err(|e| fail("signal", e))?;
    }
    let mut seen = Vec::new();
    while let Ok(p) = port.wait(no_wait()) {
        seen.push(p.key);
    }
    check_eq!(seen.len(), 500, "pocet paketu");
    check!(seen.iter().copied().eq((0..500u64).rev()), "poradi paketu = poradi signalu");
    // Port hlasi READABLE, dokud v nem neco je.
    events[0].reset().map_err(|e| fail("reset", e))?;
    port.wait_async(events[0].handle(), 1, Signals::SIGNALED)
        .map_err(|e| fail("wait_async", e))?;
    events[0].signal().map_err(|e| fail("signal", e))?;
    let s = libos::wait_one(port.handle(), Signals::READABLE, no_wait()).map_err(|e| fail("wait_one portu", e))?;
    check!(s.contains(Signals::READABLE), "port READABLE");
    let _ = port.wait(no_wait());
    check_eq!(
        libos::wait_one(port.handle(), Signals::READABLE, no_wait()).err(),
        Some(Error::TimedOut),
        "prazdny port bez READABLE"
    );
    Ok(())
}

fn port_channel(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    let (a, b) = Channel::create().map_err(|e| fail("kanal", e))?;
    port.wait_async(b.handle(), 1, Signals::READABLE)
        .map_err(|e| fail("wait_async ke cteni", e))?;
    port.wait_async(b.handle(), 2, Signals::PEER_CLOSED)
        .map_err(|e| fail("wait_async zavreni", e))?;
    a.write(b"x", Vec::new()).map_err(|e| fail("zapis", e))?;
    check_eq!(port.wait(no_wait()).map(|p| p.key).ok(), Some(1), "ke cteni");
    drop(a);
    let p = port.wait(no_wait()).map_err(|e| fail("zavreni", e))?;
    check_eq!(p.key, 2, "klic zavreni");
    check!(Signals(p.signals).contains(Signals::PEER_CLOSED), "PEER_CLOSED v paketu");
    Ok(())
}

fn port_invalid(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    let other = Port::create().map_err(|e| fail("port", e))?;
    let ev = Event::create().map_err(|e| fail("event", e))?;
    check_eq!(
        port.wait_async(other.handle(), 1, Signals::READABLE).err(),
        Some(Error::NotSupported),
        "registrace na port"
    );
    check_eq!(
        port.wait_async(ev.handle(), 1, Signals::NONE).err(),
        Some(Error::InvalidArgs),
        "prazdna maska"
    );
    // Prava se overuji drive nez typ: event nema WRITE, kanal ano.
    check_eq!(
        sys::object_wait_async(ev.handle().raw(), ev.handle().raw(), 1, Signals::SIGNALED.0).err(),
        Some(Error::AccessDenied),
        "event misto portu"
    );
    let (ch, _peer) = Channel::create().map_err(|e| fail("kanal", e))?;
    check_eq!(
        sys::object_wait_async(ev.handle().raw(), ch.handle().raw(), 1, Signals::SIGNALED.0).err(),
        Some(Error::WrongType),
        "kanal misto portu"
    );
    // Port bez prava WRITE registraci nedovoli.
    let ro = port
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::READ))
        .map_err(|e| fail("duplikace", e))?;
    check_eq!(
        sys::object_wait_async(ev.handle().raw(), ro.raw(), 1, Signals::SIGNALED.0).err(),
        Some(Error::AccessDenied),
        "port bez WRITE"
    );
    check_eq!(sys::port_wait(ev.handle().raw(), 0).err(), Some(Error::AccessDenied), "PORT_WAIT na event (bez READ)");
    check_eq!(sys::port_wait(ch.handle().raw(), 0).err(), Some(Error::WrongType), "PORT_WAIT na kanal");
    Ok(())
}

fn port_no_leak(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    // Vic registraci, nez port pojme: kazda patri objektu, ktery hned
    // zanikne. Kdyby rezervace unikala, skoncilo by to NO_RESOURCES.
    for i in 0..(PORT_MAX_PENDING as u64 + 100) {
        let ev = Event::create().map_err(|e| fail("event", e))?;
        port.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("wait_async", e))?;
    }
    check_eq!(port.wait(no_wait()).err(), Some(Error::TimedOut), "zadne pakety");
    Ok(())
}

fn port_process_quota(_: &Startup) -> TestResult {
    // Stejny event muze mit libovolne registraci; kvota je spolecna pro
    // vsechny porty procesu.
    let ev = Event::create().map_err(|e| fail("event", e))?;
    let a = Port::create().map_err(|e| fail("port A", e))?;
    let b = Port::create().map_err(|e| fail("port B", e))?;
    let half = PROCESS_MAX_PORT_PENDING / 2;
    for i in 0..half as u64 {
        a.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("registrace A", e))?;
    }
    for i in 0..(PROCESS_MAX_PORT_PENDING - half) as u64 {
        b.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("registrace B", e))?;
    }
    check_eq!(
        b.wait_async(ev.handle(), 0, Signals::SIGNALED).err(),
        Some(Error::NoResources),
        "kvota procesu vycerpana"
    );
    // Zavreny port vrati svou cast kvoty.
    drop(a);
    b.wait_async(ev.handle(), 1, Signals::SIGNALED)
        .map_err(|e| fail("registrace po zavreni A", e))?;
    // Vyzvednuty paket taky: signal doruci vsechny registrace B.
    ev.signal().map_err(|e| fail("signal", e))?;
    let mut n = 0;
    while b.wait(no_wait()).is_ok() {
        n += 1;
    }
    check_eq!(n, PROCESS_MAX_PORT_PENDING - half + 1, "pakety B");
    let c = Port::create().map_err(|e| fail("port C", e))?;
    ev.reset().map_err(|e| fail("reset", e))?;
    for i in 0..half as u64 {
        c.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("registrace C po vyzvednuti", e))?;
    }
    Ok(())
}

fn random_get(_: &Startup) -> TestResult {
    let mut a = [0u8; 64];
    let mut b = [0u8; 64];
    let seeded = sys::random_get(&mut a, 0).map_err(|e| fail("RANDOM_GET", e))?;
    sys::random_get(&mut b, 0).map_err(|e| fail("RANDOM_GET", e))?;
    check!(seeded, "generator nasazeny (QEMU: RDRAND/RDSEED a virtio-rng)");
    check!(a != b, "dve volani, ruzna data");
    check!(a != [0; 64], "data nejsou nulova");
    let mut big = alloc::vec![0u8; RANDOM_MAX + 1];
    check_eq!(sys::random_get(&mut big, 0).err(), Some(Error::InvalidArgs), "nad RANDOM_MAX");
    check_eq!(sys::random_get(&mut a, 0x80).err(), Some(Error::InvalidArgs), "neznamy priznak");
    sys::random_get(&mut a, random_flags::REQUIRE_SEEDED).map_err(|e| fail("REQUIRE_SEEDED", e))?;
    // Obal v libos zvladne i vic nez RANDOM_MAX.
    libos::random::fill_seeded(&mut big).map_err(|e| fail("fill_seeded", e))?;
    check!(big.iter().any(|&x| x != 0), "velky buffer naplnen");
    Ok(())
}

fn random_add(s: &Startup) -> TestResult {
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    libos::random::add(root, b"data od init", 0).map_err(|e| fail("RANDOM_ADD s MANAGE", e))?;
    let h = root
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::SET_CLOCK))
        .map_err(|e| fail("duplikace", e))?;
    let no_manage = Resource::from_handle(h);
    check_eq!(
        libos::random::add(&no_manage, b"x", 8).err(),
        Some(Error::AccessDenied),
        "Resource bez MANAGE"
    );
    let ev = Event::create().map_err(|e| fail("event", e))?;
    check_eq!(
        sys::random_add(ev.handle().raw(), b"x", 8).err(),
        Some(Error::AccessDenied),
        "event"
    );
    Ok(())
}

fn virtio_rng_started(_: &Startup) -> TestResult {
    let d = devices().ok_or("zarizeni nebezi")?;
    check!(!d._rngs.is_empty(), "devmgr nenasel virtio-rng (QEMU -device virtio-rng-pci)");
    Ok(())
}
