//! Terminal v beznem behu: `termd` pro okna terminalu, registr relaci
//! a zalozni konzole.
//!
//! Okno terminalu (`gui-term`, zalozni `console`) dostane od initu
//! **kanal pripojeni**. Na zpravu `connect` init spusti `termd` (vlastni
//! relace fsd, obraz initfs, kanal registru) a vrati odpoved `ok`
//! s kanalem k nemu (`err <KOD>` = nejde). Kdyz termd skonci nebo spadne,
//! okno uvidi zavreny kanal a pozada znovu; novy termd si z registru
//! klienta prevezme relace (procesy `shsess` bezi dal), takze okno se
//! pripoji ke stejne relaci.
//!
//! Kazdy klient ma vlastni registr: novy termd jednoho okna nesmi prevzit
//! relace jineho okna. Restarty jsou omezene ([`RESTART_MAX`] za
//! [`RESTART_WINDOW`]), aby padajici termd nezahltil system.
//!
//! Zalozni konzole ([`Terminals::start_console`]): kdyz GUI nenabehne
//! (bez virtio-gpu) nebo kompozitor spadne, init spusti `console` nad
//! framebufferem z firmware a preda ji displej a fokus od `inputd`.
//!
//! Napajeni z shellu (`shutdown`, `reboot`): termd dostane kanal napajeni;
//! pro kazdou relaci posle initu zpravou `session` jeden konec noveho
//! kanalu (druhy dostane relace). Pozadavek `powerproto::Request` na nem
//! init preda vypinani ([`crate::power::request_from_shell`]).
//!
//! Smycku obsluhuje supervisor ([`crate::supervisor`]): handly z
//! [`Terminals::wait_items`] jsou soucasti jeho cekani.

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use libos::abi::bootstrap::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Handle, Process, Result, Signals, Startup, println, time};
use term::ctl::{RegMsg, Registry};

/// Okno, ve kterem se pocitaji starty termd jednoho klienta.
pub const RESTART_WINDOW: Duration = Duration::from_secs(60);
/// Nejvic startu termd jednoho klienta v okne.
pub const RESTART_MAX: usize = 10;
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

/// Okno terminalu a jeho termd.
struct Client {
    name: &'static str,
    conn: Channel,
    store: Registry<Handle>,
    /// Kanal registru beziciho termd.
    reg: Option<Channel>,
    termd: Option<Process>,
    starts: Vec<u64>,
    /// Okno skoncilo (odebere se v `cleanup`).
    closed: bool,
    /// Kanal napajeni k beziciemu termd (zpravy `session`).
    power: Option<Channel>,
    /// Kanaly napajeni relaci shellu (`None` = zavreny, odebere `cleanup`).
    power_sessions: Vec<Option<Channel>>,
    /// Relace sluzby "otevri cestu" (`crate::open`) pro relace shellu;
    /// obvykle = `session`, testy dosadi testovaci relaci sluzby.
    open_session: Option<u64>,
    /// Relace prihlaseneho uzivatele (`crate::session`): termd dostane jeji
    /// identitu FS a bezi v jejim jobu. `None` = system (testy, zalozni
    /// konzole).
    session: Option<u64>,
}

#[derive(Default)]
pub struct Terminals {
    clients: Vec<Client>,
    /// Zalozni konzole: proces a ridici kanal (zavreni = konec konzole).
    console: Option<(Process, Channel)>,
    /// Zbytky klonu bokem po minulem behu uz jsou uklizene.
    cleaned: bool,
}

/// Identita uzivatele terminalu v prehledu uloh (system je zatim pro
/// jednoho uzivatele).
const USER: &str = "uzivatel";

/// Kde lezi klony bokem relaci shellu systemu (uzivatele: `/tmp/<uid>/.aside`,
/// viz `shsess`).
const ASIDE_DIR: &str = "/tmp/.aside";

/// Klony bokem po minulem behu: `/tmp/.aside` (relace systemu) a `.aside`
/// v soukromych slozkach uzivatelu `/tmp/<uid>`.
fn leftover_clones(fs: &fsproto::FsClient) -> Vec<String> {
    let mut dirs = alloc::vec![String::from(ASIDE_DIR)];
    if let Ok(list) = fs.list(fsproto::perm::TMP_DIR) {
        dirs.extend(
            list.into_iter()
                .filter(|e| e.name.parse::<u64>().is_ok())
                .map(|e| alloc::format!("{}/{}", fsproto::perm::TMP_DIR, e.name) + "/.aside"),
        );
    }
    let mut out = Vec::new();
    for d in dirs {
        if let Ok(list) = fs.list(&d) {
            out.extend(list.into_iter().map(|e| alloc::format!("{d}/{}", e.name)));
        }
    }
    out
}

/// Terminaly initu (bezny beh).
pub static TERMINALS: spin::Mutex<Terminals> = spin::Mutex::new(Terminals { clients: Vec::new(), console: None, cleaned: false });

/// Co supervisor hlida.
#[derive(Clone, Copy)]
pub enum Wait {
    Conn(usize),
    Reg(usize),
    Termd(usize),
    Power(usize),
    PowerSession(usize, usize),
}

impl Terminals {
    /// Novy klient (okno); vrati konec kanalu pripojeni pro okno.
    pub fn client(&mut self, name: &'static str) -> Result<Channel> {
        self.client_for(name, None)
    }

    /// Novy klient (okno) relace `session`.
    pub fn client_for(&mut self, name: &'static str, session: Option<u64>) -> Result<Channel> {
        let (ours, theirs) = Channel::create()?;
        // Domov prihlaseneho uzivatele (`~`, `cd` bez argumentu); bez
        // prihlaseni koren.
        let mut store = Registry::default();
        if let Some(name) = session.and_then(crate::session::user_name) {
            store.home = alloc::format!("/home/{name}");
        }
        self.clients.push(Client {
            name,
            conn: ours,
            store,
            reg: None,
            termd: None,
            starts: Vec::new(),
            closed: false,
            power: None,
            power_sessions: Vec::new(),
            open_session: session,
            session,
        });
        Ok(theirs)
    }

    /// (Testy) Klient bez prihlaseni, ale s kanalem sluzby "otevri cestu"
    /// testovaci relace (`open::test_session`).
    pub fn client_with_open(&mut self, name: &'static str, open_session: u64) -> Result<Channel> {
        let ch = self.client_for(name, None)?;
        if let Some(c) = self.clients.last_mut() {
            c.open_session = Some(open_session);
        }
        Ok(ch)
    }

    /// Handly k cekani (pripojeni, registry, procesy termd).
    pub fn wait_items<'a>(&'a self, items: &mut Vec<(&'a Handle, Signals)>, what: &mut Vec<Wait>) {
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        for (i, c) in self.clients.iter().enumerate() {
            if c.closed {
                continue;
            }
            items.push((c.conn.handle(), rd));
            what.push(Wait::Conn(i));
            if let Some(r) = &c.reg {
                items.push((r.handle(), rd));
                what.push(Wait::Reg(i));
            }
            if let Some(p) = &c.termd {
                items.push((p.handle(), Signals::TERMINATED));
                what.push(Wait::Termd(i));
            }
            if let Some(p) = &c.power {
                items.push((p.handle(), rd));
                what.push(Wait::Power(i));
            }
            for (k, s) in c.power_sessions.iter().enumerate() {
                if let Some(s) = s {
                    items.push((s.handle(), rd));
                    what.push(Wait::PowerSession(i, k));
                }
            }
        }
    }

    /// Zpracuje, co prislo (`sig` = pozorovane signaly polozky `w`).
    /// `audio` = relace zvukove sluzby s listkem pro noveho termd (kanal,
    /// listek), vola se jen pri jeho startu.
    pub fn on_signal(&mut self, startup: &Startup, w: Wait, sig: Signals, audio: &mut dyn FnMut() -> Option<(Channel, Channel)>) {
        match w {
            Wait::Conn(i) if sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) => self.conn_messages(startup, i, audio),
            Wait::Reg(i) if sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) => self.registry(i),
            Wait::Termd(i) if sig.intersects(Signals::TERMINATED) => {
                let c = &mut self.clients[i];
                if let Some(p) = c.termd.take() {
                    let code = libos::sys::process_exit_code(p.handle().raw()).unwrap_or(i64::MIN);
                    println!("init: termd ({}) skoncil: {}", c.name, libos::abi::process::describe_exit(code));
                }
                // Zbyle zpravy registru (Park pred koncem) jeste zpracovat.
                self.registry(i);
            }
            Wait::Power(i) if sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) => {
                let c = &mut self.clients[i];
                while let Some(p) = &c.power {
                    match p.try_read() {
                        Ok(m) if &m.data[..] == b"session" => {
                            c.power_sessions.extend(m.handles.into_iter().map(|h| Some(Channel::from_handle(h))));
                        }
                        Ok(_) => {}
                        Err(Error::ShouldWait) => break,
                        Err(_) => c.power = None,
                    }
                }
            }
            Wait::PowerSession(i, k) if sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) => {
                let session = self.clients[i].session;
                let slot = &mut self.clients[i].power_sessions[k];
                if let Some(chan) = slot.take() {
                    match chan.try_read() {
                        // Pozadavek: init si kanal vezme (odpoved pri chybe),
                        // nebo ho vrati.
                        Ok(m) => *slot = crate::power::request_from_shell(&m.data, chan, session),
                        Err(Error::ShouldWait) => *slot = Some(chan),
                        // Relace skoncila: kanal zahodit.
                        Err(_) => {}
                    }
                }
            }
            _ => {}
        }
    }

    /// Pred vypnutim: ukonci termd a zalozni konzoli (relace shellu skonci
    /// samy, kdyz nikdo nedrzi jejich registr).
    pub fn stop_all(&mut self) {
        for c in &mut self.clients {
            if let Some(p) = c.termd.take() {
                let _ = p.kill();
            }
            c.closed = true;
        }
        if let Some((p, _)) = self.console.take() {
            let _ = p.kill();
        }
    }

    /// Po zpracovani: zavrene klienty odebrat (indexy `Wait` uz neplati).
    pub fn cleanup(&mut self) {
        for c in &mut self.clients {
            c.power_sessions.retain(Option::is_some);
        }
        self.clients.retain(|c| !c.closed);
    }

    fn conn_messages(&mut self, startup: &Startup, i: usize, audio: &mut dyn FnMut() -> Option<(Channel, Channel)>) {
        // Vsechny cekajici zpravy najednou: vic `connect` za sebou (okno to
        // zkusilo znovu, nez init odpovedel) = jen posledni spusti termd,
        // drivejsi dostanou `err CANCELED` (jinak by kazdy dalsi ukoncil
        // termd predchoziho).
        let mut connects = 0usize;
        loop {
            match self.clients[i].conn.try_read() {
                Ok(m) if &m.data[..] == b"connect" => connects += 1,
                Ok(_) => {
                    let _ = self.clients[i].conn.write(b"err INVALID_ARGS", Vec::new());
                }
                Err(Error::ShouldWait) => break,
                Err(_) => {
                    // Okno skoncilo: jeho termd ukoncit (relace bezi dal
                    // v registru jen do konce initu).
                    let c = &mut self.clients[i];
                    println!("init: okno terminalu ({}) skoncilo", c.name);
                    if let Some(p) = c.termd.take() {
                        let _ = p.kill();
                    }
                    c.reg = None;
                    c.closed = true;
                    return;
                }
            }
        }
        for _ in 1..connects {
            let _ = self.clients[i].conn.write(b"err CANCELED", Vec::new());
        }
        if connects == 0 {
            return;
        }
        let reply = match self.spawn_termd(startup, i, audio) {
            Ok(win) => self.clients[i].conn.write(b"ok", alloc::vec![win.into_handle()]),
            Err(e) => {
                println!("init: termd ({}) nelze spustit: {e}", self.clients[i].name);
                self.clients[i].conn.write(alloc::format!("err {}", e.name()).as_bytes(), Vec::new())
            }
        };
        if let Err(e) = reply {
            println!("init: odpoved oknu terminalu: {e}");
        }
    }

    fn registry(&mut self, i: usize) {
        let c = &mut self.clients[i];
        let Some(chan) = &c.reg else { return };
        loop {
            match chan.try_read() {
                Ok(m) => {
                    let Ok(msg) = RegMsg::decode(&m.data) else { continue };
                    // Kanaly prehledu uloh pro relaci: razi je init (identita
                    // uzivatele, jen vlastni ulohy).
                    if let RegMsg::TasksReq { session } = msg {
                        let hs = match (crate::tasks::producer(USER), crate::tasks::client(USER, false, true)) {
                            (Ok(p), Ok(cl)) => alloc::vec![p.into_handle(), cl.into_handle()],
                            _ => Vec::new(),
                        };
                        let _ = chan.write(&RegMsg::Tasks { session }.encode(), hs);
                        continue;
                    }
                    if let Some((reply, hs)) = c.store.handle(msg, m.handles) {
                        let _ = chan.write(&reply.encode(), hs);
                    }
                }
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    // termd je pryc; uschovane konce relaci zustavaji.
                    c.reg = None;
                    return;
                }
            }
        }
    }

    /// Spusti termd klienta `i`; vrati kanal okna k nemu.
    fn spawn_termd(&mut self, startup: &Startup, i: usize, audio: &mut dyn FnMut() -> Option<(Channel, Channel)>) -> Result<Channel> {
        let c = &mut self.clients[i];
        let now = time::now();
        let window = RESTART_WINDOW.as_nanos() as u64;
        c.starts.retain(|&t| now.saturating_sub(t) < window);
        if c.starts.len() >= RESTART_MAX {
            println!("init: termd ({}) startoval {}x za {} s, odmitam", c.name, RESTART_MAX, RESTART_WINDOW.as_secs());
            return Err(Error::NoResources);
        }
        // Stary termd (pokud jeste bezi) ukoncit: okno chce novy.
        if let Some(p) = c.termd.take() {
            let _ = p.kill();
        }
        // Zpravy stareho registru (Park) zpracovat, nez ho zahodime.
        self.registry(i);
        let fs = crate::system::devices().and_then(|d| d.fs.as_ref());
        // Pred prvni relaci beziciho systemu: vsechny klony jsou zbytky po
        // padu nebo vypnuti (zadna relace jeste nebezi). V testovacim behu
        // ne: relace drivejsich testu (shsess) muzou jeste bezet a disk je
        // pri kazdem behu novy.
        let test_run = startup.flags & libos::abi::bootstrap::flags::TEST_RUN != 0;
        // Mazani bezi ve vlakne: kazdy klon je commit FS (na disku bez
        // mezipameti zapisu desitky sekund) a termd na nej nesmi cekat.
        // Seznam se vezme ted, pred startem termd: klony nove relace v nem
        // nejsou (cesta klonu existuje, dokud ji nikdo nesmaze), takze je
        // vlakno nesmaze.
        if let Some(fs) = fs
            && !self.cleaned
            && !test_run
        {
            self.cleaned = true;
            match leftover_clones(fs) {
                names if !names.is_empty() => {
                    match fs.connect() {
                        Ok(own) => {
                            if let Err(e) = libos::thread::spawn("aside-uklid", move || {
                                for n in &names {
                                    let _ = own.aside_discard(n);
                                }
                                println!("init: {} zbytku klonu po minulem behu smazano", names.len());
                            }) {
                                println!("init: uklid klonu nelze spustit: {e}");
                            }
                        }
                        Err(e) => println!("init: uklid klonu: relace FS: {e}"),
                    }
                }
                _ => {}
            }
        }
        let c = &mut self.clients[i];
        // Bez FS (sluzba nebezi, relace nejde) termd presto spustit:
        // dostane mrtvy kanal, relace pobezi bez souboru a okno ukaze chybu.
        // Okno relace: identita prihlaseneho uzivatele, ne initu.
        let own = match c.session {
            Some(s) => Some(crate::session::connect(s)),
            None => fs.map(|f| f.connect()),
        };
        let own = match own {
            Some(Ok(c)) => c.into_channel(),
            r => {
                let why = match r {
                    Some(Err(e)) => e,
                    _ => Error::NotFound,
                };
                println!("init: termd ({}) bez FS: {why}", c.name);
                Channel::create()?.0
            }
        };
        let initfs = Initfs::open(startup)?;
        let image = initfs.find("termd").ok_or(Error::NotFound)?;
        let vmo = startup.initfs.as_ref().ok_or(Error::NotFound)?;
        let (here, there) = Channel::create()?;
        let (power_here, power_there) = Channel::create()?;
        let mut handles = alloc::vec![
            StartHandle::user(own.into_handle()),
            StartHandle::user(there.into_handle()),
            StartHandle::new(kind::INITFS, 0, vmo.handle().duplicate(None)?),
            StartHandle::user(power_there.into_handle()),
        ];
        // Zvuk pro relace (prikaz `audio`): relace s listkem, prezije
        // restart audiod. Bez sluzby terminal bezi dal.
        if let Some((ch, lease)) = audio() {
            handles.push(StartHandle::tagged(term::ctl::tag::AUDIO, ch.into_handle()));
            handles.push(StartHandle::tagged(term::ctl::tag::AUDIO_LEASE, lease.into_handle()));
        }
        // Sluzba "otevri cestu" (hola cesta k souboru, `start`): jen okno
        // prihlaseneho uzivatele; kanal patri jeho relaci (identita FS).
        if let Some(s) = c.open_session {
            match crate::open::endpoint(s) {
                Ok(ch) => handles.push(StartHandle::tagged(term::ctl::tag::OPEN, ch.into_handle())),
                Err(e) => println!("init: termd ({}) bez sluzby otevri cestu: {e}", c.name),
            }
        }
        // Spousteni programu ze shellu (sluzba exec relace, i vzdalene):
        // shell sam sit nema, programy ji dostanou podle manifestu.
        if let Some(s) = c.session {
            match crate::exec::endpoint(s) {
                Ok(ch) => handles.push(StartHandle::tagged(term::ctl::tag::EXEC, ch.into_handle())),
                Err(e) => println!("init: termd ({}) bez spousteni programu: {e}", c.name),
            }
        }
        // Brana agenta AI (jen zapis zadosti o relaci).
        if let Some(h) = crate::agent::terminal_gate() {
            handles.push(StartHandle::tagged(term::ctl::tag::AGENT, h));
        }
        let (proc, win) = match c.session {
            // V jobu relace: odhlaseni ukonci termd i jeho shsess.
            Some(s) => {
                let job = crate::session::job(s)?;
                libos::spawn::spawn_with(&job, "termd", image, handles, termd_opts())?
            }
            None => libos::spawn::spawn_with(&libos::job::current()?, "termd", image, handles, termd_opts())?,
        };
        c.starts.push(now);
        c.termd = Some(proc);
        c.reg = Some(here);
        c.power = Some(power_here);
        println!("init: termd ({}) spusten", c.name);
        Ok(win)
    }

    /// Konec relace: termd jejich oken ukoncit a okna zapomenout.
    pub fn close_session(&mut self, session: u64) {
        for c in self.clients.iter_mut().filter(|c| c.session == Some(session)) {
            if let Some(p) = c.termd.take() {
                let _ = p.kill();
            }
            c.reg = None;
            c.power = None;
            c.power_sessions.clear();
            c.closed = true;
        }
    }

    /// Obslouzi vsechny klienty bez cekani (testy, kde smycku supervisoru
    /// nahrazuje test).
    pub fn poll(&mut self, startup: &Startup) {
        for i in 0..self.clients.len() {
            if !self.clients[i].closed {
                self.conn_messages(startup, i, &mut || None);
                self.registry(i);
            }
        }
    }

    /// Je klient `name` ukonceny (okno skoncilo, termd ukoncen)? Nezname
    /// jmeno = ano.
    pub fn is_closed(&self, name: &str) -> bool {
        self.clients.iter().filter(|c| c.name == name).all(|c| c.closed)
    }

    /// Bezi zalozni konzole?
    pub fn console_running(&self) -> bool {
        self.console.is_some()
    }

    /// Zalozni konzole nad framebufferem z firmware (bez GUI, po padu
    /// kompozitoru). Displej a fokus ji preda init (vlastnik).
    pub fn start_console(&mut self, startup: &Startup) -> Result<()> {
        if self.console.is_some() {
            return Ok(());
        }
        let fb = startup.framebuffer.as_ref().ok_or(Error::NotFound)?;
        let conn = self.client("console")?;
        let initfs = Initfs::open(startup)?;
        let image = initfs.find("console").ok_or(Error::NotFound)?;
        let (proc, ctl) = libos::spawn("console", image, alloc::vec![StartHandle::user(conn.into_handle())])?;
        command(&ctl, "display", alloc::vec![fb.handle().duplicate(None)?])?;
        if let Some(input) = crate::system::devices().and_then(|d| d.input.as_ref()) {
            let (mine, theirs) = Channel::create()?;
            input.command(inputproto::control::FOCUS, alloc::vec![theirs.into_handle()])?;
            command(&ctl, "focus", alloc::vec![mine.into_handle()])?;
        } else {
            println!("init: konzole bez vstupu (sluzba vstupu nebezi)");
        }
        println!("init: zalozni konzole bezi");
        self.console = Some((proc, ctl));
        Ok(())
    }
}

/// Ridici prikaz konzoli (odpoved `ok...` / `err KOD`).
fn command(ctl: &Channel, text: &str, handles: Vec<Handle>) -> Result<String> {
    ctl.write(text.as_bytes(), handles)?;
    let m = ctl.read(Some(time::deadline_after(REPLY_TIMEOUT)))?;
    let reply = String::from_utf8_lossy(&m.data).into_owned();
    if reply.starts_with("ok") {
        Ok(reply)
    } else {
        println!("init: konzole na '{text}': {reply}");
        Err(Error::InvalidArgs)
    }
}

/// Terminal spousti prikazy: relace s libd (programy se zavadecem).
fn termd_opts() -> libos::spawn::Options {
    libos::spawn::Options { libd: true, ..Default::default() }
}
