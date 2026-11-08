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

/// Kde lezi klony bokem relaci shellu (viz `shsess`).
const ASIDE_DIR: &str = "/tmp/.aside";

/// Terminaly initu (bezny beh).
pub static TERMINALS: spin::Mutex<Terminals> = spin::Mutex::new(Terminals { clients: Vec::new(), console: None, cleaned: false });

/// Co supervisor hlida.
#[derive(Clone, Copy)]
pub enum Wait {
    Conn(usize),
    Reg(usize),
    Termd(usize),
}

impl Terminals {
    /// Novy klient (okno); vrati konec kanalu pripojeni pro okno.
    pub fn client(&mut self, name: &'static str) -> Result<Channel> {
        let (ours, theirs) = Channel::create()?;
        self.clients.push(Client { name, conn: ours, store: Registry::default(), reg: None, termd: None, starts: Vec::new(), closed: false });
        Ok(theirs)
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
        }
    }

    /// Zpracuje, co prislo (`sig` = pozorovane signaly polozky `w`).
    pub fn on_signal(&mut self, startup: &Startup, w: Wait, sig: Signals) {
        match w {
            Wait::Conn(i) if sig.intersects(Signals::READABLE | Signals::PEER_CLOSED) => self.conn_messages(startup, i),
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
            _ => {}
        }
    }

    /// Po zpracovani: zavrene klienty odebrat (indexy `Wait` uz neplati).
    pub fn cleanup(&mut self) {
        self.clients.retain(|c| !c.closed);
    }

    fn conn_messages(&mut self, startup: &Startup, i: usize) {
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
        let reply = match self.spawn_termd(startup, i) {
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
    fn spawn_termd(&mut self, startup: &Startup, i: usize) -> Result<Channel> {
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
        let fs = crate::system::devices().and_then(|d| d.fs.as_ref()).ok_or(Error::NotFound)?;
        // Pred prvni relaci beziciho systemu: vsechny klony jsou zbytky po
        // padu nebo vypnuti (zadna relace jeste nebezi). V testovacim behu
        // ne: relace drivejsich testu (shsess) muzou jeste bezet a disk je
        // pri kazdem behu novy.
        let test_run = startup.flags & libos::abi::bootstrap::flags::TEST_RUN != 0;
        if !self.cleaned && !test_run {
            self.cleaned = true;
            if let Ok(list) = fs.list(ASIDE_DIR) {
                for e in list {
                    let _ = fs.aside_discard(&alloc::format!("{ASIDE_DIR}/{}", e.name));
                }
            }
        }
        let c = &mut self.clients[i];
        let own = fs.connect()?;
        let initfs = Initfs::open(startup)?;
        let image = initfs.find("termd").ok_or(Error::NotFound)?;
        let vmo = startup.initfs.as_ref().ok_or(Error::NotFound)?;
        let (here, there) = Channel::create()?;
        let handles = alloc::vec![
            StartHandle::user(own.into_channel().into_handle()),
            StartHandle::user(there.into_handle()),
            StartHandle::new(kind::INITFS, 0, vmo.handle().duplicate(None)?),
        ];
        let (proc, win) = libos::spawn("termd", image, handles)?;
        c.starts.push(now);
        c.termd = Some(proc);
        c.reg = Some(here);
        println!("init: termd ({}) spusten", c.name);
        Ok(win)
    }

    /// Obslouzi vsechny klienty bez cekani (testy, kde smycku supervisoru
    /// nahrazuje test).
    pub fn poll(&mut self, startup: &Startup) {
        for i in 0..self.clients.len() {
            if !self.clients[i].closed {
                self.conn_messages(startup, i);
                self.registry(i);
            }
        }
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
