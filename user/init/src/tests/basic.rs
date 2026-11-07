//! Zakladni syscally: cas, handly, eventy, cekani, chybove kody.

use core::time::Duration;

use libos::{Error, Event, Handle, Rights, Signals, Startup, sys, time, wait_many, wait_one};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("cas roste", clock),
    ("realny cas: zdroj a beh", wall_clock),
    ("neplatny handle -> BAD_HANDLE", bad_handle),
    ("neznamy syscall -> BAD_SYSCALL", bad_syscall),
    ("event: signal a cekani", event_signal),
    ("cekani s timeoutem -> TIMED_OUT", wait_timeout),
    ("wait_many vrati spravny index", wait_many_index),
    ("duplikace jen ubira prava", duplicate_rights),
    (
        "signal s nepovolenym bitem -> INVALID_ARGS",
        signal_bad_bits,
    ),
];

fn clock(_: &Startup) -> TestResult {
    let a = time::now();
    time::sleep(Duration::from_millis(5));
    let b = time::now();
    check!(b >= a + 5_000_000, "sleep 5 ms trval mene nez 5 ms");
    Ok(())
}

fn wall_clock(_: &Startup) -> TestResult {
    // QEMU s OVMF ma vzdy hodiny (UEFI i CMOS) nastavene podle hostitele.
    // ntpd (v testech server hostitele) muze cas stihnout seridit drive
    // (zdroj NETWORK) - po startu init a pred timto testem.
    let r = time::wall_reading().map_err(|e| fail("CLOCK_REALTIME", e))?;
    check!(
        matches!(r.source, time::source::UEFI | time::source::RTC | time::source::NETWORK),
        alloc::format!("neocekavany zdroj casu {}", r.source)
    );
    let c = r.time.to_civil();
    check!(c.year >= 2024 && c.year < 2200, alloc::format!("nesmyslny rok {}", c.year));
    time::sleep(Duration::from_millis(20));
    let b = time::wall().ok_or_else(|| alloc::string::String::from("realny cas zmizel"))?;
    let d = b.as_nanos() - r.time.as_nanos();
    check!(
        (20_000_000..2_000_000_000).contains(&d),
        alloc::format!("realny cas se za 20 ms posunul o {d} ns")
    );
    Ok(())
}

fn bad_handle(_: &Startup) -> TestResult {
    check_eq!(
        sys::handle_close(0),
        Err(Error::BadHandle),
        "zavreni handlu 0"
    );
    check_eq!(
        sys::handle_close(0x00ab_cdef),
        Err(Error::BadHandle),
        "zavreni neexistujiciho handlu"
    );
    Ok(())
}

fn bad_syscall(_: &Startup) -> TestResult {
    check_eq!(
        sys::invalid_syscall(),
        Err(Error::BadSyscall),
        "neznamy syscall"
    );
    Ok(())
}

fn event_signal(_: &Startup) -> TestResult {
    let e = Event::create().map_err(|e| fail("vytvoreni eventu", e))?;
    e.signal().map_err(|e| fail("signal", e))?;
    let s = wait_one(
        e.handle(),
        Signals::SIGNALED,
        Some(time::deadline_after(Duration::from_secs(1))),
    )
    .map_err(|e| fail("cekani na signalizovany event", e))?;
    check!(s.contains(Signals::SIGNALED), "event neni signalizovany");
    e.reset().map_err(|e| fail("reset", e))?;
    Ok(())
}

fn wait_timeout(_: &Startup) -> TestResult {
    let e = Event::create().map_err(|e| fail("vytvoreni eventu", e))?;
    let r = wait_one(
        e.handle(),
        Signals::SIGNALED,
        Some(time::deadline_after(Duration::from_millis(10))),
    );
    check_eq!(
        r.err(),
        Some(Error::TimedOut),
        "cekani na nesignalizovany event"
    );
    Ok(())
}

fn wait_many_index(_: &Startup) -> TestResult {
    let a = Event::create().map_err(|e| fail("event a", e))?;
    let b = Event::create().map_err(|e| fail("event b", e))?;
    b.signal().map_err(|e| fail("signal b", e))?;
    let (i, sig) = wait_many(
        &[
            (a.handle(), Signals::SIGNALED),
            (b.handle(), Signals::SIGNALED),
        ],
        Some(time::deadline_after(Duration::from_secs(1))),
    )
    .map_err(|e| fail("wait_many", e))?;
    check_eq!(i, 1, "index splneneho objektu");
    check!(sig[1].contains(Signals::SIGNALED), "observed signaly b");
    check!(!sig[0].contains(Signals::SIGNALED), "observed signaly a");
    Ok(())
}

fn duplicate_rights(_: &Startup) -> TestResult {
    let e = Event::create().map_err(|e| fail("vytvoreni eventu", e))?;
    let weak: Handle = e
        .handle()
        .duplicate(Some(Rights::WAIT | Rights::DUPLICATE))
        .map_err(|e| fail("dup", e))?;
    let weak = Event::from_handle(weak);
    check_eq!(
        weak.signal(),
        Err(Error::AccessDenied),
        "signal bez prava SIGNAL"
    );
    check_eq!(
        weak.handle()
            .duplicate(Some(Rights::WAIT | Rights::SIGNAL))
            .err(),
        Some(Error::AccessDenied),
        "pridani prava duplikaci"
    );
    Ok(())
}

fn signal_bad_bits(_: &Startup) -> TestResult {
    let e = Event::create().map_err(|e| fail("vytvoreni eventu", e))?;
    check_eq!(
        sys::object_signal(e.handle().raw(), 0, Signals::TERMINATED.0),
        Err(Error::InvalidArgs),
        "nastaveni TERMINATED"
    );
    Ok(())
}
