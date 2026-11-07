//! Synchronizace casu: plynula korekce v jadre (`CLOCK_ADJUST`), pravidla
//! sluzby casu pro korekce a `ntpd` proti pomocnemu serveru NTP hostitele.
//!
//! `ntpd` bezi od startu a koriguje soubezne s testy, proto testy jadra
//! overuji vlastnosti, ktere plati vzdy (realny cas nikdy nejde zpet,
//! rychlost v mezich), ne presne hodnoty (ty overuje test jadra
//! `Timeline`).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::time::{AdjustStatus, adjust};
use libos::hw::Resource;
use libos::{Channel, Error, Rights, Startup, sys, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("CLOCK_ADJUST: meze, prava, cas bez znameho stavu", adjust_args),
    ("CLOCK_ADJUST: dorovnani zpet bez kroku zpet", adjust_monotonic),
    ("ntpd: hodiny serizene podle serveru hostitele", ntpd_synced),
    ("timed: korekce od klienta synchronizace", timed_policy),
];

fn root(s: &Startup) -> Result<&Resource, &'static str> {
    s.root_resource.as_ref().ok_or("chybi korenovy Resource")
}

fn adjust_args(s: &Startup) -> TestResult {
    let r = root(s)?.handle().raw();
    let st = sys::clock_adjust(r, 0, 0, 0).map_err(|e| fail("stav", e))?;
    check!(st.freq_ppb.abs() <= adjust::MAX_FREQ_PPB, "frekvence v mezich");
    let bad = [
        (adjust::MAX_OFFSET_NS + 1, 0, adjust::OFFSET),
        (0, adjust::MAX_FREQ_PPB + 1, adjust::FREQUENCY),
        (0, -adjust::MAX_FREQ_PPB - 1, adjust::FREQUENCY),
        (0, 0, 1 << 7),
    ];
    for (o, f, fl) in bad {
        check_eq!(sys::clock_adjust(r, o, f, fl).err(), Some(Error::InvalidArgs), format!("meze {o} {f} {fl}"));
    }
    // Bez prava SET_CLOCK nic.
    let weak = root(s)?
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::MANAGE))
        .map_err(|e| fail("duplikace", e))?;
    check_eq!(
        sys::clock_adjust(weak.raw(), 1000, 0, adjust::OFFSET).err(),
        Some(Error::AccessDenied),
        "bez SET_CLOCK"
    );
    Ok(())
}

fn adjust_monotonic(s: &Startup) -> TestResult {
    let r = root(s)?.handle().raw();
    let before: AdjustStatus = sys::clock_adjust(r, 0, 0, 0).map_err(|e| fail("stav", e))?;
    // Hodiny "o 50 ms napred": dorovnat zpatky. Cas se musi jen zpomalit.
    let st = sys::clock_adjust(r, -50_000_000, 0, adjust::OFFSET).map_err(|e| fail("dorovnani", e))?;
    check_eq!(st.slew_ppb, -adjust::MAX_SLEW_PPB, "rychlost dorovnani");
    check!(st.remaining_ns <= 0 && st.remaining_ns >= -50_000_000, "zbyva dorovnat");
    let (mut last_wall, mut last_mono, mut last_u) = sample()?;
    for _ in 0..40 {
        time::sleep(Duration::from_millis(25));
        let (w, m, u) = sample()?;
        check!(w >= last_wall, "realny cas sel zpet");
        let (dw, dm) = (w - last_wall, m - last_mono);
        // Rychlost 1 +/- (slew + frekvence), vse nejvys 1000 ppm, plus
        // nejistota cteni obou vzorku.
        let tol = dm / 1000 + u + last_u + 1_000;
        check!((dw - dm).abs() <= tol, format!("rychlost mimo meze: {dw} ns za {dm} ns (+/- {tol})"));
        last_wall = w;
        last_mono = m;
        last_u = u;
    }
    // Vratit puvodni stav (ntpd pak pokracuje sam).
    sys::clock_adjust(r, before.remaining_ns, before.freq_ppb, adjust::OFFSET | adjust::FREQUENCY)
        .map_err(|e| fail("obnova", e))?;
    Ok(())
}

/// Realny cas, monotonni cas uprostred jeho cteni a nejistota (polovina
/// doby mezi dvema ctenimi monotonniho casu). Vybere nejpresnejsi z nekolika
/// pokusu (planovac muze vlakno mezi syscally prerusit).
fn sample() -> Result<(i128, i128, i128), String> {
    let mut best: Option<(i128, i128, i128)> = None;
    for _ in 0..8 {
        let m1 = time::now() as i128;
        let w = time::wall().ok_or("realny cas neni znamy")?.as_nanos();
        let m2 = time::now() as i128;
        let u = (m2 - m1) / 2 + 1;
        if best.is_none_or(|b| u < b.2) {
            best = Some((w, m1 + u, u));
        }
    }
    best.ok_or_else(|| String::from("cteni casu"))
}

/// Pripoji k timed dalsi kanal klienta synchronizace (jako ntpd).
fn sync_client() -> Result<Channel, String> {
    let (mine, theirs) = Channel::create().map_err(|e| fail("kanal", e))?;
    let guard = crate::system::TIME.lock();
    let t = guard.as_ref().ok_or("sluzba casu nebezi")?;
    t.channel
        .write(b"ntp", vec![theirs.into_handle()])
        .map_err(|e| fail("ntp", e))?;
    let r = t
        .channel
        .read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("odpoved", e))?;
    check_eq!(r.data.as_slice(), b"ok", "pripojeni klienta");
    Ok(mine)
}

fn sync(c: &Channel, offset: i64) -> Result<Vec<u8>, String> {
    let mut m = Vec::from(&b"sync"[..]);
    m.extend_from_slice(&offset.to_le_bytes());
    m.extend_from_slice(&0i64.to_le_bytes());
    m.push(0);
    c.write(&m, Vec::new()).map_err(|e| fail("sync", e))?;
    Ok(c.read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("odpoved sync", e))?
        .data)
}

fn timed_status() -> Result<String, String> {
    let guard = crate::system::TIME.lock();
    let t = guard.as_ref().ok_or("sluzba casu nebezi")?;
    t.channel.write(b"status", Vec::new()).map_err(|e| fail("status", e))?;
    let r = t
        .channel
        .read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("status", e))?;
    String::from_utf8(r.data).map_err(|_| String::from("status neni UTF-8"))
}

fn ntpd_synced(_: &Startup) -> TestResult {
    let Some(ntp) = crate::system::NTP.get() else {
        // Bez site (cargo xtask run bez karty) ntpd nebezi.
        libos::println!("[preskoceno] cas: ntpd nebezi");
        return Ok(());
    };
    let mut status = String::new();
    for _ in 0..150 {
        status = timed_status()?;
        if status.contains("synced=1") {
            break;
        }
        time::sleep(Duration::from_millis(200));
    }
    check!(status.contains("synced=1"), format!("timed nedostal korekci: {status}"));
    ntp.control.write(b"status", Vec::new()).map_err(|e| fail("status ntpd", e))?;
    let r = ntp
        .control
        .read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("status ntpd", e))?;
    let r = String::from_utf8(r.data).unwrap_or_default();
    check!(r.starts_with("ok:rounds="), format!("stav ntpd: {r}"));
    check!(r.contains("timed=ok:"), format!("timed korekci neprijal: {r}"));
    // Hodiny QEMU jdou z hodin hostitele: po synchronizaci je posun maly.
    let off: i128 = r
        .split_whitespace()
        .find_map(|w| w.strip_prefix("offset_ns="))
        .and_then(|v| v.parse().ok())
        .ok_or("posun ve stavu ntpd")?;
    check!(off.abs() < 2_000_000_000, format!("posun {} ms", off / 1_000_000));
    Ok(())
}

fn timed_policy(_: &Startup) -> TestResult {
    let c = sync_client()?;
    // Prvni korekce smi skocit (nad 128 ms) nebo dorovnat; dalsi uz jen dorovnavat.
    let first = sync(&c, 0)?;
    check!(first.starts_with(b"ok:"), "prvni korekce");
    let before = time::wall().ok_or("cas")?.as_nanos();
    let r = sync(&c, -20_000_000)?;
    check!(r.starts_with(b"ok:slew"), format!("korekce zpet: {:?}", String::from_utf8_lossy(&r)));
    check_eq!(r.len(), 7 + AdjustStatus::SIZE, "stav v odpovedi");
    let after = time::wall().ok_or("cas")?.as_nanos();
    check!(after >= before, "korekce zpet nesmi skocit");
    // Podezrele velka korekce po synchronizaci se odmitne.
    let r = sync(&c, -120_000_000_000)?;
    check!(r.starts_with(b"err:OUT_OF_RANGE"), format!("velky posun: {}", String::from_utf8_lossy(&r)));
    check_eq!(sync_raw(&c, b"blbost")?, b"err:neznama zprava".to_vec(), "neznama zprava");
    // Zpet do puvodniho stavu (ntpd pokracuje).
    let _ = sync(&c, 20_000_000)?;
    Ok(())
}

fn sync_raw(c: &Channel, m: &[u8]) -> Result<Vec<u8>, String> {
    c.write(m, Vec::new()).map_err(|e| fail("zprava", e))?;
    Ok(c.read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| fail("odpoved", e))?
        .data)
}
