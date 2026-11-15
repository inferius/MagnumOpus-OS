//! Role `Gui` (relace okna zdedena od rodice, `docs/cs/gui.md`, kapitola
//! Relace okna pro potomky; sada `gui`): program `gui-probe` spusteny pres
//! launchd s relaci a listkem od supervisoru testu.
//!
//! - potomek s `MO_GUI` (role Gui) okno otevre, potomek bez ni ne,
//! - po odvolani GUI identity (`gui::revoke`) potomek ztrati okno a obnova
//!   pres jeho (zuzeny) listek je odmitnuta.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use launchproto::{Launch, Role};
use libos::{Channel, Error, Process, Startup, time};

use super::gui::with;
use super::{TestResult, fail};
use crate::check;
use crate::gui;
use crate::supervisor::Supervisor;

const PROBE: &str = "gui-probe";
const PROBE_ID: &str = "org.magnumopus.gui-probe";
const LIMIT: Duration = Duration::from_secs(60);

/// Bezici `gui-probe` (proces, stdout a stderr).
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

/// Spusti `gui-probe <mode>` pres launchd s roli Gui (relace a listek od
/// `sup`); kanaly se ctou za behu.
fn launch(s: &Startup, sup: &mut Supervisor, mode: &str) -> Result<Probe, String> {
    let info = gui::recipe(s, PROBE).map_err(|e| fail("identita gui-probe", e))?;
    let (session, lease) = sup.session(gui::NAME, info.encode()).map_err(|e| fail("relace okna", e))?;
    let launch = crate::launch::client().map_err(|e| fail("kanal k launchd", e))?;
    let params = libos::params::Params {
        cwd: b"/".to_vec(),
        args: vec![PROBE.as_bytes().to_vec(), mode.as_bytes().to_vec()],
        // Jako u aplikace s oknem: potomci dedi relaci (osrt nastavi sam).
        env: Vec::new(),
        flags: 0,
        fds: Vec::new(),
    }
    .encode()
    .map_err(|e| fail("blok parametru", e))?;
    let req = Launch { program: String::from(PROBE), flags: 0, roles: vec![Role::Job, Role::Stdout, Role::Stderr, Role::Gui, Role::GuiLease], params }
        .encode(1)
        .map_err(|e| fail("pozadavek", e))?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let (out, out_child) = Channel::create().map_err(|e| fail("kanal stdout", e))?;
    let (err, err_child) = Channel::create().map_err(|e| fail("kanal stderr", e))?;
    launch
        .write(
            &req,
            vec![
                job.duplicate(None).map_err(|e| fail("job", e))?.into_handle(),
                out_child.into_handle(),
                err_child.into_handle(),
                session.into_handle(),
                lease.into_handle(),
            ],
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

/// Ridi supervisor testu (listky: NARROW, RECONNECT), dokud `probe`
/// neskonci; vraci jeho kod.
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
            Err(e) => {
                let _ = p.process.kill();
                return Err(format!("{e}; {}", p.text()));
            }
        }
    }
}

pub fn inherit_and_opt_out(s: &Startup) -> TestResult {
    with(|st| {
        let mut sup = Supervisor::for_tests(Duration::from_secs(10));
        st.gui.register(&mut sup);
        let mut p = launch(s, &mut sup, "dite")?;
        let code = wait(s, &mut sup, &mut p)?;
        check!(code == 0, format!("dite s roli ma okno, bez role ne: kod {code}; {}", p.text()));
        Ok(())
    })
}

pub fn revoke_reaches_children(s: &Startup) -> TestResult {
    with(|st| {
        let mut sup = Supervisor::for_tests(Duration::from_secs(10));
        st.gui.register(&mut sup);
        let before = st.gui.status().map_err(|e| fail("status", e))?.windows;
        let mut p = launch(s, &mut sup, "odvolani")?;
        // Dite drzi okno: pak GUI identite odvolat.
        let end = time::deadline_after(LIMIT);
        while !String::from_utf8_lossy(&p.stdout).contains("pripraveno") || st.gui.status().map_or(0, |x| x.windows) <= before {
            if time::now() > end {
                let _ = p.process.kill();
                return Err(format!("dite neotevrelo okno; {}", p.text()));
            }
            p.drain();
            sup.step(s, Some(time::deadline_after(Duration::from_millis(20))));
        }
        let (leases, closed) = gui::revoke(&st.gui, &mut sup, PROBE_ID).map_err(|e| fail("odvolani", e))?;
        check!(leases >= 2 && closed >= 1, format!("odvolano listku {leases}, spojeni {closed}"));
        let code = wait(s, &mut sup, &mut p)?;
        check!(code == 0, format!("dite po odvolani: kod {code}; {}", p.text()));
        let st2 = st.gui.status().map_err(|e| fail("status", e))?;
        check!(st2.windows == before, format!("okna po odvolani: {}, pred testem {before}", st2.windows));
        Ok(())
    })
}
