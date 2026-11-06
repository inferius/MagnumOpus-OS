//! Kanaly v ramci jednoho procesu.

use alloc::vec;
use alloc::vec::Vec;

use libos::{Channel, Error, Event, Signals, Startup, sys, wait_one};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("zprava tam a zpet", roundtrip),
    ("prazdna fronta -> SHOULD_WAIT", empty_queue),
    ("maly buffer -> BUFFER_TOO_SMALL s velikosti", small_buffer),
    ("predani handlu presune handle", handle_transfer),
    ("zavrena druha strana -> PEER_CLOSED", peer_closed),
    ("poslani vlastniho konce -> INVALID_ARGS", send_self),
    (
        "poslani druheho konce do nej samotneho -> INVALID_ARGS",
        send_peer,
    ),
    (
        "fronta kanalu nejvys 4 MiB -> SHOULD_WAIT",
        queue_bytes_limit,
    ),
];

fn roundtrip(_: &Startup) -> TestResult {
    let (a, b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    a.write(b"ahoj", Vec::new()).map_err(|e| fail("zapis", e))?;
    let s = wait_one(b.handle(), Signals::READABLE, None).map_err(|e| fail("cekani", e))?;
    check!(s.contains(Signals::READABLE), "READABLE po zapisu");
    let m = b.try_read().map_err(|e| fail("cteni", e))?;
    check_eq!(&m.data[..], b"ahoj", "data");
    Ok(())
}

fn empty_queue(_: &Startup) -> TestResult {
    let (_a, b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    check_eq!(
        b.try_read().err(),
        Some(Error::ShouldWait),
        "cteni z prazdne fronty"
    );
    Ok(())
}

fn small_buffer(_: &Startup) -> TestResult {
    let (a, b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    a.write(&[7u8; 100], Vec::new())
        .map_err(|e| fail("zapis", e))?;
    let mut data = [0u8; 10];
    let (r, (dlen, hlen)) = sys::channel_read(b.handle().raw(), &mut data, &mut []);
    check_eq!(r, Err(Error::BufferTooSmall), "cteni do maleho bufferu");
    check_eq!((dlen, hlen), (100, 0), "hlasena velikost zpravy");
    // Zprava zustala ve fronte.
    let m = b.try_read().map_err(|e| fail("druhe cteni", e))?;
    check_eq!(m.data, vec![7u8; 100], "data");
    Ok(())
}

fn handle_transfer(_: &Startup) -> TestResult {
    let (a, b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    let e = Event::create().map_err(|e| fail("event", e))?;
    let raw = e.handle().raw();
    a.write(b"h", vec![e.into_handle()])
        .map_err(|e| fail("zapis", e))?;
    check_eq!(
        sys::object_signal(raw, 0, 1),
        Err(Error::BadHandle),
        "puvodni handle po predani"
    );
    let m = b.try_read().map_err(|e| fail("cteni", e))?;
    check_eq!(m.handles.len(), 1, "pocet prijatych handlu");
    let received = Event::from_handle(m.handles.into_iter().next().unwrap());
    received
        .signal()
        .map_err(|e| fail("signal prijateho eventu", e))?;
    Ok(())
}

fn peer_closed(_: &Startup) -> TestResult {
    let (a, b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    a.write(b"posledni", Vec::new())
        .map_err(|e| fail("zapis", e))?;
    drop(a);
    let s = wait_one(b.handle(), Signals::PEER_CLOSED, None).map_err(|e| fail("cekani", e))?;
    check!(s.contains(Signals::PEER_CLOSED), "PEER_CLOSED");
    // Zpravy ve fronte jde dal cist, pak PEER_CLOSED.
    let m = b.try_read().map_err(|e| fail("cteni zbyle zpravy", e))?;
    check_eq!(&m.data[..], b"posledni", "zbyla zprava");
    check_eq!(
        b.try_read().err(),
        Some(Error::PeerClosed),
        "cteni po vyprazdneni"
    );
    check_eq!(
        b.write(b"x", Vec::new()),
        Err(Error::PeerClosed),
        "zapis do zavreneho"
    );
    Ok(())
}

fn send_self(_: &Startup) -> TestResult {
    let (a, _b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    let raw = a.handle().raw();
    check_eq!(
        sys::channel_write(raw, b"x", &[raw]),
        Err(Error::InvalidArgs),
        "poslani sebe sama"
    );
    Ok(())
}

fn send_peer(_: &Startup) -> TestResult {
    // Druhy konec ve vlastni fronte by drzel sam sebe a nikdy se neuvolnil.
    let (a, b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    check_eq!(
        sys::channel_write(a.handle().raw(), b"x", &[b.handle().raw()]),
        Err(Error::InvalidArgs),
        "poslani druheho konce"
    );
    Ok(())
}

fn queue_bytes_limit(_: &Startup) -> TestResult {
    let (a, _b) = Channel::create().map_err(|e| fail("vytvoreni", e))?;
    let msg = vec![0u8; 64 * 1024];
    for i in 0..64 {
        a.write(&msg, Vec::new())
            .map_err(|e| fail(&alloc::format!("zapis {i}"), e))?;
    }
    check_eq!(
        a.write(&msg, Vec::new()),
        Err(Error::ShouldWait),
        "zapis nad 4 MiB"
    );
    Ok(())
}
