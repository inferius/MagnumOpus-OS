//! Realny cas: nastaveni hodin (`CLOCK_SET`), prava, sluzba casu, pasma.

use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::syscall as nr;
use libos::hw::Resource;
use libos::time::{self, SetRequest, WallTime, source, zone};
use libos::{Error, Event, Rights, Startup, sys};

use super::{Test, TestResult, fail, time_service};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    (
        "CLOCK_SET bez prava SET_CLOCK -> ACCESS_DENIED",
        set_without_right,
    ),
    (
        "CLOCK_SET: neplatne priznaky a nanosekundy",
        set_invalid_args,
    ),
    ("CLOCK_SET nastavi cas (zdroj SET)", set_and_restore),
    ("sluzba casu nastavi cas i do RTC", time_service_set),
    ("mistni cas v pasmu Europe/Prague", local_time),
];

fn root(s: &Startup) -> Result<&Resource, &'static str> {
    s.root_resource.as_ref().ok_or("chybi korenovy Resource")
}

fn now_request() -> Result<SetRequest, &'static str> {
    let t = time::wall().ok_or("realny cas neni znamy")?;
    Ok(SetRequest {
        time: t,
        rtc_offset: 0,
    })
}

fn set_without_right(s: &Startup) -> TestResult {
    let root = root(s)?;
    let req = now_request()?;
    // Koren bez SET_CLOCK (takovy dostava spravce zarizeni).
    let h = root
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::MANAGE))
        .map_err(|e| fail("duplikace", e))?;
    let no_clock = Resource::from_handle(h);
    check_eq!(
        no_clock.set_clock(&req, 0).err(),
        Some(Error::AccessDenied),
        "koren bez SET_CLOCK"
    );
    // Omezeny prostredek ovladace pravo nema nikdy.
    let child = root
        .restrict(&[], 0, 0)
        .map_err(|e| fail("omezeny prostredek", e))?;
    check_eq!(
        child.set_clock(&req, 0).err(),
        Some(Error::AccessDenied),
        "omezeny prostredek"
    );
    // Jiny objekt nez Resource (prava se overuji drive nez typ).
    let ev = Event::create().map_err(|e| fail("event", e))?;
    check_eq!(
        sys::clock_set(ev.handle().raw(), &req, 0).err(),
        Some(Error::AccessDenied),
        "event"
    );
    check_eq!(
        sys::clock_set(0, &req, 0).err(),
        Some(Error::BadHandle),
        "handle 0"
    );
    // Cas se nezmenil na "nastaveny za behu" od nikoho z nich.
    let r = time::wall_reading().map_err(|e| fail("cteni", e))?;
    check!(
        r.source != source::SET || set_before(),
        "zdroj se zmenil bez prava"
    );
    Ok(())
}

/// Nastavila uz hodiny sluzba casu (oprava RTC v mistnim case)?
fn set_before() -> bool {
    crate::system::TIME_SETTINGS.contains("rtc = local")
}

fn set_invalid_args(s: &Startup) -> TestResult {
    let root = root(s)?.handle().raw();
    let req = now_request()?;
    check_eq!(
        sys::clock_set(root, &req, 1 << 7).err(),
        Some(Error::InvalidArgs),
        "neznamy priznak"
    );
    let mut bad = req.encode();
    bad[8..12].copy_from_slice(&1_000_000_000u32.to_le_bytes());
    // SAFETY: ukazatel na platny 16B buffer.
    let r = unsafe { sys::raw(nr::CLOCK_SET, root as u64, bad.as_ptr() as u64, 0, 0, 0, 0) };
    check_eq!(
        libos::abi::error::decode(r).err(),
        Some(Error::InvalidArgs),
        "nanosekundy >= 1e9"
    );
    // SAFETY: neplatny ukazatel jadro odmitne (BAD_ADDRESS), nic necte.
    let r = unsafe { sys::raw(nr::CLOCK_SET, root as u64, 0x10, 0, 0, 0, 0) };
    check_eq!(
        libos::abi::error::decode(r).err(),
        Some(Error::BadAddress),
        "spatny ukazatel"
    );
    Ok(())
}

fn set_and_restore(s: &Startup) -> TestResult {
    let root = root(s)?;
    let before = time::wall_reading().map_err(|e| fail("cteni", e))?;
    let mono = time::now();
    // O den dopredu, bez zapisu do RTC.
    let target = before.time.add_ns(86_400 * 1_000_000_000);
    let hw = root
        .set_clock(
            &SetRequest {
                time: target,
                rtc_offset: 0,
            },
            0,
        )
        .map_err(|e| fail("nastaveni", e))?;
    check!(!hw, "bez WRITE_HARDWARE se RTC nesmi zapsat");
    let after = time::wall_reading().map_err(|e| fail("cteni", e))?;
    check_eq!(after.source, source::SET, "zdroj");
    check_eq!(after.flags, 0, "priznaky (pasmo uz neni jen predpokladane)");
    let diff = after.time.as_nanos() - target.as_nanos();
    check!(
        (0..1_000_000_000).contains(&diff),
        "nastaveny cas neodpovida"
    );
    // Vratit puvodni cas (+ uplynuly), aby dalsi testy a FS meli spravny cas.
    let elapsed = time::now() - mono;
    root.set_clock(
        &SetRequest {
            time: before.time.add_ns(elapsed as i128),
            rtc_offset: 0,
        },
        0,
    )
    .map_err(|e| fail("vraceni casu", e))?;
    let back = time::wall().ok_or("cas zmizel")?;
    let diff = back.as_nanos() - before.time.as_nanos();
    check!((0..5_000_000_000).contains(&diff), "cas se nevratil");
    Ok(())
}

fn time_service_set(_: &Startup) -> TestResult {
    let t = time_service().ok_or("sluzba casu nebezi")?;
    let now = time::wall().ok_or("realny cas neni znamy")?;
    let mut msg = Vec::from(&b"set"[..]);
    msg.extend_from_slice(&now.secs.to_le_bytes());
    msg.extend_from_slice(&now.nanos.to_le_bytes());
    msg.push(1);
    t.channel
        .write(&msg, Vec::new())
        .map_err(|e| fail("zapis", e))?;
    let reply = t
        .channel
        .read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("odpoved", e))?;
    // QEMU q35 ma CMOS RTC, takze na x86_64 zapis do hardwarovych hodin musi
    // projit. Na aarch64 jadro hodiny zatim nema (cas jen z UEFI pri
    // startu), sluzba pak odpovi jen "ok".
    let expected = if cfg!(target_arch = "x86_64") {
        "ok:hw"
    } else {
        "ok"
    };
    check_eq!(
        core::str::from_utf8(&reply.data).unwrap_or("?"),
        expected,
        "odpoved sluzby"
    );
    let r = time::wall_reading().map_err(|e| fail("cteni", e))?;
    check_eq!(r.source, source::SET, "zdroj");
    Ok(())
}

fn local_time(_: &Startup) -> TestResult {
    let z = zone::Zone::by_name("Europe/Prague").ok_or("pasmo neni v tabulce")?;
    let utc = time::wall().ok_or("realny cas neni znamy")?;
    let l = time::local(&z).ok_or("mistni cas neni znamy")?;
    check!(
        l.offset.secs == 3600 || l.offset.secs == 7200,
        "posun Prahy neni 1 ani 2 h"
    );
    let back = WallTime::from_civil(&l.civil).ok_or("neplatne datum")?;
    let diff = back.secs - utc.secs - l.offset.secs as i64;
    check!((0..=2).contains(&diff), "mistni cas neodpovida UTC + posun");
    Ok(())
}
