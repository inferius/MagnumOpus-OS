//! Ovladace z manifestu (driverkit): spusteni devmgr podle
//! `<jmeno>.driver.toml`, spolecne zpravy `dk:`, ukazkovy ovladac edu
//! a obnova ovladace po padu.
//!
//! xtask zapise do initfs `driver-test`:
//! - `expect = edu,...`: ovladace, jejichz zarizeni QEMU ma (musi bezet),
//! - `only = <jmeno>`: `cargo xtask test --driver <jmeno>` (jen ten).
//!
//! Ovladac, jehoz zarizeni v QEMU neni (napr. kostra z `new-driver` pro
//! skutecny hardware), devmgr nespusti; test to jen zaloguje.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Channel, Startup, println};

use super::{Test, TestResult, devices, fail};
use crate::check;

pub const TESTS: &[Test] = &[
    ("ocekavane ovladace bezi a odpovi na dk:ping", ping_all),
    ("edu: faktorial pres preruseni MSI", edu_factorial),
    ("edu: DMA tam a zpet (IOMMU)", edu_dma),
    ("edu: obnova po padu (dk:crash)", edu_crash_recovery),
    ("edu: restart na zadost (dk:restart)", edu_restart),
];

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// Obnova: devmgr si vsimne konce, vypne DMA, spusti novy proces.
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(10);

struct Plan {
    expect: Vec<String>,
    only: Option<String>,
}

fn plan(startup: &Startup) -> Plan {
    let text = libos::initfs::Initfs::open(startup)
        .ok()
        .and_then(|fs| fs.find("driver-test").map(|d| String::from_utf8_lossy(d).into_owned()))
        .unwrap_or_default();
    let mut p = Plan {
        expect: Vec::new(),
        only: None,
    };
    for line in text.lines() {
        match line.split_once('=') {
            Some((k, v)) if k.trim() == "expect" => {
                p.expect = v.split(',').map(|s| String::from(s.trim())).filter(|s| !s.is_empty()).collect();
            }
            Some((k, v)) if k.trim() == "only" && !v.trim().is_empty() => p.only = Some(String::from(v.trim())),
            _ => {}
        }
    }
    p
}

impl Plan {
    fn wants(&self, name: &str) -> bool {
        self.only.as_deref().is_none_or(|o| o == name)
    }
}

/// Posle zpravu ovladaci a pocka na odpoved.
fn ask(ch: &Channel, msg: &str) -> Result<String, String> {
    ch.write(msg.as_bytes(), Vec::new()).map_err(|e| fail(msg, e))?;
    let r = ch
        .read(Some(libos::time::deadline_after(REPLY_TIMEOUT)))
        .map_err(|e| fail(&format!("odpoved na '{msg}'"), e))?;
    Ok(String::from_utf8_lossy(&r.data).into_owned())
}

/// Kanal a kolikaty start ovladace `name`.
fn driver(name: &str) -> Option<(Channel, u32)> {
    let d = devices()?;
    let list = d.drivers.lock();
    let e = list.iter().find(|e| e.name == name)?;
    Some((Channel::from_handle(e.channel.handle().duplicate(None).ok()?), e.generation))
}

fn ping_all(startup: &Startup) -> TestResult {
    let plan = plan(startup);
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    let names: Vec<String> = d.drivers.lock().iter().map(|e| e.name.clone()).collect();
    for name in plan.expect.iter().filter(|n| plan.wants(n)) {
        check!(
            names.contains(name),
            format!("ovladac {name} nebezi (devmgr nenasel zarizeni, manifest nebo program; viz log devmgr)")
        );
    }
    if let Some(o) = &plan.only
        && !names.contains(o)
    {
        // Kostra pro hardware, ktery QEMU nema: test v QEMU nema co overit.
        println!("ovladac {o}: zarizeni v QEMU neni, test v QEMU preskocen (testy v simulaci: cargo test -p {o})");
    }
    for name in names.iter().filter(|n| plan.wants(n)) {
        let (ch, _) = driver(name).ok_or("kanal ovladace")?;
        let r = ask(&ch, "dk:ping")?;
        check!(r == format!("dk:pong {name}"), format!("{name}: dk:ping -> '{r}'"));
    }
    println!("ovladace z manifestu: {}", names.join(", "));
    Ok(())
}

/// Kanal edu, nebo `None` = test se preskoci (zarizeni v QEMU neni nebo
/// `--driver` vybral jiny ovladac).
fn edu(startup: &Startup) -> Result<Option<Channel>, String> {
    let plan = plan(startup);
    if !plan.wants("edu") || !plan.expect.iter().any(|n| n == "edu") {
        println!("edu: preskoceno (QEMU bez -device edu nebo jiny --driver)");
        return Ok(None);
    }
    Ok(Some(driver("edu").ok_or("ovladac edu nebezi")?.0))
}

fn edu_factorial(startup: &Startup) -> TestResult {
    let Some(ch) = edu(startup)? else { return Ok(()) };
    for (n, want) in [(5u32, 120u32), (10, 3_628_800), (12, 479_001_600)] {
        let r = ask(&ch, &format!("fact {n}"))?;
        check!(r == format!("fact {n} {want}"), format!("fact {n} -> '{r}'"));
    }
    Ok(())
}

fn edu_dma(startup: &Startup) -> TestResult {
    let Some(ch) = edu(startup)? else { return Ok(()) };
    let text: String = (0..1000).map(|i| (b'a' + (i % 26) as u8) as char).collect();
    let r = ask(&ch, &format!("dma {text}"))?;
    check!(r == format!("dma {text}"), "data po DMA tam a zpet se lisi");
    Ok(())
}

/// Pocka, az init dostane od devmgr novy kanal ovladace (`generation`
/// vetsi nez `old`). Zpravy devmgr zpracuje stejne jako supervisor.
fn wait_new_generation(name: &str, old: u32) -> Result<Channel, String> {
    let d = devices().ok_or("spravce zarizeni nebezi")?;
    let deadline = libos::time::deadline_after(RECOVERY_TIMEOUT);
    loop {
        if let Some((ch, g)) = driver(name)
            && g > old
        {
            return Ok(ch);
        }
        let m = d
            .control
            .read(Some(deadline))
            .map_err(|e| fail(&format!("{name}: devmgr neposlal obnoveny ovladac"), e))?;
        crate::system::devmgr_message(d, m);
    }
}

fn edu_crash_recovery(startup: &Startup) -> TestResult {
    let Some(ch) = edu(startup)? else { return Ok(()) };
    let (_, generation) = driver("edu").ok_or("edu")?;
    ch.write(b"dk:crash", Vec::new()).map_err(|e| fail("dk:crash", e))?;
    // Stary kanal se zavre s koncem procesu.
    check!(ch.read(Some(libos::time::deadline_after(REPLY_TIMEOUT))).is_err(), "stary kanal edu zustal otevreny");
    let ch = wait_new_generation("edu", generation)?;
    let r = ask(&ch, "fact 6")?;
    check!(r == "fact 6 720", format!("obnoveny edu: fact 6 -> '{r}'"));
    Ok(())
}

fn edu_restart(startup: &Startup) -> TestResult {
    let Some(ch) = edu(startup)? else { return Ok(()) };
    let (_, generation) = driver("edu").ok_or("edu")?;
    ch.write(b"dk:restart", Vec::new()).map_err(|e| fail("dk:restart", e))?;
    let ch = wait_new_generation("edu", generation)?;
    let r = ask(&ch, "dk:ping")?;
    check!(r == "dk:pong edu", format!("restartovany edu: '{r}'"));
    Ok(())
}
