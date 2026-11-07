//! Supervisor: smycka udalosti initu po startu systemu.
//!
//! Hlida systemove sluzby a spravce zarizeni:
//! - **Zarizeni pripojena za behu**: zpravy spravce zarizeni `input`
//!   preda sluzbe vstupu (`device`). Disky za behu (`block`) zatim jen
//!   zaloguje.
//! - **Konec sluzby**: zaloguje kod. Sluzbu, kterou jde spustit znovu
//!   samostatne (`timed`), spusti znovu s rostouci prodlevou (100 ms,
//!   200 ms, ... nejvys 5 s). Na jeji potvrzeni neceka blokujici: kanal
//!   nove sluzby je dalsi polozka smycky; kdyz do [`START_TIMEOUT`]
//!   neodpovi, ukonci ji a pocita to jako pad. Kdyz spadne vic nez
//!   [`RESTART_MAX`]-krat za [`RESTART_WINDOW`], vzda to. U ostatnich sluzeb (`devmgr`, `fsd`,
//!   `inputd`) jen zaloguje pad: obnova potrebuje znovu napojit ovladace
//!   a je popsana jako dalsi krok v dokumentaci.
//!
//! Jedno kolo smycky je [`Supervisor::step`] (testy ji tak mohou ridit
//! a videt, co se stalo); [`Supervisor::run`] ho opakuje do nekonecna.

use alloc::vec::Vec;
use core::time::Duration;

use libos::{Handle, Signals, Startup, println, time};

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

/// Jak sluzbu spustit znovu.
#[derive(Clone, Copy)]
pub struct Restart {
    /// Spusti novy proces (neceka na nej).
    pub start: fn(&Startup) -> libos::Result<Started>,
    /// Kanal z [`Started::ready`] je citelny nebo zavreny: dokonci start
    /// (precte potvrzeni). Chyba = sluzba se ukonci a pocita se jako pad.
    pub ready: Option<fn() -> libos::Result<()>>,
}

/// Nove spustena sluzba.
pub struct Started {
    /// Proces (supervisor ceka na jeho konec).
    pub process: Handle,
    /// Kanal, na kterem sluzba potvrdi start (`None` = bezi hned).
    pub ready: Option<Handle>,
}

/// Co se v kole stalo (pro testy a log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Happened {
    /// Zarizeni pripojene za behu predane sluzbe vstupu.
    DeviceAdded,
    /// Sluzba skoncila s kodem.
    Exited { name: &'static str, code: i64 },
    Restarted(&'static str),
    /// Prilis mnoho padu: sluzba zustane zastavena.
    GaveUp(&'static str),
    /// Spravce zarizeni zavrel kanal.
    DevmgrGone,
}

impl Watched {
    fn kill(&self) {
        if let Some(p) = &self.process {
            let _ = libos::sys::process_kill(p.raw());
        }
    }
}

struct Watched {
    name: &'static str,
    /// Proces, na jehoz konec se ceka (`None` = nebezi).
    process: Option<Handle>,
    restart: Option<Restart>,
    /// Casy restartu v okne [`RESTART_WINDOW`].
    history: Vec<u64>,
    restart_at: Option<u64>,
    /// Sluzba po restartu ceka na potvrzeni: kanal a nejzazsi cas.
    starting: Option<(Handle, u64)>,
    given_up: bool,
}

pub struct Supervisor {
    watched: Vec<Watched>,
    devmgr_open: bool,
    start_timeout: Duration,
}

/// Supervisor initu (testy ho ridi pres [`Supervisor::step`]).
pub static SUPERVISOR: spin::Mutex<Supervisor> = spin::Mutex::new(Supervisor::new());

enum Item {
    Devmgr,
    Service(usize),
    /// Kanal sluzby, ktera ceka na potvrzeni startu.
    Starting(usize),
}

impl Supervisor {
    pub const fn new() -> Self {
        Self {
            watched: Vec::new(),
            devmgr_open: true,
            start_timeout: START_TIMEOUT,
        }
    }

    /// Samostatny supervisor pro testy: jina lhuta na potvrzeni startu
    /// a bez spravce zarizeni (zpravy devmgr zustanou hlavnimu).
    pub const fn for_tests(start_timeout: Duration) -> Self {
        Self {
            watched: Vec::new(),
            devmgr_open: false,
            start_timeout,
        }
    }

    /// Hlidat proces `process` sluzby `name`; `restart` = jak ji spustit
    /// znovu (`None` = jen logovat pad).
    pub fn watch(&mut self, name: &'static str, process: &libos::Process, restart: Option<Restart>) {
        match process.handle().duplicate(None) {
            Ok(h) => self.watched.push(Watched {
                name,
                process: Some(h),
                restart,
                history: Vec::new(),
                restart_at: None,
                starting: None,
                given_up: false,
            }),
            Err(e) => println!("init: sluzbu {name} nelze hlidat: {e}"),
        }
    }

    /// Smycka do nekonecna (system po startu).
    pub fn run(&mut self, startup: &Startup) -> ! {
        println!("init: supervisor bezi");
        loop {
            self.step(startup, None);
        }
    }

    /// Jedno kolo: pocka na udalost (nejdyl do `deadline`, nebo do
    /// naplanovaneho restartu) a zpracuje vse, co prislo.
    pub fn step(&mut self, startup: &Startup, deadline: Option<u64>) -> Vec<Happened> {
        let mut out = Vec::new();
        let devices = system::devices();
        let mut items = Vec::new();
        let mut what = Vec::new();
        if self.devmgr_open
            && let Some(d) = devices
        {
            items.push((d.control.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            what.push(Item::Devmgr);
        }
        for (i, w) in self.watched.iter().enumerate() {
            if let Some(h) = &w.process {
                items.push((h, Signals::TERMINATED));
                what.push(Item::Service(i));
            }
        }
        for (i, w) in self.watched.iter().enumerate() {
            if let Some((h, _)) = &w.starting {
                items.push((h, Signals::READABLE | Signals::PEER_CLOSED));
                what.push(Item::Starting(i));
            }
        }
        let due = self
            .watched
            .iter()
            .flat_map(|w| [w.restart_at, w.starting.as_ref().map(|s| s.1)])
            .flatten()
            .min();
        let wake = match (deadline, due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let observed = match libos::wait_many(&items, wake) {
            Ok((_, o)) => o,
            Err(_) => Vec::new(),
        };
        drop(items);
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
                Item::Service(_) | Item::Starting(_) => {}
            }
        }
        self.start_timeouts();
        self.restart_due(startup, &mut out);
        out
    }

    /// Sluzba po restartu poslala potvrzeni (nebo zavrela kanal).
    fn started(&mut self, i: usize, out: &mut Vec<Happened>) {
        let w = &mut self.watched[i];
        // Mezitim skoncila (zpracovano v `exited`).
        if w.starting.take().is_none() {
            return;
        }
        let ready = w.restart.and_then(|r| r.ready);
        match ready.map_or(Ok(()), |f| f()) {
            Ok(()) => {
                println!("init: sluzba {} znovu spustena", w.name);
                out.push(Happened::Restarted(w.name));
            }
            Err(e) => {
                // Ukoncit: konec procesu pak zpracuje `exited` jako pad.
                println!("init: sluzba {} nepotvrdila start: {e}", w.name);
                w.kill();
            }
        }
    }

    /// Sluzby, ktere do lhuty nepotvrdily start, ukonci (konec procesu
    /// zpracuje `exited` jako pad).
    fn start_timeouts(&mut self) {
        let now = time::now();
        for w in self.watched.iter_mut() {
            if w.starting.as_ref().is_some_and(|s| s.1 <= now) {
                w.starting = None;
                println!("init: sluzba {} neodpovedela, ukoncuji ji", w.name);
                w.kill();
            }
        }
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

    fn exited(&mut self, i: usize, out: &mut Vec<Happened>) {
        let w = &mut self.watched[i];
        let Some(h) = w.process.take() else { return };
        w.starting = None;
        let code = libos::sys::process_exit_code(h.raw()).unwrap_or(i64::MIN);
        println!(
            "init: sluzba {} skoncila: {}",
            w.name,
            libos::abi::process::describe_exit(code)
        );
        out.push(Happened::Exited { name: w.name, code });
        if w.restart.is_none() {
            println!("init: sluzbu {} zatim neumim obnovit", w.name);
            return;
        }
        let now = time::now();
        let window = RESTART_WINDOW.as_nanos() as u64;
        w.history.retain(|&t| now.saturating_sub(t) < window);
        if w.history.len() >= RESTART_MAX {
            w.given_up = true;
            println!(
                "init: sluzba {} spadla {}x za {} s, vzdavam to",
                w.name,
                RESTART_MAX + 1,
                RESTART_WINDOW.as_secs()
            );
            out.push(Happened::GaveUp(w.name));
            return;
        }
        let delay = (RESTART_DELAY.as_nanos() as u64)
            .saturating_mul(1 << w.history.len())
            .min(RESTART_DELAY_MAX.as_nanos() as u64);
        w.restart_at = Some(now.saturating_add(delay));
    }

    fn restart_due(&mut self, startup: &Startup, out: &mut Vec<Happened>) {
        let now = time::now();
        for w in self.watched.iter_mut() {
            let (Some(at), Some(restart)) = (w.restart_at, w.restart) else {
                continue;
            };
            if at > now {
                continue;
            }
            w.restart_at = None;
            w.history.push(now);
            match (restart.start)(startup) {
                Ok(s) => {
                    w.process = Some(s.process);
                    match s.ready {
                        Some(ch) => {
                            let deadline =
                                now.saturating_add(self.start_timeout.as_nanos() as u64);
                            w.starting = Some((ch, deadline));
                        }
                        None => {
                            println!("init: sluzba {} znovu spustena", w.name);
                            out.push(Happened::Restarted(w.name));
                        }
                    }
                }
                Err(e) => {
                    // Start selhal: pocita se jako dalsi pad.
                    println!("init: sluzbu {} nelze spustit: {e}", w.name);
                    if w.history.len() >= RESTART_MAX {
                        w.given_up = true;
                        out.push(Happened::GaveUp(w.name));
                    } else {
                        w.restart_at = Some(now.saturating_add(RESTART_DELAY_MAX.as_nanos() as u64));
                    }
                }
            }
        }
    }

    /// Ceka sluzba po restartu na potvrzeni startu?
    pub fn is_starting(&self, name: &str) -> bool {
        self.watched
            .iter()
            .any(|w| w.name == name && w.starting.is_some())
    }

    /// Sluzba, ktera to vzdala, dostane novou sanci (rucni zasah spravce
    /// systemu; testy).
    pub fn revive(&mut self, name: &str) -> bool {
        match self.watched.iter_mut().find(|w| w.name == name && w.given_up) {
            Some(w) => {
                w.given_up = false;
                w.history.clear();
                w.restart_at = Some(time::now());
                true
            }
            None => false,
        }
    }

    /// Bezi sluzba (neni po padu ani ji supervisor nevzdal)?
    pub fn is_running(&self, name: &str) -> bool {
        self.watched
            .iter()
            .any(|w| w.name == name && w.process.is_some())
    }
}
