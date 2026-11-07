//! Supervisor: smycka udalosti initu po startu systemu.
//!
//! Hlida systemove sluzby a spravce zarizeni:
//! - **Zarizeni pripojena za behu**: zpravy spravce zarizeni `input`
//!   preda sluzbe vstupu (`device`). Disky za behu (`block`) zatim jen
//!   zaloguje.
//! - **Konec sluzby**: zaloguje kod. Sluzbu, kterou jde spustit znovu
//!   samostatne (`timed`), spusti znovu s rostouci prodlevou (100 ms,
//!   200 ms, ... nejvys 5 s). Kdyz spadne vic nez [`RESTART_MAX`]-krat za
//!   [`RESTART_WINDOW`], vzda to. U ostatnich sluzeb (`devmgr`, `fsd`,
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

/// Spusti sluzbu znovu; vraci handle noveho procesu (pro cekani).
pub type Restart = fn(&Startup) -> libos::Result<Handle>;

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

struct Watched {
    name: &'static str,
    /// Proces, na jehoz konec se ceka (`None` = nebezi).
    process: Option<Handle>,
    restart: Option<Restart>,
    /// Casy restartu v okne [`RESTART_WINDOW`].
    history: Vec<u64>,
    restart_at: Option<u64>,
    given_up: bool,
}

pub struct Supervisor {
    watched: Vec<Watched>,
    devmgr_open: bool,
}

/// Supervisor initu (testy ho ridi pres [`Supervisor::step`]).
pub static SUPERVISOR: spin::Mutex<Supervisor> = spin::Mutex::new(Supervisor::new());

enum Item {
    Devmgr,
    Service(usize),
}

impl Supervisor {
    pub const fn new() -> Self {
        Self {
            watched: Vec::new(),
            devmgr_open: true,
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
        let due = self.watched.iter().filter_map(|w| w.restart_at).min();
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
                Item::Service(_) => {}
            }
        }
        self.restart_due(startup, &mut out);
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

    fn exited(&mut self, i: usize, out: &mut Vec<Happened>) {
        let w = &mut self.watched[i];
        let Some(h) = w.process.take() else { return };
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
            match restart(startup) {
                Ok(h) => {
                    w.process = Some(h);
                    println!("init: sluzba {} znovu spustena", w.name);
                    out.push(Happened::Restarted(w.name));
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
