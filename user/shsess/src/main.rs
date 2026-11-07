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

#![no_std]
#![no_main]

extern crate alloc;

use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use libos::{Channel, Error, Startup, println, time};
use term::ctl::SessMsg;
use term::proto::{ClientMsg, ServerMsg};
use term::session::{Env, Link, Service};

mod fs;

libos::entry!(main);

/// Zasobnik programu je 256 KB; interpret smi pouzit nejvys tolik.
const STACK_BUDGET: usize = 160 * 1024;

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

struct SysEnv;

impl Env for SysEnv {
    fn now(&mut self) -> Option<i64> {
        time::wall().map(|t| t.secs.saturating_mul(1_000_000_000).saturating_add(t.nanos as i64))
    }

    fn sleep(&mut self, ns: i64) {
        if ns > 0 {
            time::sleep(Duration::from_nanos(ns as u64));
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
    let mut nofs = fs::NoFs;
    let mut svc = Service::single(&home, session);
    svc.stack_budget = STACK_BUDGET;
    let mut env = SysEnv;
    println!("shsess {session}: bezim ({tag})");

    loop {
        // Hlavni smycka: zpravy okna pres termd.
        let mut link = MainLink { chan: &main_chan, open: true };
        while link.open {
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
