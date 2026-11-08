//! shsess: proces jedne relace shellu (interpret, koncept radku, undo).
//!
//! Spousti ho termd pro kazdou relaci. Bootstrap kanal = hlavni kanal
//! k termd; prvni zprava `SessMsg::Init` nese cislo relace, znacku a dva
//! handly: nahradni kanal (druhy konec drzi registr relaci) a vlastni
//! relaci fsd.
//!
//! Kdyz termd skonci nebo spadne (hlavni kanal zavren), relace bezi dal:
//! dobehne prikaz i ulohy na pozadi a ceka na nahradnim kanalu na `Adopt`
//! od noveho termd. Kdyz zavre i registr (nikdo ji uz neprevezme) a zadna
//! uloha neceka, skonci.
//!
//! Ulohy `async` bezi ve vlaknech tohoto procesu (vlastni interpret
//! a vlastni relace fsd se stejnou identitou); hlavni vlakno ceka na
//! zpravu okna nebo na udalost od uloh (zadne aktivni cekani).

#![no_std]
#![no_main]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use libos::{Channel, Error, Event, Signals, Startup, println, time};
use shlang::host::Fs;
use term::ctl::SessMsg;
use term::proto::{ClientMsg, ServerMsg};
use term::session::{Env, Link, Service};
use term::tasks::{Clock, Joinable, Notify};

mod audio;
mod fs;

libos::entry!(main);

/// Zasobnik programu je 256 KB; interpret smi pouzit nejvys tolik.
const STACK_BUDGET: usize = 160 * 1024;
/// Zasobnik vlakna ulohy je take 256 KB (`libos::thread::STACK_SIZE`).
const TASK_STACK_BUDGET: usize = 160 * 1024;

/// Spojeni s termd (hlavni kanal). Neplatna zprava se zahodi.
struct MainLink<'a> {
    chan: &'a Channel,
    open: bool,
}

impl MainLink<'_> {
    fn client(&mut self, r: Result<libos::Message, Error>) -> Option<ClientMsg> {
        match r {
            Ok(m) => match SessMsg::decode(&m.data) {
                Ok(SessMsg::Client(c)) => Some(c),
                _ => None,
            },
            Err(Error::ShouldWait) | Err(Error::TimedOut) => None,
            Err(_) => {
                self.open = false;
                None
            }
        }
    }
}

impl Link for MainLink<'_> {
    fn send(&mut self, msg: &ServerMsg) {
        if self.open && self.chan.write(&msg.encode(), Vec::new()).is_err() {
            self.open = false;
        }
    }

    fn poll(&mut self) -> Option<ClientMsg> {
        if !self.open {
            return None;
        }
        let r = self.chan.try_read();
        self.client(r)
    }

    fn wait(&mut self) -> Option<ClientMsg> {
        if !self.open {
            return None;
        }
        let r = self.chan.read(None);
        self.client(r)
    }
}

fn wall_ns() -> Option<i64> {
    time::wall().map(|t| t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64))
}

fn sleep_ns(ns: i64) {
    if ns > 0 {
        time::sleep(Duration::from_nanos(ns as u64));
    }
}

/// Probouzeni hlavniho vlakna udalosti jadra.
struct EventNotify(Event);

impl Notify for EventNotify {
    fn notify(&self) {
        let _ = self.0.signal();
    }

    fn wait(&self, ns: i64) {
        let deadline = time::deadline_after(Duration::from_nanos(ns.max(0) as u64));
        if libos::wait_one(self.0.handle(), Signals::SIGNALED, Some(deadline)).is_ok() {
            let _ = self.0.reset();
        }
    }
}

struct ThreadJoin(libos::thread::JoinHandle);

impl Joinable for ThreadJoin {
    fn join(self: Box<Self>) {
        let _ = self.0.join(None);
    }
}

struct SysEnv {
    /// Relace fsd, ze ktere vznikaji relace uloh (stejna identita a cesty).
    spawner: Option<FsClient>,
    tag: String,
    notify: Option<Arc<EventNotify>>,
    /// Zvukova sluzba (`None` = relace ji nedostala).
    audio: Option<audioproto::conn::AudioClient>,
}

impl Env for SysEnv {
    fn audio(&mut self, req: shlang::host::AudioReq) -> Option<Result<shlang::value::Value, shlang::ShError>> {
        Some(audio::handle(self.audio.as_ref()?, req))
    }

    fn now(&mut self) -> Option<i64> {
        wall_ns()
    }

    fn sleep(&mut self, ns: i64) {
        sleep_ns(ns)
    }

    fn spawn(&mut self, job: Box<dyn FnOnce() + Send>) -> Option<Box<dyn Joinable>> {
        match libos::thread::spawn("shsess-task", job) {
            Ok(h) => Some(Box::new(ThreadJoin(h))),
            Err(e) => {
                println!("shsess: vlakno ulohy nejde spustit: {e}");
                None
            }
        }
    }

    fn task_fs(&mut self, task: u64) -> Option<Box<dyn Fs + Send>> {
        let c = self.spawner.as_ref()?.connect().ok()?;
        Some(Box::new(fs::FsdFs::new(c, alloc::format!("{}-t{task}", self.tag))))
    }

    fn notifier(&mut self) -> Option<Arc<dyn Notify>> {
        self.notify.clone().map(|n| n as Arc<dyn Notify>)
    }

    fn new_notifier(&mut self) -> Option<Arc<dyn Notify>> {
        Event::create().ok().map(|e| Arc::new(EventNotify(e)) as Arc<dyn Notify>)
    }

    fn clock(&self) -> Option<Clock> {
        Some(Clock { now: wall_ns, sleep: sleep_ns })
    }
}

fn tick() -> Option<u64> {
    Some(time::deadline_after(Duration::from_millis(200)))
}

fn main(startup: Startup) -> i64 {
    let mut main_chan = startup.bootstrap;
    let first = match main_chan.read(Some(time::deadline_after(Duration::from_secs(10)))) {
        Ok(m) => m,
        Err(e) => {
            println!("shsess: chybi Init: {e}");
            return 1;
        }
    };
    let Ok(SessMsg::Init { session, tag, home }) = SessMsg::decode(&first.data) else {
        println!("shsess: prvni zprava neni Init");
        return 1;
    };
    let mut hs = first.handles.into_iter();
    let mut spare = hs.next().map(Channel::from_handle);
    let mut fsd = hs.next().map(|h| fs::FsdFs::new(FsClient::new(Channel::from_handle(h)), tag.clone()));
    let audio = hs.next().map(|h| audioproto::conn::AudioClient::new(Channel::from_handle(h)));
    let mut nofs = fs::NoFs;
    let mut svc = Service::single(&home, session);
    svc.stack_budget = STACK_BUDGET;
    svc.task_stack = TASK_STACK_BUDGET;
    let notify = Event::create().ok().map(|e| Arc::new(EventNotify(e)));
    let spawner = fsd.as_ref().and_then(|f| f.client.connect().ok());
    let mut env = SysEnv { spawner, tag: tag.clone(), notify: notify.clone(), audio };
    println!("shsess {session}: bezim ({tag})");

    loop {
        // Hlavni smycka: zpravy okna pres termd.
        let mut link = MainLink { chan: &main_chan, open: true };
        while link.open {
            // Zprava okna, nebo uloha neco poslala (udalost), nebo tik.
            if let Some(n) = notify.as_ref() {
                let items = [
                    (main_chan.handle(), Signals::READABLE | Signals::PEER_CLOSED),
                    (n.0.handle(), Signals::SIGNALED),
                ];
                let woke = libos::wait_many(&items, tick());
                if matches!(woke, Ok((1, _)) | Err(Error::TimedOut)) {
                    let _ = n.0.reset();
                    let fs: &mut dyn shlang::host::Fs = match fsd.as_mut() {
                        Some(f) => f,
                        None => &mut nofs,
                    };
                    svc.idle(&mut link, fs, &mut env);
                    continue;
                }
            }
            let r = main_chan.read(tick());
            let fs: &mut dyn shlang::host::Fs = match fsd.as_mut() {
                Some(f) => f,
                None => &mut nofs,
            };
            match r {
                Err(Error::TimedOut) => svc.idle(&mut link, fs, &mut env),
                r => {
                    if let Some(m) = link.client(r) {
                        svc.handle(m, &mut link, fs, &mut env);
                    }
                }
            }
        }
        // termd je pryc: relace se odpoji, prikazy a ulohy dobehnou bez okna.
        for s in &mut svc.sessions {
            s.attached = false;
        }
        println!("shsess {session}: termd skoncil, cekam na prevzeti");
        let adopted = loop {
            let fs: &mut dyn shlang::host::Fs = match fsd.as_mut() {
                Some(f) => f,
                None => &mut nofs,
            };
            let mut dead = MainLink { chan: &main_chan, open: false };
            svc.idle(&mut dead, fs, &mut env);
            let Some(sp) = spare.as_ref() else {
                if svc.has_jobs() {
                    // Ulohy async bezi ve vlaknech: pockat na jejich udalost.
                    match notify.as_ref() {
                        Some(n) => n.wait(200_000_000),
                        None => sleep_ns(200_000_000),
                    }
                    continue;
                }
                break None;
            };
            match sp.read(tick()) {
                Ok(m) => {
                    if SessMsg::decode(&m.data) == Ok(SessMsg::Adopt) {
                        break Some(m.handles);
                    }
                }
                Err(Error::TimedOut) => {}
                Err(_) => spare = None,
            }
        };
        let Some(handles) = adopted else {
            println!("shsess {session}: nikdo neprevzal, koncim");
            return 0;
        };
        // Nahradni kanal se stava hlavnim, novy nahradni prisel ve zprave.
        main_chan = spare.take().unwrap();
        spare = handles.into_iter().next().map(Channel::from_handle);
        println!("shsess {session}: prevzato novym termd");
    }
}
