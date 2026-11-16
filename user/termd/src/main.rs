//! termd: prostrednik terminalu (milnik 6b). Interpret v nem neni.
//!
//! Start: bootstrap kanal = spojeni s oknem terminalu (nativni protokol
//! `term::proto`); handly `kind::USER` podle poradi = [kanal sluzby fsd,
//! kanal registru relaci, kanal napajeni] (napajeni: pro kazdou relaci
//! termd vytvori kanal, jeden konec posle initu zpravou `session`, druhy
//! relaci `SessMsg::Power` - prikazy `shutdown`/`reboot`); volitelne
//! handly se stitkem (`term::ctl::tag`, `StartHandle::tagged`): brana
//! agenta AI, relace zvukove sluzby a jeji listek (kazda relace shellu
//! dostane vlastni relaci zvuku zpravou `SessMsg::Audio`, s listkem
//! prezije restart `audiod`), kanal sluzby "otevri cestu" (relace shellu
//! ho dostane zpravou `SessMsg::Open`); `kind::INITFS` = obraz initfs (odtud se
//! spousti `shsess`).
//!
//! Kazda relace bezi ve vlastnim procesu `shsess` (`term::broker`). termd
//! jen smeruje zpravy, takze pad relace neshodi ostatni ani termd. Nahradni
//! konec kanalu kazde relace uschova registr (`term::ctl`): kdyz termd
//! skonci (zavrene okno) nebo spadne, relace bezi dal a novy termd si je
//! po startu prevezme (`Claim`, `Adopt`). Bez registru relace dobehnou
//! a skonci.
//!
//! Vystup se zaroven zrcadli jako text (adapter VT100) do logu, tedy i na
//! seriovou linku. Vstup z klavesnice prijde od `inputd`; do te doby
//! radky posila okno/test pres kanal.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use libos::initfs::Initfs;
use libos::{Channel, Error, Handle, Signals, Startup, println, time};
use term::broker::{Broker, Out};
use term::ctl::{RegMsg, SessMsg};
use term::proto::{ClientMsg, ServerMsg};

libos::entry!(main);

struct Sess {
    id: u64,
    chan: Channel,
}

struct Termd {
    window: Channel,
    window_open: bool,
    fsd: Option<FsClient>,
    registry: Option<Channel>,
    /// Zvukova sluzba: kazda relace dostane vlastni kanal (`CONNECT`).
    audio: Option<audioproto::conn::AudioClient>,
    /// Kanal k initu pro napajeni relaci.
    power: Option<Channel>,
    /// Zapisovaci konec brany agenta AI (kazda relace dostane duplikat).
    agent: Option<Channel>,
    /// Kanal sluzby "otevri cestu" relace uzivatele (kazda relace shellu
    /// dostane duplikat zpravou `SessMsg::Open`).
    open: Option<Channel>,
    /// Sluzba exec relace uzivatele (duplikat pro kazdou relaci shellu).
    exec: Option<Channel>,
    image: Option<&'static [u8]>,
    broker: Broker,
    sessions: Vec<Sess>,
    /// Domov uzivatele okna (z registru; "/" bez prihlaseni).
    home: String,
}

fn mirror(msg: &ServerMsg) {
    let text = term::vt::message(msg, None);
    for line in text.lines() {
        println!("termd| {line}");
    }
}

impl Termd {
    fn to_window(&mut self, msg: &ServerMsg) {
        mirror(msg);
        if self.window_open && self.window.write(&msg.encode(), Vec::new()).is_err() {
            self.window_open = false;
        }
    }

    fn park(&self, session: u64, tag: &str, spare: Channel) {
        if let Some(reg) = &self.registry {
            let m = RegMsg::Park { session, tag: tag.into() };
            let _ = reg.write(&m.encode(), alloc::vec![spare.into_handle()]);
        }
    }

    fn exec(&mut self, outs: Vec<Out>) {
        for o in outs {
            match o {
                Out::Window(m) => self.to_window(&m),
                Out::Session(id, m) => {
                    let Some(s) = self.sessions.iter().find(|s| s.id == id) else { continue };
                    let _ = s.chan.write(&SessMsg::Client(m).encode(), Vec::new());
                }
                Out::Spawn { session, tag } => {
                    if let Err(e) = self.spawn(session, &tag) {
                        self.broker.session_ended(session);
                        self.to_window(&ServerMsg::Fail { message: alloc::format!("relaci nejde spustit: {e}") });
                    }
                }
            }
        }
    }

    /// Spusti proces relace: Init s nahradnim kanalem a vlastni relaci fsd.
    fn spawn(&mut self, session: u64, tag: &str) -> Result<(), Error> {
        let image = self.image.ok_or(Error::NotFound)?;
        let (keep, give) = Channel::create()?;
        let mut handles: Vec<Handle> = alloc::vec![give.into_handle()];
        // FS nejde (sluzba spadla, disk hlasi chybu): relace bezi bez
        // souboru a okno ukaze proc - terminal nesmi zustat bez shellu.
        let fs = match self.fsd.as_ref().map(|f| f.connect()) {
            Some(Ok(c)) => Some(c),
            Some(Err(e)) => {
                println!("termd: relace {session} bez FS: {e}");
                self.to_window(&ServerMsg::Fail { message: alloc::format!("souborový systém nedostupný ({e}), relace běží bez souborů") });
                None
            }
            None => None,
        };
        if let Some(c) = fs {
            handles.push(c.into_channel().into_handle());
            // Brana agenta jen za relaci FS (poradi handlu v shsess).
            if let Some(a) = &self.agent
                && let Ok(d) = a.handle().duplicate(None)
            {
                handles.push(d);
            }
        }
        let (_proc, chan) = libos::spawn("shsess", image, Vec::new())?;
        let init = SessMsg::Init { session, tag: tag.into(), home: self.home.clone() };
        chan.write(&init.encode(), handles)?;
        // Zvuk je volitelny: kdyz sluzba neodpovi, relace bezi bez nej.
        if let Some(a) = &self.audio {
            let hs = if a.conn().has_lease() {
                a.narrow().map(|(ch, lease)| alloc::vec![ch.into_handle(), lease.into_handle()])
            } else {
                a.connect().map(|ch| alloc::vec![ch.into_handle()])
            };
            if let Ok(hs) = hs {
                let _ = chan.write(&SessMsg::Audio.encode(), hs);
            }
        }
        if let Some(o) = &self.open
            && let Ok(d) = o.handle().duplicate(None)
        {
            let _ = chan.write(&SessMsg::Open.encode(), alloc::vec![d]);
        }
        if let Some(x) = &self.exec
            && let Ok(d) = x.handle().duplicate(None)
        {
            let _ = chan.write(&SessMsg::Exec.encode(), alloc::vec![d]);
        }
        if let Some(p) = &self.power
            && let Ok((ours, theirs)) = Channel::create()
            && p.write(b"session", alloc::vec![ours.into_handle()]).is_ok()
        {
            let _ = chan.write(&SessMsg::Power.encode(), alloc::vec![theirs.into_handle()]);
        }
        self.park(session, tag, keep);
        self.sessions.push(Sess { id: session, chan });
        // Kanaly prehledu uloh razi init: pozadat, odpoved prijde pozdeji
        // (`registry_msg`), relace mezitim bezi bez nich.
        if let Some(reg) = &self.registry {
            let _ = reg.write(&RegMsg::TasksReq { session }.encode(), Vec::new());
        }
        Ok(())
    }

    /// Zpravy registru po startu: kanaly uloh pro relace. `false` =
    /// registr zavrel kanal.
    fn registry_msg(&mut self) -> bool {
        let Some(reg) = &self.registry else { return true };
        loop {
            match reg.try_read() {
                Ok(m) => {
                    if let Ok(RegMsg::Tasks { session }) = RegMsg::decode(&m.data)
                        && let Some(s) = self.sessions.iter().find(|s| s.id == session)
                    {
                        let _ = s.chan.write(&SessMsg::Tasks.encode(), m.handles);
                    }
                }
                Err(Error::ShouldWait) => return true,
                Err(_) => return false,
            }
        }
    }

    /// Po startu: osirele relace z registru prevzit.
    fn claim(&mut self) {
        let Some(reg) = &self.registry else { return };
        if reg.write(&RegMsg::Claim.encode(), Vec::new()).is_err() {
            return;
        }
        let Ok(m) = reg.read(Some(time::deadline_after(Duration::from_secs(2)))) else { return };
        let Ok(RegMsg::Parked { sessions, home }) = RegMsg::decode(&m.data) else { return };
        // Domov uzivatele okna pro nove relace.
        self.home = home;
        for ((id, tag), h) in sessions.into_iter().zip(m.handles) {
            let chan = Channel::from_handle(h);
            let Ok((keep, give)) = Channel::create() else { continue };
            if chan.write(&SessMsg::Adopt.encode(), alloc::vec![give.into_handle()]).is_err() {
                continue; // relace mezitim skoncila
            }
            self.park(id, &tag, keep);
            self.broker.adopt(id, tag);
            self.sessions.push(Sess { id, chan });
            println!("termd: prevzata relace {id}");
        }
    }

    fn window_msg(&mut self) -> bool {
        loop {
            match self.window.try_read() {
                Ok(m) => {
                    let outs = match ClientMsg::decode(&m.data) {
                        Ok(cm) => self.broker.from_window(cm),
                        Err(_) => alloc::vec![Out::Window(ServerMsg::Fail { message: "neplatna zprava protokolu".into() })],
                    };
                    self.exec(outs);
                }
                Err(Error::ShouldWait) => return true,
                Err(_) => return false,
            }
        }
    }

    /// Zpravy relace `i`; `false` = proces relace skoncil.
    fn session_msg(&mut self, i: usize) -> bool {
        loop {
            let id = self.sessions[i].id;
            match self.sessions[i].chan.try_read() {
                Ok(m) => {
                    if let Ok(sm) = ServerMsg::decode(&m.data)
                        && let Some(fwd) = self.broker.from_session(id, sm)
                    {
                        self.to_window(&fwd);
                    }
                }
                Err(Error::ShouldWait) => return true,
                Err(_) => return false,
            }
        }
    }
}

fn main(mut startup: Startup) -> i64 {
    let initfs = Initfs::open(&startup).ok();
    let image = initfs.as_ref().and_then(|i| i.find("shsess"));
    let mut hs = core::mem::take(&mut startup.handles).into_iter();
    let fsd = hs.next().map(|h| FsClient::new(Channel::from_handle(h)));
    let registry = hs.next().map(Channel::from_handle);
    let power = hs.next().map(Channel::from_handle);
    let agent = startup.take_tagged(term::ctl::tag::AGENT).map(Channel::from_handle);
    let open = startup.take_tagged(term::ctl::tag::OPEN).map(Channel::from_handle);
    let exec = startup.take_tagged(term::ctl::tag::EXEC).map(Channel::from_handle);
    let audio = startup.take_tagged(term::ctl::tag::AUDIO).map(Channel::from_handle).map(|ch| {
        match startup.take_tagged(term::ctl::tag::AUDIO_LEASE) {
            Some(lease) => audioproto::conn::AudioClient::with_lease(ch, Channel::from_handle(lease)),
            None => audioproto::conn::AudioClient::new(ch),
        }
    });
    let nonce = time::wall().map(|t| (t.secs as u64) ^ ((t.nanos as u64) << 20)).unwrap_or_else(time::now);
    let mut t = Termd {
        window: startup.bootstrap,
        window_open: true,
        fsd,
        registry,
        audio,
        power,
        agent,
        open,
        exec,
        image,
        broker: Broker::new(nonce),
        sessions: Vec::new(),
        home: String::from("/"),
    };
    t.claim();
    println!(
        "termd: bezim (protokol v{}, FS {}, registr {}, prevzato {})",
        term::proto::VERSION,
        if t.fsd.is_some() { "fsd" } else { "zadny" },
        if t.registry.is_some() { "ano" } else { "ne" },
        t.sessions.len()
    );
    // Relaci je promenny pocet: cekani pres port (`WaitSet`), ne pres
    // `wait_many` (limit 64 objektu).
    let mut waits = match libos::WaitSet::new() {
        Ok(w) => w,
        Err(e) => {
            println!("termd: port nejde vytvorit: {e}");
            return 1;
        }
    };
    loop {
        let mask = Signals::READABLE | Signals::PEER_CLOSED;
        let mut items: Vec<(&Handle, Signals)> = alloc::vec![(t.window.handle(), mask)];
        items.extend(t.sessions.iter().map(|s| (s.chan.handle(), mask)));
        if let Some(r) = &t.registry {
            items.push((r.handle(), mask));
        }
        match waits.wait(&items, None) {
            Ok(_) => {}
            Err(e) => {
                println!("termd: chyba cekani: {e}");
                return 1;
            }
        }
        if !t.registry_msg() {
            t.registry = None;
        }
        let mut i = 0;
        while i < t.sessions.len() {
            if t.session_msg(i) {
                i += 1;
                continue;
            }
            let s = t.sessions.remove(i);
            if let Some(m) = t.broker.session_ended(s.id) {
                t.to_window(&m);
            }
            if let Some(reg) = &t.registry {
                let _ = reg.write(&RegMsg::Unpark { session: s.id }.encode(), Vec::new());
            }
        }
        if !t.window_msg() {
            // Okno je pryc: relace odpojit; bezi dal ve svych procesech
            // a novy termd si je vezme z registru.
            t.window_open = false;
            let outs = t.broker.window_closed();
            t.exec(outs);
            println!("termd: okno zavreno, koncim ({} relaci bezi dal)", t.sessions.len());
            return 0;
        }
    }
}
