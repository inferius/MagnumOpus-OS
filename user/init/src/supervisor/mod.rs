//! Supervisor: smycka udalosti initu po startu systemu.
//!
//! Hlida systemove sluzby a spravce zarizeni (protokol obnovy viz
//! `docs/cs/obnova-sluzeb.md`):
//!
//! - **Registr sluzeb** ([`Spec`]): jmeno, zavislosti ([`Dep`]) a adapter
//!   ([`Service`]), ktery umi spustit novou instanci a vytvorit na ni
//!   relaci podle receptu. Kazda instance ma generaci (prvni start = 1).
//! - **Pad sluzby** (`lifecycle.rs`): restart s rostouci prodlevou (100 ms,
//!   200 ms, ... nejvys 5 s) az po startu zavislosti. Potvrzeni nove
//!   instance je dalsi polozka smycky (neblokuje); kdyz do lhuty
//!   neprijde, instance se ukonci a pocita se to jako pad. Vic nez
//!   [`RESTART_MAX`] restartu za [`RESTART_WINDOW`] = vzdat. Sluzby
//!   s `Dep::restart` na spadle sluzbe se ukonci a spusti po ni.
//! - **Listky** (`leases.rs`): klient po padu pozada pres svuj listek
//!   o novou relaci se stejnym receptem; supervisor odpovi, az bezi nova
//!   instance (nebo chybou po lhute klienta).
//! - **Zarizeni pripojena za behu**: zpravy spravce zarizeni `input`
//!   preda sluzbe vstupu (`device`). Disky za behu (`block`) zatim jen
//!   zaloguje.
//!
//! - **Terminal**: kanaly pripojeni oken terminalu, registry relaci
//!   a procesy `termd` ([`crate::terminal`]); po padu kompozitoru spusti
//!   zalozni konzoli.
//!
//! Jedno kolo smycky je [`Supervisor::step`] (testy ji tak mohou ridit
//! a videt, co se stalo); [`Supervisor::run`] ho opakuje do nekonecna.

mod leases;
mod lifecycle;
mod service;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::time::Duration;

use libos::{Handle, Port, Signals, Startup, println};

// DepMode a Kept pouzivaji adaptery sluzeb (zapojuji vlakna sluzeb).
#[allow(unused_imports)]
pub use service::{Dep, DepMode, Kept, Restart, Service, Spec, StartCtx, Started};
pub use svcclient::proto::State;

use crate::system;

/// Okno, ve kterem se pocitaji restarty jedne sluzby.
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// Nejvic restartu v okne; dalsi pad = sluzba se vzda.
pub const RESTART_MAX: usize = 5;
/// Prodleva pred prvnim restartem (pak dvojnasobna, nejvys 5 s).
const RESTART_DELAY: Duration = Duration::from_millis(100);
const RESTART_DELAY_MAX: Duration = Duration::from_secs(5);
/// Jak dlouho ma nova sluzba na potvrzeni startu.
pub const START_TIMEOUT: Duration = Duration::from_secs(5);

/// Co se v kole stalo (pro testy a log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Happened {
    /// Zarizeni pripojene za behu predane sluzbe vstupu.
    DeviceAdded,
    /// Sluzba skoncila s kodem.
    Exited { name: &'static str, code: i64 },
    Restarted(&'static str),
    /// Prilis mnoho padu (nebo to vzdala zavislost): sluzba zustane
    /// zastavena.
    GaveUp(&'static str),
    /// Sluzba ukoncena, protoze spadla jeji zavislost (`Dep::restart`).
    Stopped { name: &'static str, because: &'static str },
    /// Sluzba dostala zpravu, ze zavislost (`Dep::notify`) bezi znovu.
    Notified { name: &'static str, dep: &'static str },
    /// Klient dostal pres listek novou relaci na generaci `generation`.
    Reconnected { name: &'static str, generation: u64 },
    /// Spravce zarizeni zavrel kanal.
    DevmgrGone,
}

/// Faze sluzby v registru.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    /// Nova instance ceka na potvrzeni startu do `deadline`.
    Starting { deadline: u64 },
    /// Po padu ceka na prodlevu.
    Backoff { at: u64 },
    /// Ceka, az pobezi zavislosti (`Dep::restart`).
    WaitingDeps,
    GaveUp,
    /// Hlidana sluzba bez restartu skoncila.
    Stopped,
}

/// Stav sluzby pro prehled uloh a testy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceStatus {
    pub name: &'static str,
    pub state: State,
    pub generation: u64,
    /// Restartu od startu systemu.
    pub restarts: u32,
    /// Kod posledniho konce instance.
    pub last_exit: Option<i64>,
    /// Proc to supervisor vzdal (nebo proc sluzba ceka).
    pub reason: Option<&'static str>,
}

struct Entry {
    name: &'static str,
    deps: &'static [Dep],
    service: Option<Box<dyn Service>>,
    /// Proces bezici instance (`None` = nebezi).
    process: Option<Handle>,
    /// Kanal potvrzeni startu (ve fazi `Starting`).
    ready: Option<Handle>,
    generation: u64,
    phase: Phase,
    /// Casy restartu v okne [`RESTART_WINDOW`].
    history: Vec<u64>,
    restarts: u32,
    last_exit: Option<i64>,
    reason: Option<&'static str>,
    /// Instance se ukoncuje kvuli padu zavislosti (konec se nepocita jako
    /// pad sluzby).
    stopping_for: Option<&'static str>,
}

impl Entry {
    fn kill(&self) {
        if let Some(p) = &self.process {
            let _ = libos::sys::process_kill(p.raw());
        }
    }

    fn state(&self) -> State {
        match self.phase {
            Phase::Running => State::Running,
            Phase::Starting { .. } | Phase::Backoff { .. } | Phase::WaitingDeps => State::Restarting,
            Phase::GaveUp => State::GaveUp,
            Phase::Stopped => State::Stopped,
        }
    }
}

pub struct Supervisor {
    entries: Vec<Entry>,
    devmgr_open: bool,
    start_timeout: Duration,
    /// Port listku (vytvori se s prvnim listkem).
    port: Option<Port>,
    leases: BTreeMap<u64, leases::Lease>,
    next_lease: u64,
}

/// Supervisor initu (testy ho ridi pres [`Supervisor::step`]).
pub static SUPERVISOR: spin::Mutex<Supervisor> = spin::Mutex::new(Supervisor::new());

enum Item {
    Devmgr,
    Service(usize),
    /// Kanal sluzby, ktera ceka na potvrzeni startu.
    Starting(usize),
    Leases,
    Terminal(crate::terminal::Wait),
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
            devmgr_open: true,
            start_timeout: START_TIMEOUT,
            port: None,
            leases: BTreeMap::new(),
            next_lease: 1,
        }
    }

    /// Samostatny supervisor pro testy: jina lhuta na potvrzeni startu
    /// a bez spravce zarizeni (zpravy devmgr zustanou hlavnimu).
    pub const fn for_tests(start_timeout: Duration) -> Self {
        let mut s = Self::new();
        s.devmgr_open = false;
        s.start_timeout = start_timeout;
        s
    }

    /// Hlidat bezici proces `process` sluzby `name`; `restart` = jak ji
    /// spustit znovu (`None` = jen logovat pad). Bez zavislosti a relaci.
    pub fn watch(&mut self, name: &'static str, process: &libos::Process, restart: Option<Restart>) {
        let service = restart.map(|r| Box::new(r) as Box<dyn Service>);
        self.register(
            Spec {
                name,
                deps: &[],
                service,
            },
            Some(process),
        );
    }

    /// Prida sluzbu do registru. `process` = uz bezici instance (generace
    /// 1, start systemu); `None` = supervisor ji spusti sam v pristim kole
    /// (az pobezi zavislosti).
    pub fn register(&mut self, spec: Spec, process: Option<&libos::Process>) {
        if self.find(spec.name).is_some() {
            println!("init: sluzba {} uz je v registru", spec.name);
            return;
        }
        let (process, phase, generation) = match process {
            Some(p) => match p.handle().duplicate(None) {
                Ok(h) => (Some(h), Phase::Running, 1),
                Err(e) => {
                    println!("init: sluzbu {} nelze hlidat: {e}", spec.name);
                    return;
                }
            },
            None => (None, Phase::WaitingDeps, 0),
        };
        self.entries.push(Entry {
            name: spec.name,
            deps: spec.deps,
            service: spec.service,
            process,
            ready: None,
            generation,
            phase,
            history: Vec::new(),
            restarts: 0,
            last_exit: None,
            reason: None,
            stopping_for: None,
        });
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    /// Smycka do nekonecna (system po startu).
    pub fn run(&mut self, startup: &Startup) -> ! {
        println!("init: supervisor bezi");
        loop {
            self.step(startup, None);
        }
    }

    /// Jedno kolo: pocka na udalost (nejdyl do `deadline`, nebo do
    /// naplanovaneho restartu, lhuty potvrzeni ci listku) a zpracuje vse,
    /// co prislo.
    pub fn step(&mut self, startup: &Startup, deadline: Option<u64>) -> Vec<Happened> {
        let mut out = Vec::new();
        // Sluzby, ktere uz mohou startovat (zavislosti bezi), nebo nove
        // registrovane bez procesu.
        self.restart_due(startup, &mut out);
        let devices = system::devices();
        let mut items = Vec::new();
        let mut what = Vec::new();
        if self.devmgr_open
            && let Some(d) = devices
        {
            items.push((d.control.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            what.push(Item::Devmgr);
        }
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(h) = &e.process {
                items.push((h, Signals::TERMINATED));
                what.push(Item::Service(i));
            }
        }
        for (i, e) in self.entries.iter().enumerate() {
            if let Some(h) = &e.ready {
                items.push((h, Signals::READABLE | Signals::PEER_CLOSED));
                what.push(Item::Starting(i));
            }
        }
        if let Some(p) = &self.port {
            items.push((p.handle(), Signals::READABLE));
            what.push(Item::Leases);
        }
        let terminals = crate::terminal::TERMINALS.lock();
        let mut term_what = Vec::new();
        terminals.wait_items(&mut items, &mut term_what);
        what.extend(term_what.into_iter().map(Item::Terminal));
        let wake = [deadline, self.next_due(), self.leases_due()]
            .into_iter()
            .flatten()
            .min();
        let observed = match libos::wait_many(&items, wake) {
            Ok((_, o)) => o,
            Err(_) => Vec::new(),
        };
        drop(items);
        let mut terminals = terminals;
        for (k, item) in what.iter().enumerate() {
            let Some(sig) = observed.get(k).copied() else { break };
            match *item {
                Item::Devmgr => {
                    if sig.intersects(Signals::READABLE) {
                        self.devmgr_messages(&mut out);
                    } else if sig.intersects(Signals::PEER_CLOSED) {
                        println!("init: spravce zarizeni skoncil, hotplug nefunguje");
                        self.devmgr_open = false;
                        out.push(Happened::DevmgrGone);
                    }
                }
                Item::Service(i) if sig.intersects(Signals::TERMINATED) => self.exited(i, &mut out),
                Item::Starting(i) if sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) => {
                    self.started(i, &mut out)
                }
                Item::Leases if sig.intersects(Signals::READABLE) => self.lease_packets(),
                Item::Terminal(w) => terminals.on_signal(startup, w, sig),
                Item::Service(_) | Item::Starting(_) | Item::Leases => {}
            }
        }
        terminals.cleanup();
        // Kompozitor spadl: terminal v zalozni konzoli.
        if out.iter().any(|h| matches!(h, Happened::Exited { name: "compd", .. }))
            && !terminals.console_running()
            && let Err(e) = terminals.start_console(startup)
        {
            println!("init: zalozni konzoli nelze spustit: {e}");
        }
        drop(terminals);
        self.start_timeouts();
        self.restart_due(startup, &mut out);
        self.serve_pending(&mut out);
        out
    }

    /// Zpravy spravce zarizeni po startu: zarizeni pripojena za behu.
    fn devmgr_messages(&mut self, out: &mut Vec<Happened>) {
        let Some(d) = system::devices() else { return };
        while let Ok(m) = d.control.try_read() {
            match &m.data[..] {
                b"input" => {
                    let Some(input) = &d.input else { continue };
                    for h in m.handles {
                        match input.command(inputproto::control::DEVICE, alloc::vec![h]) {
                            Ok(_) => out.push(Happened::DeviceAdded),
                            Err(e) => println!("init: zarizeni pro sluzbu vstupu: {e}"),
                        }
                    }
                }
                b"block" => println!("init: disk pripojeny za behu zatim nepouzivam"),
                other => println!("init: neznama zprava od devmgr: {:?}", other),
            }
        }
    }

    /// Ceka sluzba po restartu na potvrzeni startu?
    pub fn is_starting(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.name == name && matches!(e.phase, Phase::Starting { .. }))
    }

    /// Bezi sluzba (proces existuje: bezi, nebo ceka na potvrzeni)?
    pub fn is_running(&self, name: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.name == name && e.process.is_some())
    }

    /// Generace bezici (nebo posledni) instance sluzby.
    pub fn generation(&self, name: &str) -> Option<u64> {
        self.find(name).map(|i| self.entries[i].generation)
    }

    /// Handle procesu bezici instance (testy: ukonceni sluzby).
    pub fn process(&self, name: &str) -> Option<&Handle> {
        self.find(name).and_then(|i| self.entries[i].process.as_ref())
    }

    /// Stav vsech sluzeb (prehled uloh, testy).
    pub fn status(&self) -> Vec<ServiceStatus> {
        self.entries
            .iter()
            .map(|e| ServiceStatus {
                name: e.name,
                state: e.state(),
                generation: e.generation,
                restarts: e.restarts,
                last_exit: e.last_exit,
                reason: e.reason,
            })
            .collect()
    }

    /// Stav jedne sluzby.
    pub fn status_of(&self, name: &str) -> Option<ServiceStatus> {
        self.status().into_iter().find(|s| s.name == name)
    }
}
