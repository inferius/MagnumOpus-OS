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
use term::taskboard::Chan;
use term::tasks::{Clock, Joinable, Notify};

mod audio;
mod ai;
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
    /// Kanaly prehledu uloh (`SessMsg::Tasks`), predaji se `SysEnv`.
    tasks: Option<Vec<libos::Handle>>,
}

/// Kanal k initu pro `shutdown`/`reboot` (zprava `SessMsg::Power` od termd).
static POWER: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);
/// Kanal k instalatoru (`install`), ziskany od initu pri prvnim pouziti.
static INSTALLER: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// Relace zvukove sluzby (zprava `SessMsg::Audio` od termd: kanal a listek);
/// `SysEnv` si ji vezme pri prvnim prikazu `audio`.
static AUDIO: spin::Mutex<Option<audioproto::conn::AudioClient>> = spin::Mutex::new(None);

impl MainLink<'_> {
    fn client(&mut self, r: Result<libos::Message, Error>) -> Option<ClientMsg> {
        match r {
            Ok(m) => match SessMsg::decode(&m.data) {
                Ok(SessMsg::Client(c)) => Some(c),
                Ok(SessMsg::Tasks) => {
                    self.tasks = Some(m.handles);
                    None
                }
                Ok(SessMsg::Power) => {
                    if let Some(h) = m.handles.into_iter().next() {
                        *POWER.lock() = Some(Channel::from_handle(h));
                    }
                    None
                }
                Ok(SessMsg::Audio) => {
                    let mut hs = m.handles.into_iter().map(Channel::from_handle);
                    if let Some(ch) = hs.next() {
                        let c = match hs.next() {
                            Some(lease) => audioproto::conn::AudioClient::with_lease(ch, lease),
                            None => audioproto::conn::AudioClient::new(ch),
                        };
                        *AUDIO.lock() = Some(c);
                    }
                    None
                }
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
        if !self.open {
            return;
        }
        let mut data = msg.encode();
        // Prilis velka zprava (obri vystup): misto ni chyba, spojeni s oknem
        // zustava (driv se relace odpojila a blok "bezel" navzdy).
        if data.len() > libos::abi::syscall::CHANNEL_MAX_DATA {
            let note = alloc::format!("výstup je příliš velký pro okno ({} B); uložte ho: ... | save soubor", data.len());
            data = ServerMsg::Fail { message: note }.encode();
        }
        // Plna fronta (okno nestiha): chvili zkouset, zpravu (treba konec
        // bloku) neztratit. Odpojit jen kdyz okno opravdu zmizelo.
        let end = libos::time::deadline_after(core::time::Duration::from_secs(2));
        loop {
            match self.chan.write(&data, Vec::new()) {
                Err(libos::Error::ShouldWait | libos::Error::NoResources) if libos::time::now() < end => {
                    libos::time::sleep(core::time::Duration::from_millis(2));
                }
                Err(libos::Error::PeerClosed) => {
                    self.open = false;
                    return;
                }
                _ => return,
            }
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
    /// Zvukova sluzba (`None` = relace ji nedostala) a prave prehravany
    /// stream prikazu `audio`.
    audio: Option<audioproto::conn::AudioClient>,
    playing: Option<audioproto::conn::Stream>,
    /// Prikaz `ai` (relace agenta AI).
    ai: ai::Ai,
    home: String,
    /// Prehled uloh (`taskd`): kanal producenta a klienta.
    task_producer: Option<Channel>,
    task_client: Option<Channel>,
}

impl SysEnv {
    fn task_chan(&self, c: Chan) -> Option<&Channel> {
        match c {
            Chan::Producer => self.task_producer.as_ref(),
            Chan::Client => self.task_client.as_ref(),
        }
    }
}

impl Env for SysEnv {
    fn audio(&mut self, req: shlang::host::AudioReq) -> Option<Result<shlang::value::Value, shlang::ShError>> {
        if self.audio.is_none() {
            self.audio = AUDIO.lock().take();
        }
        let client = self.audio.as_ref()?;
        Some(audio::handle(client, &mut self.playing, req))
    }

    fn now(&mut self) -> Option<i64> {
        wall_ns()
    }

    fn sleep(&mut self, ns: i64) {
        sleep_ns(ns)
    }

    fn exec(&mut self, name: &str, args: &[(Option<String>, shlang::Value)], _input: shlang::Value, _needs: &shlang::host::ExecNeeds)
        -> Option<Result<shlang::Value, shlang::ShError>> {
        if name != "ai" {
            return None;
        }
        let c = self.spawner.as_ref().and_then(|s| s.connect().ok());
        let mut f: Box<dyn Fs> = match c {
            Some(c) => Box::new(fs::FsdFs::new(c, alloc::format!("{}-ai", self.tag))),
            None => Box::new(fs::NoFs),
        };
        let home = self.home.clone();
        Some(self.ai.run(args, f.as_mut(), &home))
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

    fn task_send(&mut self, to: Chan, data: Vec<u8>) -> bool {
        match self.task_chan(to) {
            Some(c) => c.write(&data, Vec::new()).is_ok(),
            None => false,
        }
    }

    fn task_recv(&mut self) -> Vec<(Chan, Vec<u8>)> {
        let mut out = Vec::new();
        for c in [Chan::Producer, Chan::Client] {
            let Some(ch) = self.task_chan(c) else { continue };
            loop {
                match ch.try_read() {
                    Ok(m) => out.push((c, m.data)),
                    Err(Error::ShouldWait) => break,
                    Err(_) => {
                        // taskd skoncil: bez prehledu uloh dal.
                        match c {
                            Chan::Producer => self.task_producer = None,
                            Chan::Client => self.task_client = None,
                        }
                        break;
                    }
                }
            }
        }
        out
    }

    /// `shutdown`/`reboot`: pozadavek initu (ten ukonci sluzby a pozada
    /// acpid). Uspech = pocitac se vypne; odpoved prijde jen pri chybe.
    fn power(&mut self, reboot: bool) -> Result<(), Error> {
        let guard = POWER.lock();
        let chan = guard.as_ref().ok_or(Error::NotSupported)?;
        let req = if reboot { powerproto::Request::Reboot } else { powerproto::Request::Shutdown };
        chan.write(&req.encode(), Vec::new())?;
        match chan.read(Some(time::deadline_after(Duration::from_secs(60)))) {
            Ok(m) => match powerproto::Reply::decode(&m.data) {
                Ok(powerproto::Reply::Failed(e)) => Err(e),
                Ok(_) => Ok(()),
                Err(e) => Err(e),
            },
            // Init ukoncuje system (kanal zavren): vypnuti probiha.
            Err(Error::PeerClosed) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `loglevel`: uroven logu jadra (pres init, ten ma pravo DIAG).
    fn log_level(&mut self, target: &str, level: &str) -> Result<String, Error> {
        let guard = POWER.lock();
        let chan = guard.as_ref().ok_or(Error::NotSupported)?;
        let req = powerproto::Request::LogLevel { target: String::from(target), level: String::from(level) };
        chan.write(&req.encode(), Vec::new())?;
        let m = chan.read(Some(time::deadline_after(Duration::from_secs(10))))?;
        match powerproto::Reply::decode(&m.data)? {
            powerproto::Reply::Text(t) => Ok(t),
            powerproto::Reply::Failed(e) => Err(e),
            _ => Err(Error::Corrupted),
        }
    }

    /// `install`: kanal k instalatoru (od initu, jednou), pozadavek,
    /// zpravy prubehu, konecna odpoved.
    fn installer(&mut self, req: &[u8], progress: &mut dyn FnMut(&[u8])) -> Result<Vec<u8>, Error> {
        let mut inst = INSTALLER.lock();
        if inst.is_none() {
            let guard = POWER.lock();
            let chan = guard.as_ref().ok_or(Error::NotSupported)?;
            chan.write(&powerproto::Request::OpenInstaller.encode(), Vec::new())?;
            let m = chan.read(Some(time::deadline_after(Duration::from_secs(30))))?;
            match powerproto::Reply::decode(&m.data)? {
                powerproto::Reply::Ok => {}
                powerproto::Reply::Failed(e) => return Err(e),
                _ => return Err(Error::Corrupted),
            }
            let h = m.handles.into_iter().next().ok_or(Error::Corrupted)?;
            *inst = Some(Channel::from_handle(h));
        }
        let ch = inst.as_ref().unwrap();
        ch.write(req, Vec::new())?;
        loop {
            // Instalace trva minuty: bez terminu, prubeh chodi prubezne.
            let m = ch.read(None)?;
            if matches!(installplan::proto::Reply::decode(&m.data), Ok(installplan::proto::Reply::Progress { .. })) {
                progress(&m.data);
                continue;
            }
            return Ok(m.data);
        }
    }

    /// `report`: balicek pro vyvojare sestavi sluzba logu (pres init).
    fn report(&mut self) -> Result<String, Error> {
        let guard = POWER.lock();
        let chan = guard.as_ref().ok_or(Error::NotSupported)?;
        chan.write(&powerproto::Request::Report.encode(), Vec::new())?;
        let m = chan.read(Some(time::deadline_after(Duration::from_secs(90))))?;
        match powerproto::Reply::decode(&m.data)? {
            powerproto::Reply::Text(t) => Ok(alloc::format!("balicek pro vyvojare: {t}")),
            powerproto::Reply::Failed(e) => Err(e),
            _ => Err(Error::Corrupted),
        }
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
    let agent_gate = hs.next().map(Channel::from_handle);
    let mut nofs = fs::NoFs;
    let mut svc = Service::single(&home, session);
    svc.stack_budget = STACK_BUDGET;
    svc.task_stack = TASK_STACK_BUDGET;
    // Radky ve vlaknech: okno obsluhovane i behem prikazu (Ctrl+C, Ctrl+Z,
    // fronta radku, `&` na pozadi).
    svc.line_threads = true;
    let notify = Event::create().ok().map(|e| Arc::new(EventNotify(e)));
    let spawner = fsd.as_ref().and_then(|f| f.client.connect().ok());
    let mut env = SysEnv {
        spawner,
        tag: tag.clone(),
        notify: notify.clone(),
        ai: ai::Ai::new(agent_gate),
        home: home.clone(),
        audio: None,
        playing: None,
        task_producer: None,
        task_client: None,
    };
    println!("shsess {session}: bezim ({tag})");

    loop {
        // Hlavni smycka: zpravy okna pres termd.
        let mut link = MainLink { chan: &main_chan, open: true, tasks: None };
        while link.open {
            // Kanaly prehledu uloh prisly (termd je pozadal u initu).
            if let Some(hs) = link.tasks.take() {
                let mut hs = hs.into_iter().map(Channel::from_handle);
                env.task_producer = hs.next();
                env.task_client = hs.next();
            }
            // Zprava okna, uloha neco poslala (udalost), zprava prehledu
            // uloh, nebo tik.
            if let Some(n) = notify.as_ref() {
                let rd = Signals::READABLE | Signals::PEER_CLOSED;
                let mut items: Vec<(&libos::Handle, Signals)> =
                    alloc::vec![(main_chan.handle(), rd), (n.0.handle(), Signals::SIGNALED)];
                for c in [env.task_producer.as_ref(), env.task_client.as_ref()].into_iter().flatten() {
                    items.push((c.handle(), rd));
                }
                let woke = libos::wait_many(&items, tick());
                drop(items);
                if matches!(woke, Ok((i, _)) if i >= 1) || matches!(woke, Err(Error::TimedOut)) {
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
            let mut dead = MainLink { chan: &main_chan, open: false, tasks: None };
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
