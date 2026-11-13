//! GUI (milnik 6c): start kompozitoru `compd`, predani vstupu, spousteni
//! aplikaci s oknem a obnova kompozitoru po padu.
//!
//! Identitu aplikace (id a jmeno z manifestu, co smi) posila kompozitoru
//! init, ne aplikace: odznak duvery tak nejde podvrhnout. Aplikace dostane
//! kanal sveho okna a listek k supervisoru (startovni zprava, kind USER:
//! 0 = okno, 1 = listek, 2+ = handly aplikace).
//!
//! **Obnova** (`docs/cs/obnova-sluzeb.md`): adapter [`CompdService`]
//! spusti novou instanci nad stejnym koncem kanalu k ovladaci displeje
//! (drzak koncu [`Kept`]; compd se s ovladacem nejdriv synchronizuje),
//! preda ji fokus a bezpecnou zkratku od `inputd` a z receptu (=
//! `ConnectInfo` z manifestu) vytvori relace oken. Aplikace se pripoji
//! sama pres listek (`ui::client`). Restart `inputd` (`Dep::notify`)
//! kanal fokusu nezavre (konce drzi init), kompozitor jen dostane
//! `Focus(false)` a `Focus(true)`.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use guiproto::control::{self, ConnectInfo, Status};
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Handle, Process, Result, Startup, Vmo, println, time};

use crate::apps::{self, App};
use crate::input::{self, Input};
use crate::supervisor::{Dep, Kept, Service, Spec, StartCtx, Started, Supervisor};

/// Kanal ovladace grafiky (od spravce zarizeni, nebo `fbdisplay`), dokud
/// ho nevezme kompozitor.
pub static DISPLAY: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// Bezici GUI (bezny beh).
pub static GUI: spin::Mutex<Option<Gui>> = spin::Mutex::new(None);

/// Kanal od kompozitoru se zadostmi `launch <id>` (lista, spoustec).
pub static LAUNCHER: spin::Mutex<Option<Channel>> = spin::Mutex::new(None);

/// Aplikace plochy, ktere lze spustit z listy (programy v initfs).
pub const DESKTOP_APPS: &[&str] = &["gui-term", "gui-demo", "gui-tasks"];

/// Jmeno kompozitoru v registru supervisoru.
pub const NAME: &str = "compd";
/// Zavislosti kompozitoru: o restartu `inputd` se jen dozvi (fokus zustava).
pub const DEPS: &[Dep] = &[Dep::notify("inputd")];
/// Ovladac `fbdisplay` (framebuffer z firmwaru), bezi-li.
pub static FBDISPLAY: spin::Mutex<Option<Process>> = spin::Mutex::new(None);

/// Prstenec startovniho obrazu pri predani (promile) pro prvni instanci
/// kompozitoru (`display <pasmo> splash=<promile>`); bere se jednou.
static SPLASH_HANDOFF: spin::Mutex<Option<u32>> = spin::Mutex::new(None);

const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// Prvni start kompozitoru (nacteni, prevzeti displeje, prvni snimek).
/// Ladici build pod TCG kresli prvni snimek prihlasovaci obrazovky s uctem
/// ~6 s; 10 s nestacilo (test instalace). Na HW je start pod 1 s, delsi
/// limit jen oddali zalozni konzoli, kdyz kompozitor visi.
const START_TIMEOUT: Duration = Duration::from_secs(30);
/// Odpoved na `connect` (relace okna; kompozitor odpovida hned).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Bezici instance kompozitoru, sdilena adapterem a [`Gui`].
pub struct Compd {
    pub process: Option<Process>,
    control: Option<Channel>,
    /// Velikost obrazovky.
    pub size: (u32, u32),
    /// Konec kanalu k ovladaci displeje (duplikat dostane kazda instance).
    display: Kept,
}

pub type CompdRef = Arc<spin::Mutex<Compd>>;

impl Compd {
    /// Ridici prikaz; odpoved `ok ...` -> text a handly.
    fn command(&self, data: &[u8], handles: Vec<Handle>, timeout: Duration) -> Result<(String, Vec<Handle>)> {
        let control = self.control.as_ref().ok_or(Error::ServiceUnavailable)?;
        input::drain(control);
        control.write(data, handles)?;
        let msg = control.read(Some(time::deadline_after(timeout)))?;
        parse_reply(&msg.data).map(|t| (t, msg.handles))
    }

    /// Preda kompozitoru fokus vstupu a kanal bezpecne zkratky od `inputd`
    /// (predchozi vlastnik o ne prijde).
    fn take_input(&self, input: &Input) -> Result<()> {
        let (mine, theirs) = Channel::create()?;
        input.command(inputproto::control::FOCUS, vec![theirs.into_handle()])?;
        self.command(control::INPUT.as_bytes(), vec![mine.into_handle()], REPLY_TIMEOUT)?;
        let (mine, theirs) = Channel::create()?;
        input.command(inputproto::control::SECURE, vec![theirs.into_handle()])?;
        self.command(control::SECURE.as_bytes(), vec![mine.into_handle()], REPLY_TIMEOUT)?;
        Ok(())
    }

    /// Fokus od bezici instance `inputd` (kdyz bezi).
    fn input_from_inputd(&self) {
        match input::with_input(|i| self.take_input(i)) {
            Some(Err(e)) => println!("init: vstup pro kompozitor: {e}"),
            Some(Ok(())) | None => {}
        }
    }
}

/// `ok ...` -> text, `err KOD` -> chyba.
fn parse_reply(data: &[u8]) -> Result<String> {
    let reply = core::str::from_utf8(data).map_err(|_| Error::Corrupted)?;
    if let Some(code) = reply.strip_prefix("err ") {
        return Err(libos::abi::error::ALL.iter().copied().find(|e| e.name() == code).unwrap_or(Error::Corrupted));
    }
    let rest = reply.strip_prefix("ok").ok_or(Error::Corrupted)?;
    Ok(String::from(rest.trim()))
}

/// Adapter kompozitoru pro supervisor.
///
/// Recept relace = zakodovany `ConnectInfo` (identita aplikace
/// z manifestu); zuzeni neni (okno nema co zuzit, `NOT_SUPPORTED`).
pub struct CompdService {
    shared: CompdRef,
    /// Novou instanci napojit na `inputd` (bezny beh; prvni instanci
    /// v sade testu `gui` napoji test sam).
    give_input: bool,
}

impl CompdService {
    pub fn new(shared: CompdRef, give_input: bool) -> Self {
        Self { shared, give_input }
    }
}

impl Service for CompdService {
    fn start(&mut self, ctx: &StartCtx) -> Result<Started> {
        let fs = Initfs::open(ctx.startup)?;
        let image = fs.find("compd").ok_or(Error::NotFound)?;
        let mut s = self.shared.lock();
        let display = s.display.give()?;
        let (process, control) = libos::spawn::service("compd", image, Vec::new())?;
        // Odpoved na `display` = potvrzeni startu (compd se synchronizuje
        // s ovladacem a pripoji snimek).
        // Pasmo pro hodiny v liste ze stejneho nastaveni jako sluzba casu.
        let zone = libos::time::zone::Settings::parse(crate::system::TIME_SETTINGS).map(|s| s.zone.name).ok().filter(|n| !n.is_empty()).unwrap_or("UTC");
        // Prvni start po startovnim obrazu: kompozitor na nej navaze.
        let mut splash = SPLASH_HANDOFF.lock().take().filter(|_| !ctx.restart).map(|p| alloc::format!(" {}{p}", control::SPLASH_ARG)).unwrap_or_default();
        // Bez otevrene relace bude prihlasovaci obrazovka: znak miri do avataru.
        if !splash.is_empty() && !crate::session::any_open() {
            splash.push(' ');
            splash.push_str(control::LOGIN_ARG);
        }
        control.write(alloc::format!("{} {zone}{splash}", control::DISPLAY).as_bytes(), vec![display.into_handle()])?;
        let started = Started {
            process: process.handle().duplicate(None)?,
            ready: Some(control.handle().duplicate(None)?),
        };
        s.process = Some(process);
        s.control = Some(control);
        Ok(started)
    }

    fn ready(&mut self) -> Result<bool> {
        let mut s = self.shared.lock();
        let control = s.control.as_ref().ok_or(Error::BadState)?;
        let m = match control.try_read() {
            Ok(m) => m,
            Err(Error::ShouldWait) => return Ok(false),
            Err(e) => return Err(e),
        };
        let text = parse_reply(&m.data)?;
        let mut it = text.split_whitespace().map(|v| v.parse::<u32>().unwrap_or(0));
        s.size = (it.next().unwrap_or(0), it.next().unwrap_or(0));
        println!("init: kompozitor bezi ({}x{})", s.size.0, s.size.1);
        if self.give_input {
            s.input_from_inputd();
            // Nova instance: relace a prihlasovaci obrazovka znovu.
            give_accounts(&s);
        }
        Ok(true)
    }

    fn connect(&mut self, recipe: &[u8]) -> Result<Channel> {
        // Recept musi byt platna identita (nic jineho kompozitor nedostane).
        let info = ConnectInfo::decode(recipe)?;
        let (_, mut h) = self.shared.lock().command(&info.encode(), Vec::new(), CONNECT_TIMEOUT)?;
        if h.len() != 1 {
            return Err(Error::Corrupted);
        }
        Ok(Channel::from_handle(h.remove(0)))
    }

    /// Restart `inputd`: kanal fokusu a `secure` kompozitoru zustava
    /// (konce na strane `inputd` drzi init v drzaku koncu a preda je nove
    /// instanci). Kompozitor dostane `Focus(false)`, `Focus(true)` a pusti
    /// stisknute klavesy a tazeni (`libs/comp`). Nove predani by bylo jen
    /// zbytecne `Focus(false)` navic.
    fn dependency_restarted(&mut self, dep: &str, generation: u64) {
        println!("init: {dep} bezi znovu (generace {generation}), fokus kompozitoru zustava");
    }

    fn stopped(&mut self) {
        let mut s = self.shared.lock();
        s.process = None;
        s.control = None;
    }
}

/// Kompozitoru: bezici relace (`session open ...`) a kanal `login` ke
/// sluzbe uctu (prihlasovaci obrazovka, kdyz nikdo neni prihlaseny).
fn give_accounts(s: &Compd) {
    // Jen bezny beh (plocha od `start_desktop`); testy GUI si kompozitor
    // ridi samy a prihlasovaci obrazovka by jim zakryla okna.
    if !ACCOUNTS_UI.load(core::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let list: Vec<(u64, u64, String, bool)> = crate::session::SESSIONS.lock().iter().map(|u| (u.id, u.uid, u.name.clone(), u.locked)).collect();
    for (id, uid, name, locked) in list {
        let _ = s.command(alloc::format!("{} open {id} {uid} {name}", control::SESSION).as_bytes(), Vec::new(), REPLY_TIMEOUT);
        if locked {
            let _ = s.command(alloc::format!("{} lock {id}", control::SESSION).as_bytes(), Vec::new(), REPLY_TIMEOUT);
        }
    }
    // Duveryhodny dialog pro ukony spravy (`ELEVATE`) dostane sluzba uctu.
    let dialog = s.command(control::DIALOG.as_bytes(), Vec::new(), REPLY_TIMEOUT).and_then(|(_, mut h)| h.pop().ok_or(Error::Corrupted));
    match dialog.and_then(|h| crate::accounts::give_dialog(Channel::from_handle(h))) {
        Ok(()) => {}
        Err(e) => println!("init: dialog spravy pro sluzbu uctu: {e}"),
    }
    match crate::accounts::channel("login") {
        Ok(ch) => {
            if let Err(e) = s.command(control::LOGIN.as_bytes(), vec![ch.into_handle()], REPLY_TIMEOUT) {
                println!("init: kanal login pro kompozitor: {e}");
            }
        }
        Err(e) => println!("init: prihlasovaci obrazovka bez sluzby uctu: {e}"),
    }
}

/// Plocha s ucty bezi (bezny beh): kompozitor dostava relace a `login`.
static ACCOUNTS_UI: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

/// Oznami kompozitoru zmenu relace (`session open|lock|unlock|close`).
pub fn session_event(text: &str) {
    if let Some(g) = GUI.lock().as_ref()
        && let Err(e) = g.command(alloc::format!("{} {text}", control::SESSION).as_bytes(), Vec::new())
    {
        println!("init: relace pro kompozitor: {e}");
    }
}

pub struct Gui {
    pub shared: CompdRef,
    pub apps: Vec<App>,
    /// Aplikace relaci: (relace, index v `apps`); odhlaseni je ukonci.
    pub session_apps: Vec<(u64, usize)>,
    /// Velikost obrazovky.
    pub size: (u32, u32),
}

/// Displej pro kompozitor - jedine misto, kde init rozhoduje, kdo
/// kresli na obrazovku: kanal virtio-gpu od spravce zarizeni (ma
/// prednost), pak nativni ovladac (`crate::display_native`), jinak
/// ovladac `fbdisplay` nad framebufferem z firmwaru
/// (bootstrap `FRAMEBUFFER`, init preda duplikat handlu, takze
/// `Startup::framebuffer` zustava pro zachrannou konzoli).
pub fn display(startup: &Startup) -> Result<Channel> {
    // Startovni obraz konci: framebuffer prevezme displej (bez smazani),
    // prechod do plochy dokonci prvni instance kompozitoru.
    if let Some(p) = crate::splash::finish() {
        *SPLASH_HANDOFF.lock() = Some(p);
    }
    if let Some(ch) = DISPLAY.lock().take() {
        return Ok(ch);
    }
    // Nativni ovladac (nastaveni obrazu), kdyz odpovi a vidi monitor.
    if let Some(ch) = crate::display_native::choose(startup) {
        return Ok(ch);
    }
    let fb = startup.framebuffer.as_ref().ok_or(Error::NotFound)?;
    let fb = Vmo::from_handle(fb.handle().duplicate(None)?);
    let fs = Initfs::open(startup)?;
    let image = fs.find("fbdisplay").ok_or(Error::NotFound)?;
    let (mine, theirs) = Channel::create()?;
    let (process, _boot) = libos::spawn(
        "fbdisplay",
        image,
        vec![StartHandle::user(fb.into_handle()), StartHandle::user(theirs.into_handle())],
    )?;
    println!("init: displej je framebuffer z firmwaru (fbdisplay)");
    *FBDISPLAY.lock() = Some(process);
    Ok(mine)
}

/// Bezi GUI nad framebufferem (`fbdisplay`)? Kdyz skonci, jadro
/// framebuffer uvolni a init muze spustit zachrannou konzoli.
// Pouzije start zachranne konzole (vlakno terminalu 6b).
#[allow(dead_code)]
pub fn uses_framebuffer() -> bool {
    FBDISPLAY.lock().is_some()
}

/// Spusti prvni instanci kompozitoru nad displejem z [`display`] a pocka
/// na jeji start (mimo supervisor: start systemu, testy).
pub fn start(startup: &Startup) -> Result<Gui> {
    let display = display(startup)?;
    let shared: CompdRef = Arc::new(spin::Mutex::new(Compd {
        process: None,
        control: None,
        size: (0, 0),
        display: Kept::new(display),
    }));
    let mut svc = CompdService::new(shared.clone(), false);
    let started = svc.start(&StartCtx {
        startup,
        generation: 1,
        restart: false,
    })?;
    let end = time::deadline_after(START_TIMEOUT);
    let ready = started.ready.ok_or(Error::BadState)?;
    loop {
        libos::wait_one(&ready, libos::Signals::READABLE | libos::Signals::PEER_CLOSED, Some(end))?;
        if svc.ready()? {
            break;
        }
    }
    let size = shared.lock().size;
    Ok(Gui {
        shared,
        apps: Vec::new(),
        session_apps: Vec::new(),
        size,
    })
}

/// Identita aplikace `name` z manifestu v initfs (recept relace okna).
pub fn recipe(startup: &Startup, name: &str) -> Result<ConnectInfo> {
    let fs = Initfs::open(startup)?;
    let text = fs.find(&format!("{name}.manifest.toml")).and_then(|t| core::str::from_utf8(t).ok()).unwrap_or("");
    let m = apps::manifest(&fs, name)?;
    let actions = match guiproto::action::parse_manifest(text) {
        Ok(a) => a,
        Err(e) => {
            println!("init: akce v manifestu {name}: {}", e.0);
            return Err(Error::InvalidArgs);
        }
    };
    let mut perms = Vec::new();
    if m.as_ref().is_some_and(|m| m.network.is_some()) {
        perms.push(String::from("síť"));
    }
    // Systemova aplikace (odznak "Systém"): rozhoduje init, ne aplikace.
    // Klic `[app] system` se cte jen z manifestu v initfs (soucast obrazu
    // systemu sestaveneho z repozitare). Aplikace odjinud (disk, sit) tudy
    // nevedou; az budou, o systemove aplikaci rozhodne podpis a cesta,
    // ne jejich vlastni manifest.
    let system = conf::parse(text).ok().and_then(|t| t.get_bool("app.system")).unwrap_or(false);
    Ok(ConnectInfo {
        system,
        id: m.as_ref().map_or_else(|| format!("initfs.{name}"), |m| m.id.clone()),
        name: m.as_ref().map_or_else(|| String::from(name), |m| m.name.clone()),
        perms,
        actions,
    })
}

impl Gui {
    /// Ridici prikaz bezici instanci; odpoved `ok ...` -> text a handly.
    pub fn command(&self, data: &[u8], handles: Vec<Handle>) -> Result<(String, Vec<Handle>)> {
        self.shared.lock().command(data, handles, REPLY_TIMEOUT)
    }

    /// Prikaz, ktery vraci kanal (`dialog`, `inspect`, `actions <jmeno>`,
    /// `connect`).
    pub fn channel(&self, data: &[u8]) -> Result<Channel> {
        let (_, mut h) = self.command(data, Vec::new())?;
        if h.len() != 1 {
            return Err(Error::Corrupted);
        }
        Ok(Channel::from_handle(h.remove(0)))
    }

    pub fn status(&self) -> Result<Status> {
        let (t, _) = self.command(control::STATUS.as_bytes(), Vec::new())?;
        Status::parse(&t).ok_or(Error::Corrupted)
    }

    /// Preda kompozitoru fokus vstupu a kanal bezpecne zkratky od `inputd`.
    pub fn take_input(&self, input: &Input) -> Result<()> {
        self.shared.lock().take_input(input)
    }

    /// Adapter pro supervisor nad touto instanci (`give_input`: dalsi
    /// instance napojit na `inputd`).
    pub fn service(&self, give_input: bool) -> Box<dyn Service> {
        Box::new(CompdService::new(self.shared.clone(), give_input))
    }

    /// Zaregistruje kompozitor v supervisoru (bezici instance = generace 1).
    pub fn register(&self, sup: &mut Supervisor) {
        let spec = Spec {
            name: NAME,
            deps: DEPS,
            service: Some(self.service(true)),
        };
        let s = self.shared.lock();
        sup.register(spec, s.process.as_ref());
    }

    /// Spusti aplikaci z initfs s oknem bez obnovy (testy bez
    /// supervisoru): listek je kanal bez druheho konce, takze pad
    /// kompozitoru okno ukonci.
    pub fn launch(&mut self, startup: &Startup, name: &str) -> Result<usize> {
        self.launch_extra(startup, name, Vec::new())
    }

    /// Jako [`Gui::launch`] (bez obnovy), s dalsimi handly aplikace.
    pub fn launch_extra(&mut self, startup: &Startup, name: &str, extra: Vec<StartHandle>) -> Result<usize> {
        let info = recipe(startup, name)?;
        let ch = self.channel(&info.encode())?;
        let (lease, _) = Channel::create()?;
        self.launch_with(startup, name, ch, lease, extra)
    }

    /// Spusti aplikaci relace `session` v jejim jobu s oknem, ktere
    /// prezije pad kompozitoru (relace a listek od supervisoru `sup`),
    /// s dalsimi handly aplikace za listkem (napr. kanal terminalu).
    pub fn launch_for_session(&mut self, startup: &Startup, name: &str, sup: &mut Supervisor, extra: Vec<StartHandle>, session: u64) -> Result<usize> {
        let info = recipe(startup, name)?;
        let job = crate::session::job(session)?;
        let (ch, lease) = sup.session(NAME, info.encode())?;
        let i = self.launch_in(startup, name, ch, lease, extra, Some(&job))?;
        self.session_apps.push((session, i));
        Ok(i)
    }

    /// Konec relace: jeji aplikace ukoncit.
    pub fn close_session(&mut self, session: u64) {
        for (_, i) in self.session_apps.iter().filter(|(s, _)| *s == session) {
            if let Some(a) = self.apps.get(*i) {
                let _ = a.process.kill();
            }
        }
        self.session_apps.retain(|(s, _)| *s != session);
    }

    /// Spusti aplikaci: handly 0 = okno, 1 = listek, pak `extra`.
    pub fn launch_with(&mut self, startup: &Startup, name: &str, window: Channel, lease: Channel, extra: Vec<StartHandle>) -> Result<usize> {
        self.launch_in(startup, name, window, lease, extra, None)
    }

    fn launch_in(&mut self, startup: &Startup, name: &str, window: Channel, lease: Channel, extra: Vec<StartHandle>, parent: Option<&libos::job::Job>) -> Result<usize> {
        let mut handles = vec![StartHandle::user(window.into_handle()), StartHandle::user(lease.into_handle())];
        handles.extend(extra);
        let app = apps::spawn_in(startup, name, handles, parent)?;
        // Prehled uloh: aplikace i se svym jobem (jmeno a identita z manifestu).
        match recipe(startup, name) {
            Ok(info) => crate::tasks::app(&info.name, &info.id, &app.job),
            Err(_) => crate::tasks::app(name, name, &app.job),
        }
        self.apps.push(app);
        Ok(self.apps.len() - 1)
    }
}

/// Bezny beh: kompozitor (hlida a obnovuje ho supervisor), vstup
/// a ukazkove aplikace.
/// Diagnosticky snimek hned (prikaz shellu `diag snap`).
pub fn snapshot() -> Result<()> {
    let g = GUI.lock();
    let gui = g.as_ref().ok_or(Error::NotFound)?;
    gui.command(control::SNAPSHOT.as_bytes(), Vec::new()).map(|_| ())
}

/// Diagnosticky snimek: kanal z kompozitoru do logd pro obrazky obrazovky
/// (bez logd se do logu zapise jen stav).
pub fn connect_snapshots(gui: &Gui) {
    let r = crate::logd::snapshot_channel().and_then(|c| gui.command(control::SNAPSHOTS.as_bytes(), vec![c.into_handle()]).map(|_| ()));
    if let Err(e) = r {
        println!("init: kanal snimku obrazovky do logd: {e}");
    }
}

pub fn start_desktop(startup: &Startup, input: Option<&Input>, sup: &mut Supervisor) {
    let gui = match start(startup) {
        Ok(g) => g,
        Err(Error::NotFound) => {
            println!("init: grafika (virtio-gpu ani framebuffer) neni, GUI nespoustim");
            return;
        }
        Err(e) => {
            println!("init: kompozitor nelze spustit: {e}");
            return;
        }
    };
    if let Some(i) = input
        && let Err(e) = gui.take_input(i)
    {
        println!("init: vstup pro kompozitor: {e}");
    }
    gui.register(sup);
    // Stav v liste: relace site jen se stavem rozhrani a kanal ke zvuku.
    let mut names = Vec::new();
    let mut hs = Vec::new();
    if let Some(n) = crate::system::NET.get()
        && let Ok(s) = n.root.restrict(netproto::policy::rights::INFO, &netproto::policy::Policy::only(Vec::new()))
    {
        names.push("net");
        hs.push(s.into_channel().into_handle());
    }
    if let Ok(a) = crate::audio::connect() {
        names.push("audio");
        hs.push(a.into_handle());
    }
    if !hs.is_empty()
        && let Err(e) = gui.command(format!("{} {}", control::SOURCES, names.join(" ")).as_bytes(), hs)
    {
        println!("init: stav pro listu: {e}");
    }
    connect_snapshots(&gui);
    // Aplikace plochy se spousti az pro prihlasenou relaci (udalost
    // relace, `crate::desktop`), v jejim jobu a s identitou uzivatele.
    connect_launcher(&gui, startup);
    // Relace uz otevrene (vyvojove prihlaseni) a prihlasovaci obrazovka.
    ACCOUNTS_UI.store(true, core::sync::atomic::Ordering::Relaxed);
    give_accounts(&gui.shared.lock());
    *GUI.lock() = Some(gui);
}

/// Spusti aplikaci plochy pod supervisorem s handly, ktere potrebuje
/// (terminal: kanal k termd; Spravce uloh: klient prehledu uloh).
pub fn launch_app(gui: &mut Gui, startup: &Startup, sup: &mut Supervisor, program: &str, session: u64) -> Result<usize> {
    let extra = match program {
        // Terminal: za oknem a listkem kanal pripojeni k termd (init ho
        // spousti a hlida, viz `terminal.rs`); termd s identitou relace.
        "gui-term" => vec![StartHandle::user(crate::terminal::TERMINALS.lock().client_for("gui-term", Some(session))?.into_handle())],
        // Spravce uloh: vidi vsechny ulohy (systemove i aplikace spustene
        // initem, stejne jako ikona v liste), ridit smi jen sve.
        "gui-tasks" => vec![StartHandle::user(crate::tasks::client_view_all("uzivatel")?.into_handle())],
        _ => Vec::new(),
    };
    gui.launch_for_session(startup, program, sup, extra, session)
}

/// Seznam aplikaci pro listu a kanal pro zadosti o spusteni.
fn connect_launcher(gui: &Gui, startup: &Startup) {
    let mut list = String::from(control::APPS);
    let fs = Initfs::open(startup).ok();
    for p in DESKTOP_APPS {
        if let Ok(info) = recipe(startup, p) {
            // `[window] instances = "multi"`: vic oken (vychozi single).
            let text = fs.as_ref().and_then(|f| f.find(&format!("{p}.manifest.toml"))).and_then(|t| core::str::from_utf8(t).ok()).unwrap_or("");
            let multi = conf::parse(text).ok().and_then(|t| t.get_str("window.instances").map(|s| s == "multi")).unwrap_or(false);
            list.push_str(&format!("\n{}\t{}\t{}", info.id, info.name, if multi { "multi" } else { "single" }));
        }
    }
    if let Err(e) = gui.command(list.as_bytes(), Vec::new()) {
        println!("init: seznam aplikaci pro listu: {e}");
    }
    let r = Channel::create().and_then(|(mine, theirs)| {
        gui.command(control::LAUNCHER.as_bytes(), vec![theirs.into_handle()])?;
        *LAUNCHER.lock() = Some(mine);
        Ok(())
    });
    if let Err(e) = r {
        println!("init: kanal spousteni aplikaci: {e}");
    }
}

/// Zadosti kompozitoru `launch <id>`: aplikace podle id z manifestu.
pub fn launch_requests(startup: &Startup, sup: &mut Supervisor) {
    loop {
        let m = match LAUNCHER.lock().as_ref().map(|l| l.try_read()) {
            Some(Ok(m)) => m,
            Some(Err(Error::ShouldWait)) | None => return,
            Some(Err(_)) => {
                *LAUNCHER.lock() = None;
                return;
            }
        };
        let text = String::from_utf8_lossy(&m.data).into_owned();
        // Paleta prikazu: vypnout / restartovat (jako prikaz shellu).
        if let Some(what) = text.strip_prefix("system ") {
            let reboot = match what {
                "shutdown" => false,
                "reboot" => true,
                _ => continue,
            };
            println!("init: paleta prikazu zada {}", if reboot { "restart" } else { "vypnuti" });
            let e = crate::power::shutdown(sup, reboot);
            println!("init: vypnuti z palety selhalo: {e}");
            continue;
        }
        let Some(id) = text.strip_prefix("launch ") else { continue };
        let Some(program) = DESKTOP_APPS.iter().find(|p| recipe(startup, p).is_ok_and(|i| i.id == id)) else {
            println!("init: kompozitor chce spustit neznamou aplikaci {id}");
            continue;
        };
        // Do aktivni relace; bez prihlaseni se nic nespousti.
        let Some(session) = crate::session::active() else {
            println!("init: {program} z listy: nikdo neni prihlaseny");
            continue;
        };
        let mut g = GUI.lock();
        let Some(gui) = g.as_mut() else { return };
        match launch_app(gui, startup, sup, program, session) {
            Ok(_) => println!("init: {program} spusten z listy"),
            Err(e) => println!("init: {program} z listy nelze spustit: {e}"),
        }
    }
}
