//! Kostra testu "zabij sluzbu" (viz `docs/cs/obnova-sluzeb.md`, kapitola
//! "Testy zabij sluzbu").
//!
//! [`Harness`] ma vlastni supervisor (`Supervisor::for_tests`), ktery test
//! ridi pres `step`. Klient bezi v **jinem procesu** (`svctest` v roli
//! klienta): klient v initu by pri obnove cekal na supervisor, ktery bezi
//! ve stejnem vlakne. Testy sluzeb (fsd, netd, ...) muzou kostru pouzit
//! s vlastnim adapterem sluzby ([`Harness::add`] s libovolnym `Service`)
//! a vlastnim programem klienta.
//!
//! Testovaci sluzba `svctest` ma adapter [`TestSvc`]: recept = identita
//! (`alice`), zuzeni = pripona (`alice/child`).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, time};

use crate::supervisor::{Dep, Happened, Service, Spec, StartCtx, Started, Supervisor};

/// Lhuta na potvrzeni startu v testech (kratsi nez v systemu).
pub const START_TIMEOUT: Duration = Duration::from_secs(5);
/// Nejdel, kolik test ceka na jednu udalost.
pub const TIMEOUT: Duration = Duration::from_secs(25);

/// Stav adapteru `svctest` sdileny s testem.
#[derive(Default)]
pub struct Shared {
    process: Option<Process>,
    control: Option<Channel>,
    events: Option<Channel>,
    /// Rezim instanci od generace 2 (`normal`, `slow:<ms>`, `crash`).
    pub mode_after: &'static str,
    /// Zpravy `dependency_restarted` (zavislost, generace).
    pub notified: Vec<(String, u64)>,
}

pub type SharedRef = Arc<spin::Mutex<Shared>>;

/// Adapter testovaci sluzby `svctest`.
pub struct TestSvc(pub SharedRef);

impl Service for TestSvc {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let fs = Initfs::open(ctx.startup)?;
        let image = fs.find("svctest").ok_or(Error::NotFound)?;
        let (events, theirs) = Channel::create()?;
        let (process, control) = libos::spawn("svctest", image, vec![StartHandle::user(theirs.into_handle())])?;
        let mut s = self.0.lock();
        let mode = if ctx.generation == 1 { "normal" } else { s.mode_after };
        control.write(format!("serve {} {mode}", ctx.generation).as_bytes(), Vec::new())?;
        let started = Started {
            process: process.handle().duplicate(None)?,
            ready: Some(control.handle().duplicate(None)?),
        };
        s.process = Some(process);
        s.control = Some(control);
        s.events = Some(events);
        Ok(started)
    }

    fn ready(&mut self) -> Result<bool> {
        let s = self.0.lock();
        let control = s.control.as_ref().ok_or(Error::BadState)?;
        match control.try_read() {
            Ok(m) if m.data == b"ready" => Ok(true),
            Ok(_) => Err(Error::Corrupted),
            Err(Error::ShouldWait) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn connect(&mut self, recipe: &[u8]) -> Result<Channel> {
        let s = self.0.lock();
        let control = s.control.as_ref().ok_or(Error::ServiceUnavailable)?;
        let mut cmd = Vec::from(&b"connect "[..]);
        cmd.extend_from_slice(recipe);
        control.write(&cmd, Vec::new())?;
        let mut m = control.read(Some(time::deadline_after(Duration::from_secs(2))))?;
        if m.data != b"ok" || m.handles.len() != 1 {
            return Err(Error::Corrupted);
        }
        Ok(Channel::from_handle(m.handles.remove(0)))
    }

    /// Zuzeni = jedna dalsi slozka identity; nic jineho (zadne `/`, `..`,
    /// prazdne) - to by mohlo identitu rozsirit.
    fn narrow(&self, parent: &[u8], narrowing: &[u8]) -> Result<Vec<u8>> {
        let ok = !narrowing.is_empty()
            && narrowing != b".."
            && narrowing.iter().all(|&b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            return Err(Error::AccessDenied);
        }
        let mut r = parent.to_vec();
        r.push(b'/');
        r.extend_from_slice(narrowing);
        Ok(r)
    }

    fn dependency_restarted(&mut self, dep: &str, generation: u64) {
        self.0.lock().notified.push((String::from(dep), generation));
    }

    fn stopped(&mut self) {
        let mut s = self.0.lock();
        s.process = None;
        s.control = None;
        s.events = None;
    }
}

/// Klient v jinem procesu (`svctest client`).
pub struct Client {
    process: Process,
    channel: Channel,
}

pub struct Harness<'a> {
    pub sup: Supervisor,
    startup: &'a Startup,
    services: Vec<(&'static str, SharedRef)>,
    clients: Vec<Client>,
    /// Vse, co supervisor od zacatku testu udelal.
    pub log: Vec<Happened>,
}

impl<'a> Harness<'a> {
    pub fn new(startup: &'a Startup) -> Self {
        Self {
            sup: Supervisor::for_tests(START_TIMEOUT),
            startup,
            services: Vec::new(),
            clients: Vec::new(),
            log: Vec::new(),
        }
    }

    /// Zaregistruje instanci `svctest` jmenem `name` a pocka na jeji start.
    pub fn add(&mut self, name: &'static str, deps: &'static [Dep], mode_after: &'static str) -> core::result::Result<SharedRef, String> {
        let shared: SharedRef = Arc::new(spin::Mutex::new(Shared {
            mode_after,
            ..Shared::default()
        }));
        self.add_service(name, deps, Box::new(TestSvc(shared.clone())))?;
        self.services.push((name, shared.clone()));
        Ok(shared)
    }

    /// Zaregistruje sluzbu s vlastnim adapterem a pocka na jeji start.
    pub fn add_service(&mut self, name: &'static str, deps: &'static [Dep], service: Box<dyn Service>) -> core::result::Result<(), String> {
        self.sup.register(
            Spec {
                name,
                deps,
                service: Some(service),
            },
            None,
        );
        self.drive_until(|h| h.contains(&Happened::Restarted(name)))
            .map(|_| ())
    }

    /// Ridi supervisor, dokud `done` nad vsim, co se od volani stalo,
    /// nevrati `true`. Vraci tyto udalosti.
    pub fn drive_until(&mut self, mut done: impl FnMut(&[Happened]) -> bool) -> core::result::Result<Vec<Happened>, String> {
        let until = time::deadline_after(TIMEOUT);
        let mut seen = Vec::new();
        loop {
            let h = self
                .sup
                .step(self.startup, Some(time::deadline_after(Duration::from_millis(30))));
            seen.extend(h.iter().cloned());
            self.log.extend(h);
            if done(&seen) {
                return Ok(seen);
            }
            if time::now() > until {
                return Err(format!("supervisor nereagoval, stalo se: {seen:?}"));
            }
        }
    }

    /// Klient `svctest` s relaci a listkem podle receptu `recipe`; vraci
    /// jeho cislo pro [`Self::send`] a [`Self::ask`].
    pub fn client(&mut self, service: &str, recipe: &str) -> core::result::Result<usize, String> {
        let (session, lease) = self
            .sup
            .session(service, recipe.as_bytes().to_vec())
            .map_err(|e| format!("relace pro klienta: {e}"))?;
        let fs = Initfs::open(self.startup).map_err(|e| format!("initfs: {e}"))?;
        let image = fs.find("svctest").ok_or("svctest v initfs neni")?;
        let handles = vec![
            StartHandle::user(session.into_handle()),
            StartHandle::user(lease.into_handle()),
        ];
        let (process, channel) = libos::spawn("svctest-client", image, handles).map_err(|e| format!("start klienta: {e}"))?;
        channel.write(b"client", Vec::new()).map_err(|e| format!("klient: {e}"))?;
        self.clients.push(Client { process, channel });
        Ok(self.clients.len() - 1)
    }

    /// Posle klientovi prikaz (odpoved precte [`Self::reply`]).
    pub fn send(&self, client: usize, cmd: &str) -> core::result::Result<(), String> {
        self.clients[client]
            .channel
            .write(cmd.as_bytes(), Vec::new())
            .map_err(|e| format!("prikaz klientovi: {e}"))
    }

    /// Ridi supervisor, dokud klient neodpovi; vraci odpoved a jak dlouho
    /// trvala (ms).
    pub fn reply(&mut self, client: usize) -> core::result::Result<(String, u64), String> {
        let start = time::now();
        let until = time::deadline_after(TIMEOUT);
        loop {
            if let Ok(m) = self.clients[client].channel.try_read() {
                let ms = (time::now() - start) / 1_000_000;
                return Ok((String::from_utf8_lossy(&m.data).into_owned(), ms));
            }
            let h = self
                .sup
                .step(self.startup, Some(time::deadline_after(Duration::from_millis(20))));
            self.log.extend(h);
            if time::now() > until {
                return Err(String::from("klient visi: neodpovedel do lhuty"));
            }
        }
    }

    /// Prikaz a odpoved.
    pub fn ask(&mut self, client: usize, cmd: &str) -> core::result::Result<String, String> {
        self.send(client, cmd)?;
        self.reply(client).map(|(r, _)| r)
    }

    /// Ridi supervisor, dokud sluzba `name` neohlasi drzeny pozadavek.
    pub fn wait_held(&mut self, name: &str) -> core::result::Result<(), String> {
        let shared = self.shared(name)?;
        let until = time::deadline_after(TIMEOUT);
        loop {
            let got = shared
                .lock()
                .events
                .as_ref()
                .is_some_and(|e| e.try_read().is_ok_and(|m| m.data == b"held"));
            if got {
                return Ok(());
            }
            let h = self
                .sup
                .step(self.startup, Some(time::deadline_after(Duration::from_millis(20))));
            self.log.extend(h);
            if time::now() > until {
                return Err(format!("sluzba {name} pozadavek nedostala"));
            }
        }
    }

    fn shared(&self, name: &str) -> core::result::Result<SharedRef, String> {
        self.services
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| s.clone())
            .ok_or_else(|| format!("sluzba {name} neni v testu"))
    }

    /// Zabije bezici instanci sluzby (jako pad).
    pub fn kill(&self, name: &str) -> core::result::Result<(), String> {
        let p = self.sup.process(name).ok_or_else(|| format!("{name} nebezi"))?;
        libos::sys::process_kill(p.raw()).map_err(|e| format!("ukonceni {name}: {e}"))
    }

    /// Zabije sluzbu a pocka, az ji supervisor spusti znovu.
    pub fn kill_and_restart(&mut self, name: &'static str) -> core::result::Result<Vec<Happened>, String> {
        self.kill(name)?;
        self.drive_until(|h| h.contains(&Happened::Restarted(name)))
    }
}

impl Drop for Harness<'_> {
    fn drop(&mut self) {
        for c in &self.clients {
            let _ = c.process.kill();
        }
        for (name, _) in &self.services {
            if let Some(p) = self.sup.process(name) {
                let _ = libos::sys::process_kill(p.raw());
            }
        }
    }
}
