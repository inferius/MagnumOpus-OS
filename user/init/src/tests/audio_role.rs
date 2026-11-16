//! Role `Audio` (relace zvuku zdedena od rodice, `docs/cs/zvuk.md`,
//! kapitola Relace zvuku pro potomky; sada `sdl`): program `gui-probe`
//! spusteny pres launchd s relaci a listkem zvuku od hlavniho supervisoru
//! (audiod je pod nim). Zvuk jde pres spolecne jadro `moaudio`
//! (`libs/mowin/src/os/audio.rs`), stejne jako SDL a Wine.
//!
//! - potomek s `MO_AUDIO` (role Audio) hraje, potomek bez ni relaci nema,
//! - po odvolani zvuku identite (`audio::revoke`) potomek prestane hrat
//!   (audiod zavre relace, obnova pres zuzeny listek je odmitnuta).

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use audioproto::session::{Capture, Recipe};
use launchproto::{Launch, Role};
use libos::{Channel, Error, Process, Startup, time};

use super::{TestResult, fail};
use crate::check;
use crate::supervisor::{SUPERVISOR, Supervisor};

const PROBE: &str = "gui-probe";
const PROBE_ID: &str = "org.magnumopus.gui-probe";
const LIMIT: Duration = Duration::from_secs(60);

struct Probe {
    process: Process,
    out: Channel,
    err: Channel,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Probe {
    fn drain(&mut self) {
        while let Ok(m) = self.out.try_read() {
            self.stdout.extend_from_slice(&m.data);
        }
        while let Ok(m) = self.err.try_read() {
            self.stderr.extend_from_slice(&m.data);
        }
    }

    fn text(&self) -> String {
        format!("stdout: {}; stderr: {}", String::from_utf8_lossy(&self.stdout).trim(), String::from_utf8_lossy(&self.stderr).trim())
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.process.kill();
    }
}

fn recipe() -> Recipe {
    // Nahravani manifest nechce: potomek ho nedostane ani pres roli.
    Recipe { app_id: String::from(PROBE_ID), app_name: String::from("Test roli Gui a Audio"), uid: 0, capture: Capture::No }
}

/// Spusti `gui-probe <mode>` pres launchd s roli Audio (relace a listek od
/// `sup`).
fn launch(sup: &mut Supervisor, mode: &str) -> Result<Probe, String> {
    let (session, lease) = crate::audio::app_session(sup, &recipe()).map_err(|e| fail("relace zvuku", e))?;
    let launch = crate::launch::client().map_err(|e| fail("kanal k launchd", e))?;
    let params = libos::params::Params { cwd: b"/".to_vec(), args: vec![PROBE.as_bytes().to_vec(), mode.as_bytes().to_vec()], env: Vec::new(), flags: 0, fds: Vec::new() }
        .encode()
        .map_err(|e| fail("blok parametru", e))?;
    let req = Launch { program: String::from(PROBE), flags: 0, roles: vec![Role::Job, Role::Stdout, Role::Stderr, Role::Audio, Role::AudioLease], params }
        .encode(1)
        .map_err(|e| fail("pozadavek", e))?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let (out, out_child) = Channel::create().map_err(|e| fail("kanal stdout", e))?;
    let (err, err_child) = Channel::create().map_err(|e| fail("kanal stderr", e))?;
    launch
        .write(
            &req,
            vec![job.duplicate(None).map_err(|e| fail("job", e))?.into_handle(), out_child.into_handle(), err_child.into_handle(), session.into_handle(), lease.into_handle()],
        )
        .map_err(|e| fail("LAUNCH", e))?;
    let m = launch.read(Some(time::deadline_after(Duration::from_secs(10)))).map_err(|e| fail("odpoved launchd", e))?;
    let (_, r) = launchproto::parse_reply(&m.data).map_err(|e| fail("odpoved launchd", e))?;
    if let Err((e, msg)) = r {
        return Err(format!("launchd: {e}: {msg}"));
    }
    let mut hs = m.handles;
    let _control = hs.pop();
    let process = Process::from_handle(hs.pop().ok_or("chybi proces")?);
    Ok(Probe { process, out, err, stdout: Vec::new(), stderr: Vec::new() })
}

/// Ridi supervisor (listky: NARROW, RECONNECT), dokud `probe` neskonci.
fn wait(s: &Startup, sup: &mut Supervisor, p: &mut Probe) -> Result<i64, String> {
    let end = time::deadline_after(LIMIT);
    loop {
        p.drain();
        match p.process.wait_exit(Some(time::deadline_after(Duration::from_millis(2)))) {
            Ok(c) => {
                p.drain();
                return Ok(c);
            }
            Err(Error::TimedOut) if time::now() <= end => {
                sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
            }
            Err(e) => return Err(format!("{e}; {}", p.text())),
        }
    }
}

/// Potomek s `MO_AUDIO` hraje, bez ni relaci nema.
pub fn inherit_and_opt_out(s: &Startup) -> TestResult {
    let mut sup = SUPERVISOR.lock();
    let mut p = launch(&mut sup, "zvuk-dite")?;
    let code = wait(s, &mut sup, &mut p)?;
    check!(code == 0, format!("dite s roli hraje, bez role ne: kod {code}; {}", p.text()));
    Ok(())
}

/// Po odvolani zvuku identite potomek (zuzeny listek) prestane hrat.
pub fn revoke_reaches_children(s: &Startup) -> TestResult {
    let mut sup = SUPERVISOR.lock();
    let mut p = launch(&mut sup, "zvuk-odvolani")?;
    let end = time::deadline_after(LIMIT);
    while !String::from_utf8_lossy(&p.stdout).contains("gui-probe: hraje") {
        if time::now() > end || p.process.wait_exit(Some(0)).is_ok() {
            p.drain();
            return Err(format!("dite nezacalo hrat; {}", p.text()));
        }
        p.drain();
        sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
    }
    let (leases, closed) = crate::audio::revoke(&mut sup, PROBE_ID).map_err(|e| fail("odvolani", e))?;
    check!(leases >= 2 && closed >= 1, format!("odvolano listku {leases}, relaci {closed}"));
    let code = wait(s, &mut sup, &mut p)?;
    check!(code == 0 && String::from_utf8_lossy(&p.stdout).contains("zvuk prestal"), format!("dite po odvolani: kod {code}; {}", p.text()));
    Ok(())
}
