//! Port (cekani na libovolne mnozstvi objektu) a nahodna cisla jadra.

use alloc::format;
use alloc::vec::Vec;

use libos::abi::syscall::{RANDOM_MAX, random_flags};
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
    ("event: uzivatelske bity USER_0..7 a pravo SIGNAL", event_user_bits),
    ("port: zanikly objekt vrati rezervaci (bez uniku)", port_no_leak),
    ("port: 300 000 registraci (bez pevneho limitu)", port_unbounded),
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

/// Init sdili korenovy job se sluzbami, ktere bezi soubezne: spotreba
/// jadra se muze mirne menit i bez testu. Unik registraci by byl o rady
/// vetsi (70 000 x 64 B).
fn near(a: u64, b: u64) -> bool {
    a.abs_diff(b) < 256 * 1024
}

fn port_no_leak(_: &Startup) -> TestResult {
    let port = Port::create().map_err(|e| fail("port", e))?;
    let before = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    // Registrace objektu, ktere hned zaniknou: rezervace se musi vratit
    // (rozpocet jadra jobu se vrati na puvodni hodnotu).
    for i in 0..70_000u64 {
        let ev = Event::create().map_err(|e| fail("event", e))?;
        port.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("wait_async", e))?;
    }
    check_eq!(port.wait(no_wait()).err(), Some(Error::TimedOut), "zadne pakety");
    let after = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check!(
        near(after.kernel.used, before.kernel.used),
        alloc::format!("rozpocet jadra po zaniku objektu: {} -> {}", before.kernel.used, after.kernel.used)
    );
    Ok(())
}

fn port_unbounded(_: &Startup) -> TestResult {
    // Drive pevny limit 65 536 na port i proces; ted jen rozpocet jobu.
    // 300 000 paketu je i vic, nez by se veslo do jedne souvisle fronty.
    // Registrace jsou rozlozene na 300 eventu (jako sluzba s mnoha kanaly).
    let n = 300_000u64;
    let per_event = 1000;
    let events: Vec<Event> = (0..n / per_event)
        .map(|_| Event::create())
        .collect::<Result<_, _>>()
        .map_err(|e| fail("event", e))?;
    let a = Port::create().map_err(|e| fail("port A", e))?;
    let b = Port::create().map_err(|e| fail("port B", e))?;
    let before = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    for i in 0..n {
        let ev = &events[(i / per_event) as usize];
        a.wait_async(ev.handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("registrace A", e))?;
    }
    for i in 0..1000 {
        b.wait_async(events[0].handle(), i, Signals::SIGNALED)
            .map_err(|e| fail("registrace B", e))?;
    }
    let during = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check!(
        during.kernel.used + 256 * 1024 >= before.kernel.used + n * 64,
        "registrace se nestrhavaji z rozpoctu"
    );
    for ev in &events {
        ev.signal().map_err(|e| fail("signal", e))?;
    }
    let mut count = 0u64;
    let mut ordered = true;
    while let Ok(p) = a.wait(no_wait()) {
        ordered &= p.key == count;
        count += 1;
    }
    check_eq!(count, n, "pakety A");
    check!(ordered, "poradi paketu = poradi signalu");
    // Zavreny port vrati nevyzvednute pakety.
    drop(b);
    let after = libos::job::current_info().map_err(|e| fail("JOB_INFO", e))?;
    check!(
        near(after.kernel.used, before.kernel.used),
        alloc::format!("rozpocet po vyzvednuti a zavreni: {} -> {}", before.kernel.used, after.kernel.used)
    );
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

fn event_user_bits(_: &Startup) -> TestResult {
    let ev = Event::create().map_err(|e| fail("event", e))?;
    let user = Signals::USER_0 | Signals::USER_3 | Signals::USER_7;
    ev.update(Signals::NONE, user).map_err(|e| fail("nastaveni USER", e))?;
    let got = libos::wait_one(ev.handle(), Signals::USER_3, no_wait()).map_err(|e| fail("cekani na USER_3", e))?;
    check!(got.contains(user), format!("signaly {:#x}", got.0));
    check!(!got.contains(Signals::SIGNALED), "SIGNALED se nenastavil");
    // Port: registrace na uzivatelsky bit.
    let port = Port::create().map_err(|e| fail("port", e))?;
    port.wait_async(ev.handle(), 7, Signals::USER_1).map_err(|e| fail("wait_async", e))?;
    check_eq!(port.wait(no_wait()).err(), Some(Error::TimedOut), "USER_1 neni nastaveny");
    ev.update(Signals::USER_0, Signals::USER_1).map_err(|e| fail("zmena bitu", e))?;
    let p = port.wait(no_wait()).map_err(|e| fail("port_wait", e))?;
    check_eq!(p.key, 7, "klic");
    check!(Signals(p.signals).contains(Signals::USER_1) && !Signals(p.signals).contains(Signals::USER_0), format!("signaly v paketu {:#x}", p.signals));
    // Signaly, ktere ridi jadro, uzivatel menit nesmi.
    for bad in [Signals::READABLE, Signals::PEER_CLOSED, Signals::TERMINATED, Signals(1 << 12)] {
        check_eq!(ev.update(Signals::NONE, bad).err(), Some(Error::InvalidArgs), format!("bit {:#x}", bad.0));
    }
    // Klient s handlem jen pro cekani bity nezmeni (sluzba drzi SIGNAL).
    let dup = ev.handle().duplicate(Some(Rights::WAIT)).map_err(|e| fail("duplikat", e))?;
    let client = Event::from_handle(dup);
    check_eq!(client.update(Signals::USER_ALL, Signals::NONE).err(), Some(Error::AccessDenied), "zmena bez SIGNAL");
    let seen = libos::wait_one(client.handle(), Signals::USER_1, no_wait()).map_err(|e| fail("cekani klienta", e))?;
    check!(seen.contains(Signals::USER_1), "klient vidi bity");
    Ok(())
}
