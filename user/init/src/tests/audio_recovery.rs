//! Obnova zvuku po padu (docs/cs/obnova-sluzeb.md): "zabij audiod"
//! a "zabij ovladac hda" nad hlavnim supervisorem.
//!
//! - audiod: supervisor ho spusti znovu s duplikaty kanalu k ovladacum
//!   (Kept), takze nova instance hned ma zarizeni. Klient s listkem se
//!   pripoji sam (volani bezi ve vlakne, hlavni vlakno ridi supervisor).
//! - hda: devmgr ho spusti znovu a posle initu novy kanal; init ho preda
//!   bezici sluzbe (zprava `device`) a ta prejde na nove zarizeni.

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use core::time::Duration;

use audioproto::conn::AudioClient;
use audioproto::{Direction, Format};
use libos::{Startup, time};

use super::{Test, TestResult, fail};
use crate::check;
use crate::supervisor::{Happened, SUPERVISOR, Supervisor};

pub const TESTS: &[Test] = &[("zabij audiod", kill_audiod), ("zabij ovladac hda", kill_driver)];

const TIMEOUT: Duration = Duration::from_secs(30);

/// Ridi supervisor, dokud `done` nevrati `true`.
fn drive(sup: &mut Supervisor, s: &Startup, what: &str, mut done: impl FnMut(&[Happened]) -> bool) -> TestResult {
    let until = time::deadline_after(TIMEOUT);
    let mut seen = alloc::vec::Vec::new();
    loop {
        let h = sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
        seen.extend(h);
        if done(&seen) {
            return Ok(());
        }
        if time::now() > until {
            return Err(format!("{what}: nedockal jsem se, stalo se {seen:?}"));
        }
    }
}

/// Kratke prehrani (overi, ze zarizeni opravdu hraje).
fn play_short(a: &AudioClient) -> TestResult {
    let s = a.open(Direction::Output, Format::DEVICE_DEFAULT, 50_000).map_err(|e| fail("OPEN", e))?;
    s.write_all(&alloc::vec![0u8; 48 * 4 * 100], true, Some(time::deadline_after(Duration::from_secs(10))))
        .map_err(|e| fail("zapis", e))?;
    s.drain().map_err(|e| fail("DRAIN", e))?;
    s.close().map_err(|e| fail("CLOSE", e))
}

fn kill_audiod(s: &Startup) -> TestResult {
    let mut sup = SUPERVISOR.lock();
    let (ch, lease) = sup.session("audiod", alloc::vec::Vec::new()).map_err(|e| fail("relace s listkem", e))?;
    let leased = AudioClient::with_lease(ch, lease);
    let before = leased.status().map_err(|e| fail("STATUS", e))?;
    let p = sup.process("audiod").ok_or("audiod nebezi pod supervisorem")?;
    libos::sys::process_kill(p.raw()).map_err(|e| fail("ukonceni audiod", e))?;
    drive(&mut sup, s, "restart audiod", |h| h.contains(&Happened::Restarted("audiod")))?;
    // Nova instance ma zarizeni hned (kanaly drzel supervisor).
    let fresh = AudioClient::new(crate::audio::connect().map_err(|e| fail("CONNECT", e))?);
    let st = fresh.status().map_err(|e| fail("STATUS nove instance", e))?;
    check!(st.device == before.device, format!("po restartu jine zarizeni: '{}' (pred: '{}')", st.device, before.device));
    play_short(&fresh)?;
    // Klient s listkem: obnova pres supervisor (ten ridi hlavni vlakno).
    let result: Arc<spin::Mutex<Option<Result<String, String>>>> = Arc::new(spin::Mutex::new(None));
    let (r, c) = (result.clone(), leased.clone());
    let t = libos::thread::spawn("audio-lease", move || {
        *r.lock() = Some(c.status().map(|s| s.device).map_err(|e| format!("{e}")));
    })
    .map_err(|e| fail("vlakno", e))?;
    drive(&mut sup, s, "obnova relace s listkem", |_| result.lock().is_some())?;
    let _ = t.join(None);
    let r = result.lock().take().unwrap_or(Err(String::from("bez vysledku")));
    check!(r.as_deref() == Ok(before.device.as_str()), format!("klient s listkem po restartu: {r:?}"));
    Ok(())
}

fn kill_driver(s: &Startup) -> TestResult {
    let mut sup = SUPERVISOR.lock();
    let a = AudioClient::new(crate::audio::connect().map_err(|e| fail("CONNECT", e))?);
    let before = a.status().map_err(|e| fail("STATUS", e))?;
    check!(!before.device.is_empty(), "audiod nema zarizeni");
    crate::audio::crash_driver().map_err(|e| fail("pad ovladace", e))?;
    // devmgr ovladac spusti znovu a posle `audio`; supervisor ho preda.
    drive(&mut sup, s, "novy ovladac", |h| h.contains(&Happened::DeviceAdded))?;
    let until = time::deadline_after(TIMEOUT);
    loop {
        let st = a.status().map_err(|e| fail("STATUS", e))?;
        if st.device == before.device {
            break;
        }
        if time::now() > until {
            return Err(format!("sluzba nema po restartu ovladace zarizeni (je '{}')", st.device));
        }
        sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
    }
    play_short(&a)
}
