//! Ploty (`FENCE_*`) mezi procesy: producent a spotrebitel v jinem procesu
//! (`echo`), casova osa ve `FENCE_WAIT` i v portu, trvala chyba, opusteni
//! po padu producenta a pri zahozene zprave, prava a neplatne argumenty.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::fence::{self, Fence};
use libos::initfs::Initfs;
use libos::{Channel, Error, Event, Port, Rights, Signals, Startup, spawn, sys, time, wait_one};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("plot: producent v jinem procesu, osa ve FENCE_WAIT a v portu", producer_in_child),
    ("plot: spotrebitel v jinem procesu (jen WAIT)", consumer_in_child),
    ("plot: trvala chyba FENCE_FAILED", failed),
    ("plot: opusteni po kill producenta (port i cekani)", abandoned_by_kill),
    ("plot: producent ve zprave kanalu plot neopusti, zahozena zprava ano", abandoned_in_flight),
    ("plot: vsechny polozky, index pri chybe, termin", wait_all_and_index),
    ("plot: prava a neplatne argumenty", rights_and_args),
];

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(5)))
}

fn spawn_echo(s: &Startup) -> Result<(libos::Process, Channel), String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("echo").ok_or("echo neni v initfs")?;
    spawn("echo", image, Vec::new()).map_err(|e| fail("spusteni echo", e))
}

fn ask(ch: &Channel, msg: &str, handles: Vec<libos::Handle>) -> Result<Vec<u8>, String> {
    ch.write(msg.as_bytes(), handles).map_err(|e| fail("zapis", e))?;
    Ok(ch.read(deadline()).map_err(|e| fail("odpoved", e))?.data)
}

/// Plot, jehoz jediny handle s `SIGNAL` drzi `echo`; vrati handle
/// spotrebitele.
fn give_producer(ch: &Channel) -> Result<Fence, String> {
    let f = Fence::create(0).map_err(|e| fail("FENCE_CREATE", e))?;
    let waiter = f.for_waiter().map_err(|e| fail("duplikat WAIT", e))?;
    check_eq!(ask(ch, "fence-hold", vec![f.into_handle()])?, b"ok".to_vec(), "fence-hold");
    Ok(waiter)
}

fn producer_in_child(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    let f = give_producer(&ch)?;
    check_eq!(f.query().map(|i| (i.value, i.status)), Ok((0, 0)), "novy plot");
    // Binarni pohled: SIGNALED az po hodnote 1.
    check_eq!(
        wait_one(f.handle(), Signals::SIGNALED, Some(0)).err(),
        Some(Error::TimedOut),
        "SIGNALED pred signalem"
    );
    check_eq!(ask(&ch, "fence-signal:1", Vec::new())?, b"ok".to_vec(), "signal 1");
    f.wait(1, deadline()).map_err(|e| fail("cekani na 1", e))?;
    wait_one(f.handle(), Signals::SIGNALED, Some(0)).map_err(|e| fail("SIGNALED", e))?;
    // Osa: cekani na 3 skonci signalem 5.
    check_eq!(ask(&ch, "fence-signal:5", Vec::new())?, b"ok".to_vec(), "signal 5");
    check_eq!(fence::wait_any(&[(&f, 3)], deadline()), Ok(0), "cekani na 3");
    check_eq!(fence::wait_any(&[(&f, 6)], Some(0)), Err((Error::TimedOut, None)), "6 jeste ne");
    // Port: registrace na 7, paket az po signalu 7.
    let port = Port::create().map_err(|e| fail("port", e))?;
    f.wait_async(7, &port, 77).map_err(|e| fail("FENCE_WAIT_ASYNC", e))?;
    f.wait_async(5, &port, 55).map_err(|e| fail("FENCE_WAIT_ASYNC dosazena", e))?;
    let pk = port.wait(Some(0)).map_err(|e| fail("paket dosazene hodnoty", e))?;
    check_eq!(pk.key, 55, "dosazena hodnota hned");
    check_eq!(port.wait(Some(0)).err(), Some(Error::TimedOut), "7 jeste ne");
    check_eq!(ask(&ch, "fence-signal:7", Vec::new())?, b"ok".to_vec(), "signal 7");
    let pk = port.wait(deadline()).map_err(|e| fail("paket 7", e))?;
    check_eq!(pk.key, 77, "klic paketu");
    check!(Signals(pk.signals).contains(Signals::SIGNALED), "signaly paketu");
    check_eq!(f.query().map(|i| i.value), Ok(7), "hodnota");
    // Zpet nejde.
    check_eq!(ask(&ch, "fence-signal:6", Vec::new())?, b"err:INVALID_ARGS".to_vec(), "zpet");
    drop(ch);
    check_eq!(p.wait_exit(deadline()), Ok(0), "konec echo");
    Ok(())
}

fn consumer_in_child(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    let f = Fence::create(0).map_err(|e| fail("FENCE_CREATE", e))?;
    let w = f.for_waiter().map_err(|e| fail("dup", e))?;
    ch.write(b"fence-wait:4", vec![w.into_handle()]).map_err(|e| fail("zapis", e))?;
    time::sleep(Duration::from_millis(20));
    check_eq!(ch.read(Some(0)).err(), Some(Error::TimedOut), "echo ceka");
    f.signal(4).map_err(|e| fail("signal", e))?;
    let r = ch.read(deadline()).map_err(|e| fail("odpoved", e))?;
    check_eq!(r.data, b"ok".to_vec(), "spotrebitel docekal");
    // Spotrebitel bez SIGNAL nemuze posouvat (echo drzi jen WAIT kopii).
    let w = f.for_waiter().map_err(|e| fail("dup", e))?;
    check_eq!(ask(&ch, "fence-hold", vec![w.into_handle()])?, b"ok".to_vec(), "hold");
    check_eq!(ask(&ch, "fence-signal:9", Vec::new())?, b"err:ACCESS_DENIED".to_vec(), "WAIT nesignalizuje");
    drop(ch);
    p.wait_exit(deadline()).map_err(|e| fail("konec", e))?;
    // Zanik WAIT handlu plot neopusti (producent je tady).
    check_eq!(f.query().map(|i| i.status), Ok(0), "plot neni opusteny");
    Ok(())
}

fn failed(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    let f = give_producer(&ch)?;
    check_eq!(ask(&ch, "fence-signal:2", Vec::new())?, b"ok".to_vec(), "signal 2");
    check_eq!(ask(&ch, "fence-fail:2", Vec::new())?, b"err:INVALID_ARGS".to_vec(), "chyba na dosazene");
    check_eq!(ask(&ch, "fence-fail:4", Vec::new())?, b"ok".to_vec(), "chyba od 4");
    check_eq!(f.wait(2, deadline()), Ok(()), "bod pred chybou");
    check_eq!(f.wait(3, deadline()), Err(Error::FenceFailed), "bod za chybou");
    check_eq!(f.wait(1000, deadline()), Err(Error::FenceFailed), "vzdaleny bod");
    let i = f.query().map_err(|e| fail("query", e))?;
    check_eq!((i.value, i.failed_at, i.error()), (2, 4, Some(Error::FenceFailed)), "stav");
    check_eq!(ask(&ch, "fence-signal:9", Vec::new())?, b"err:BAD_STATE".to_vec(), "po chybe nic");
    drop(ch);
    p.wait_exit(deadline()).map_err(|e| fail("konec", e))?;
    // Opusteni chybu neprepise.
    check_eq!(f.query().map(|i| i.status), Ok(Error::FenceFailed.code()), "chyba zustava");
    Ok(())
}

fn abandoned_by_kill(s: &Startup) -> TestResult {
    let (p, ch) = spawn_echo(s)?;
    let f = give_producer(&ch)?;
    check_eq!(ask(&ch, "fence-signal:1", Vec::new())?, b"ok".to_vec(), "signal 1");
    let port = Port::create().map_err(|e| fail("port", e))?;
    f.wait_async(2, &port, 9).map_err(|e| fail("wait_async", e))?;
    ch.write(b"sleep", Vec::new()).map_err(|e| fail("zapis", e))?;
    time::sleep(Duration::from_millis(10));
    p.kill().map_err(|e| fail("kill", e))?;
    p.wait_exit(deadline()).map_err(|e| fail("konec", e))?;
    let pk = port.wait(deadline()).map_err(|e| fail("paket po padu", e))?;
    check_eq!(pk.key, 9, "klic");
    check_eq!(f.query().map(|i| (i.value, i.error())), Ok((1, Some(Error::FenceAbandoned))), "stav");
    check_eq!(f.wait(1, Some(0)), Ok(()), "dosazena hodnota plati");
    check_eq!(f.wait(2, deadline()), Err(Error::FenceAbandoned), "nedosazena");
    Ok(())
}

fn abandoned_in_flight(_: &Startup) -> TestResult {
    let f = Fence::create(0).map_err(|e| fail("FENCE_CREATE", e))?;
    let w = f.for_waiter().map_err(|e| fail("dup", e))?;
    let (a, b) = Channel::create().map_err(|e| fail("kanal", e))?;
    a.write(b"plot", vec![f.into_handle()]).map_err(|e| fail("zapis", e))?;
    check_eq!(w.query().map(|i| i.status), Ok(0), "ve zprave neni opusteny");
    // Prevzeti zpravy: producent zije dal.
    let m = b.read(deadline()).map_err(|e| fail("cteni", e))?;
    let f = Fence::from_handle(m.handles.into_iter().next().ok_or("chybi handle")?);
    f.signal(1).map_err(|e| fail("signal po predani", e))?;
    // Znovu do kanalu a zahodit nevyzvednute: plot je opusteny.
    a.write(b"plot", vec![f.into_handle()]).map_err(|e| fail("zapis 2", e))?;
    drop(b);
    drop(a);
    check_eq!(w.wait(2, deadline()), Err(Error::FenceAbandoned), "zahozena zprava");
    check_eq!(w.query().map(|i| i.value), Ok(1), "hodnota zustava");
    Ok(())
}

fn wait_all_and_index(_: &Startup) -> TestResult {
    let a = Fence::create(0).map_err(|e| fail("a", e))?;
    let b = Fence::create(0).map_err(|e| fail("b", e))?;
    a.signal(3).map_err(|e| fail("a 3", e))?;
    check_eq!(fence::wait_all(&[(&a, 3), (&b, 1)], Some(time::deadline_after(Duration::from_millis(20)))), Err((Error::TimedOut, None)), "b chybi");
    b.signal(1).map_err(|e| fail("b 1", e))?;
    check_eq!(fence::wait_all(&[(&a, 3), (&b, 1)], deadline()), Ok(()), "vsechny");
    check_eq!(fence::wait_any(&[(&a, 9), (&b, 1)], deadline()), Ok(1), "index splnene");
    b.fail(2).map_err(|e| fail("b chyba", e))?;
    check_eq!(fence::wait_any(&[(&a, 9), (&b, 5)], deadline()), Err((Error::FenceFailed, Some(1))), "index chyby");
    check_eq!(fence::wait_all(&[(&b, 1), (&a, 9)], Some(0)), Err((Error::TimedOut, None)), "vsechny s dosazenou b");
    Ok(())
}

fn rights_and_args(_: &Startup) -> TestResult {
    let f = Fence::create(0).map_err(|e| fail("FENCE_CREATE", e))?;
    let w = f.for_waiter().map_err(|e| fail("dup", e))?;
    check_eq!(w.signal(1), Err(Error::AccessDenied), "WAIT nesignalizuje");
    check_eq!(
        w.handle().duplicate(Some(Rights::BASIC | Rights::SIGNAL)).err(),
        Some(Error::AccessDenied),
        "SIGNAL nejde pridat"
    );
    check_eq!(sys::fence_create(0).map(|_| ()).err(), None, "druhy plot");
    check_eq!(sys::fence_signal(f.handle().raw(), 1, 1 << 5), Err(Error::InvalidArgs), "neznamy priznak");
    check_eq!(sys::fence_wait(&[], 0, 0).err(), Some((Error::InvalidArgs, None)), "prazdny seznam");
    let items = [fence::WaitItem::new(f.handle().raw(), 1); 65];
    check_eq!(sys::fence_wait(&items, 0, 0).err(), Some((Error::InvalidArgs, None)), "nad WAIT_MANY_MAX");
    check_eq!(sys::fence_wait(&items[..1], 1 << 7, 0).err(), Some((Error::InvalidArgs, None)), "neznamy priznak cekani");
    let ev = Event::create().map_err(|e| fail("event", e))?;
    check_eq!(sys::fence_query(ev.handle().raw()).err(), Some(Error::WrongType), "event neni plot");
    check_eq!(sys::fence_create(0).and_then(|h| sys::handle_close(h)), Ok(()), "zavreni");
    // Plot zalozeny s hodnotou: hned SIGNALED.
    let g = Fence::create(10).map_err(|e| fail("plot 10", e))?;
    wait_one(g.handle(), Signals::SIGNALED, Some(0)).map_err(|e| fail("SIGNALED od zacatku", e))?;
    check_eq!(g.signal(10), Ok(()), "stejna hodnota");
    check_eq!(g.signal(9), Err(Error::InvalidArgs), "zpet");
    Ok(())
}
