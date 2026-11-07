//! termd: sluzba relaci shellu a chytreho terminalu (milnik 6b).
//!
//! Start: bootstrap kanal = spojeni s oknem terminalu (nativni protokol
//! `term::proto`); handly `kind::USER` = [kanal sluzby fsd] (volitelne;
//! bez nej shell vidi prazdny FS jen pro cteni).
//!
//! Relace patri sluzbe: kdyz okno posle `Detach` nebo se kanal zavre
//! uprostred prikazu, prikaz dobehne a jeho bloky pockaji na dalsi
//! `Attach`. Vystup se zaroven zrcadli jako text (adapter VT100) do logu,
//! tedy i na seriovou linku.
//!
//! Vstup z klavesnice prijde od `inputd` (milnik 6a, zatim mimo `main`);
//! do te doby radky posila okno/test pres kanal. Kresleni do
//! framebufferu (`term::screen`) se zapne, az jadro preda framebuffer jako
//! capability.

#![no_std]
#![no_main]

extern crate alloc;

mod fs;

use alloc::vec::Vec;
use core::time::Duration;

use fsproto::FsClient;
use libos::{Channel, Error, Startup, println, time};
use term::proto::{ClientMsg, ServerMsg};
use term::session::{Env, Link, Service};

libos::entry!(main);

/// Zasobnik programu je 256 KB; interpret smi pouzit nejvys tolik.
const STACK_BUDGET: usize = 160 * 1024;

struct ChanLink<'a> {
    chan: &'a Channel,
    open: bool,
    /// Zrcadlit vystup jako text do logu.
    mirror: bool,
}

impl Link for ChanLink<'_> {
    fn send(&mut self, msg: &ServerMsg) {
        if self.mirror {
            let text = term::vt::message(msg, false);
            for line in text.lines() {
                println!("termd| {line}");
            }
        }
        if self.open && self.chan.write(&msg.encode(), Vec::new()).is_err() {
            self.open = false;
        }
    }

    fn poll(&mut self) -> Option<ClientMsg> {
        if !self.open {
            return None;
        }
        match self.chan.try_read() {
            Ok(m) => ClientMsg::decode(&m.data).ok(),
            Err(Error::ShouldWait) => None,
            Err(_) => {
                self.open = false;
                None
            }
        }
    }

    fn wait(&mut self) -> Option<ClientMsg> {
        if !self.open {
            return None;
        }
        match self.chan.read(None) {
            Ok(m) => ClientMsg::decode(&m.data).ok(),
            Err(_) => {
                self.open = false;
                None
            }
        }
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

fn main(mut startup: Startup) -> i64 {
    let chan = startup.bootstrap;
    let mut fsd = match startup.handles.pop() {
        Some(h) => Some(fs::FsdFs { client: FsClient::new(Channel::from_handle(h)) }),
        None => None,
    };
    let mut nofs = fs::NoFs;
    let mut svc = Service::new("/");
    svc.stack_budget = STACK_BUDGET;
    let mut env = SysEnv;
    let mut link = ChanLink { chan: &chan, open: true, mirror: true };
    println!("termd: bezim (protokol v{}, FS {})", term::proto::VERSION, if fsd.is_some() { "fsd" } else { "zadny" });
    loop {
        let msg = chan.read(Some(time::deadline_after(Duration::from_millis(200))));
        let fs: &mut dyn shlang::host::Fs = match fsd.as_mut() {
            Some(f) => f,
            None => &mut nofs,
        };
        match msg {
            Ok(m) => match ClientMsg::decode(&m.data) {
                Ok(cm) => svc.handle(cm, &mut link, fs, &mut env),
                Err(_) => link.send(&ServerMsg::Fail { message: "neplatna zprava protokolu".into() }),
            },
            Err(Error::TimedOut) => svc.idle(&mut link, fs, &mut env),
            Err(Error::PeerClosed) => {
                // Okno je pryc; dobehnout ulohy a skoncit (relace v pameti
                // sluzby zatim neprezije konec procesu).
                link.open = false;
                svc.idle(&mut link, fs, &mut env);
                println!("termd: okno zavreno, koncim");
                return 0;
            }
            Err(e) => {
                println!("termd: chyba cteni: {e}");
                return 1;
            }
        }
    }
}
