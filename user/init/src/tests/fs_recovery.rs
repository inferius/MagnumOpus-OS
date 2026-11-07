//! Obnova fsd po padu (docs/cs/obnova-sluzeb.md): test "zabij fsd
//! uprostred prace" nad hlavnim supervisorem a skutecnym diskem.
//!
//! Klient (`fsrtest`) bezi v jinem procesu s relaci uzivatele a listkem;
//! test zabije bezici `fsd` a ridi hlavni supervisor (`step`), dokud klient
//! neodpovi. Overi: cteni pokracuje na nove instanci se stejnou identitou,
//! otevreny soubor i smazany sirotek se otevrou znovu (trvale handly),
//! ztraceny zamek se ohlasi `SERVICE_RESTARTED`, nic nevisi. Na konci
//! kontrola konzistence pres novou relaci (fsd bezi znovu).
//!
//! Sada bezi posledni: hlavni supervisor v testovacim behu nikdo jiny
//! neridi, takze relace s listkem (init, fwd) by po padu fsd cekaly.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use fsproto::perm::Mount;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Process, Startup, time};

use super::{Test, TestResult, devices, fail};
use crate::fsd::Recipe;
use crate::supervisor::{Happened, SUPERVISOR, Supervisor};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[("zabij fsd uprostred prace", kill_fsd)];

/// Nejdel, kolik test ceka na jednu udalost (restart, odpoved klienta).
const TIMEOUT: Duration = Duration::from_secs(30);
/// Testovaci uzivatel a jeho adresar.
const USER: u64 = 4242;
const DIR: &str = "/f-obnova";

struct Client {
    process: Process,
    channel: Channel,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

/// Posle klientovi prikaz a ridi supervisor, dokud neodpovi.
fn ask(sup: &mut Supervisor, s: &Startup, c: &Client, cmd: &str) -> Result<String, String> {
    c.channel
        .write(cmd.as_bytes(), Vec::new())
        .map_err(|e| format!("prikaz klientovi: {e}"))?;
    let until = time::deadline_after(TIMEOUT);
    loop {
        if let Ok(m) = c.channel.try_read() {
            return Ok(String::from_utf8_lossy(&m.data).into_owned());
        }
        sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
        if time::now() > until {
            return Err(format!("klient visi na prikazu {cmd:?}"));
        }
    }
}

/// Ridi supervisor, dokud nespusti `fsd` znovu.
fn wait_restart(sup: &mut Supervisor, s: &Startup) -> Result<(), String> {
    let until = time::deadline_after(TIMEOUT);
    let mut seen = Vec::new();
    loop {
        let h = sup.step(s, Some(time::deadline_after(Duration::from_millis(30))));
        if h.contains(&Happened::Restarted("fsd")) {
            return Ok(());
        }
        seen.extend(h);
        if time::now() > until {
            return Err(format!("supervisor fsd nespustil znovu, stalo se: {seen:?}"));
        }
    }
}

fn kill_fsd(s: &Startup) -> TestResult {
    let fs = devices().and_then(|d| d.fs.as_ref()).ok_or("sluzba fsd nebezi")?;
    // Adresar uzivatele (relace ho vidi jako /data).
    match fs.create(DIR, fsproto::kind::DIR) {
        Ok(()) | Err(libos::Error::AlreadyExists) => {}
        Err(e) => return Err(fail("adresar", e)),
    }
    fs.set_owner(DIR, USER).map_err(|e| fail("vlastnik", e))?;
    crate::system::grant_traverse(fs, "/", USER).map_err(|e| fail("pruchod", e))?;
    let mut sup = SUPERVISOR.lock();
    let gen1 = sup.generation("fsd").ok_or("fsd neni v registru supervisoru")?;
    let recipe = Recipe::User {
        user: USER,
        groups: Vec::new(),
        mounts: vec![Mount::new("/data", DIR, 0)],
    };
    // Zuzeni receptu: jen podmnozina pripojenych cest, nikdy vic.
    let sub = crate::fsd::narrowing(&[Mount::new("/data", "/f-obnova/sub", fsproto::perm::mount_flags::READ_ONLY)]);
    check!(crate::fsd::narrow_recipe(&recipe.encode(), &sub).is_ok(), "zuzeni na podadresar");
    let wider = crate::fsd::narrowing(&[Mount::new("/vse", "/", 0)]);
    check_eq!(crate::fsd::narrow_recipe(&recipe.encode(), &wider).err(), Some(libos::Error::AccessDenied), "rozsireni odmitnuto");
    let (session, lease) = sup.session("fsd", recipe.encode()).map_err(|e| fail("relace s listkem", e))?;
    let image = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = image.find("fsrtest").ok_or("fsrtest v initfs neni")?;
    let handles = vec![StartHandle::user(session.into_handle()), StartHandle::user(lease.into_handle())];
    let (process, channel) = libos::spawn("fsrtest", image, handles).map_err(|e| fail("start klienta", e))?;
    let c = Client { process, channel };

    // Prace pred padem: soubor, otevreny handle se zamkem, sirotek.
    check_eq!(ask(&mut sup, s, &c, "write /data/a.txt puvodni")?, String::from("ok "), "zapis");
    let h = ask(&mut sup, s, &c, "open /data/a.txt")?;
    let h: String = h.strip_prefix("ok ").ok_or(format!("open: {h}"))?.into();
    check_eq!(ask(&mut sup, s, &c, &format!("lock {h}"))?, String::from("ok "), "zamek");
    check_eq!(ask(&mut sup, s, &c, "write /data/smazany.txt sirotek")?, String::from("ok "), "zapis sirotka");
    let o = ask(&mut sup, s, &c, "open /data/smazany.txt")?;
    let o: String = o.strip_prefix("ok ").ok_or(format!("open sirotka: {o}"))?.into();
    fs.remove(&format!("{DIR}/smazany.txt")).map_err(|e| fail("smazani otevreneho", e))?;
    // Data nepotvrzena commitem/fsync by se padem ztratila (jako vypadek).
    fs.commit().map_err(|e| fail("commit", e))?;

    // Pad fsd.
    let p = sup.process("fsd").ok_or("fsd nebezi")?;
    libos::sys::process_kill(p.raw()).map_err(|e| fail("ukonceni fsd", e))?;
    wait_restart(&mut sup, s)?;
    check_eq!(sup.generation("fsd"), Some(gen1 + 1), "nova generace fsd");

    // Cteni pokracuje na nove instanci (relace obnovena pres listek).
    check_eq!(ask(&mut sup, s, &c, "read /data/a.txt")?, String::from("ok puvodni"), "cteni po padu");
    check_eq!(ask(&mut sup, s, &c, "gen")?, format!("ok {}", gen1 + 1), "klient na nove generaci");
    // Sirotek: nova instance ho po restartu neuklidila, klient ho otevre znovu.
    check_eq!(ask(&mut sup, s, &c, &format!("readh {o}"))?, String::from("ok sirotek"), "sirotek po padu");
    // Zamek stare instance zanikl: prvni operace to ohlasi, dalsi projde.
    let r = ask(&mut sup, s, &c, &format!("writeh {h} novy"))?;
    check_eq!(r, String::from("err SERVICE_RESTARTED"), "ztraceny zamek");
    check_eq!(ask(&mut sup, s, &c, &format!("writeh {h} novy"))?, String::from("ok 4"), "zapis po znovuotevreni");
    let r = ask(&mut sup, s, &c, "read /data/a.txt")?;
    check!(r.starts_with("ok novy"), format!("obsah po zapisu: {r}").as_str());
    check_eq!(ask(&mut sup, s, &c, &format!("close {o}"))?, String::from("ok "), "zavreni sirotka");
    check_eq!(ask(&mut sup, s, &c, &format!("close {h}"))?, String::from("ok "), "zavreni");
    drop(c);

    // fsd bezi znovu: nova ridici relace a kontrola konzistence.
    let (ch, lease) = sup.session("fsd", Recipe::System.encode()).map_err(|e| fail("nova relace", e))?;
    drop(sup);
    let fresh = FsClient::with_lease(ch, lease);
    let problems = fresh.check(true).map_err(|e| fail("kontrola", e))?;
    check!(problems.is_empty(), format!("problemy: {problems:?}").as_str());
    Ok(())
}
