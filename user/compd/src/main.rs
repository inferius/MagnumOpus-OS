//! compd: kompozitor (milnik 6c).
//!
//! Logika je v knihovne `comp` (testuje se na hostiteli); tady jsou
//! kanaly, objekty pameti a smycka udalosti. Ridici prikazy od initu
//! (bootstrap kanal) popisuje `guiproto::control`.
//!
//! Smycka: cekani na vsechny kanaly s terminem pristiho tiku. Vstup se
//! vyridi vzdy prvni (priorita vstupu pred skladanim), pak zpravy klientu,
//! nakonec tik (skladani do VMO snimku a `PRESENT` ovladaci displeje).
//! Na displeji je vzdy nejvys jeden snimek.

#![no_std]
#![no_main]

extern crate alloc;

mod peers;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use comp::{Comp, Out};
use gfx::text::Text;
use guiproto::control::{self, ConnectInfo};
use guiproto::display::{Reply, Request};
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::{Channel, Error, Handle, Mapping, Result, Rights, Signals, Startup, Vmo, println};

use peers::{Peers, VmoPixels};

libos::entry!(main);

/// Obraz obrazovky (VMO sdilena s ovladacem displeje).
struct Screen {
    display: Channel,
    w: u32,
    h: u32,
    _vmo: Vmo,
    map: Mapping,
    seq: u64,
}

impl Screen {
    fn pixels(&mut self) -> &mut [u32] {
        // SAFETY: mapovani `w * h` pixelu zije se `Screen`; ovladac ho jen
        // cte (soubezne cteni muze dat roztrzeny snimek, ne chybu pameti).
        unsafe { core::slice::from_raw_parts_mut(self.map.addr() as *mut u32, (self.w * self.h) as usize) }
    }
}

struct Service {
    control: Channel,
    screen: Option<Screen>,
    comp: Option<Comp>,
    focus: Option<Channel>,
    secure: Option<Channel>,
    peers: Peers,
}

fn main(startup: Startup) -> i64 {
    println!("compd: bezim");
    let mut s = Service {
        control: startup.bootstrap,
        screen: None,
        comp: None,
        focus: None,
        secure: None,
        peers: Peers::default(),
    };
    loop {
        if !s.wait_and_serve() {
            break;
        }
    }
    println!("compd: init zavrel ridici kanal, koncim");
    0
}

impl Service {
    fn wait_and_serve(&mut self) -> bool {
        let now = libos::time::now();
        let mut deadline = match (&self.comp, &self.screen) {
            (Some(c), Some(_)) => c.next_wakeup(now),
            // Bez displeje se nesklada; jen casovace (ping, skupiny).
            (Some(_), None) => Some(now + 1_000_000_000),
            _ => None,
        };
        if self.peers.has_backlog() {
            // Odlozene zpravy zkusit znovu brzy (kanal nema signal "volno").
            let soon = now + 5_000_000;
            deadline = Some(deadline.map_or(soon, |d| d.min(soon)));
        }
        let mut items: Vec<(&Handle, Signals)> = Vec::new();
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        items.push((self.control.handle(), rd));
        if let Some(f) = &self.focus {
            items.push((f.handle(), rd));
        }
        if let Some(f) = &self.secure {
            items.push((f.handle(), rd));
        }
        if let Some(sc) = &self.screen {
            items.push((sc.display.handle(), rd));
        }
        self.peers.wait_items(&mut items);
        let r = libos::wait_many(&items, deadline);
        drop(items);
        let obs = match r {
            Ok((_, o)) => o,
            Err(Error::TimedOut) => Vec::new(),
            Err(e) => {
                println!("compd: cekani selhalo: {e}");
                return false;
            }
        };
        let now = libos::time::now();
        let mut i = 0;
        let mut next = |o: &Vec<Signals>| {
            let v = o.get(i).copied().unwrap_or(Signals(0));
            i += 1;
            v
        };
        let ctl = next(&obs);
        let foc = if self.focus.is_some() { next(&obs) } else { Signals(0) };
        let sec = if self.secure.is_some() { next(&obs) } else { Signals(0) };
        let disp = if self.screen.is_some() { next(&obs) } else { Signals(0) };
        let peer_obs: Vec<Signals> = obs.get(i..).map(|s| s.to_vec()).unwrap_or_default();
        // 1. Vstup ma prednost.
        if foc.intersects(Signals::READABLE) {
            self.read_input(false, now);
        } else if foc.intersects(Signals::PEER_CLOSED) {
            self.focus = None;
        }
        if sec.intersects(Signals::READABLE) {
            self.read_input(true, now);
        } else if sec.intersects(Signals::PEER_CLOSED) {
            self.secure = None;
        }
        // 2. Displej (potvrzeni snimku).
        if disp.intersects(Signals::READABLE) {
            self.read_display();
        } else if disp.intersects(Signals::PEER_CLOSED) {
            // Ovladac skoncil: bez displeje dal (okna, dialogy, akce bezi),
            // snimek neni "na ceste".
            println!("compd: ovladac displeje skoncil");
            self.screen = None;
            self.peers.frame = None;
            if let Some(c) = self.comp.as_mut() {
                c.presented(now);
            }
        }
        // 3. Ridici kanal a klienti.
        if ctl.intersects(Signals::READABLE) {
            while let Ok(m) = self.control.try_read() {
                self.command(m.data, m.handles, now);
            }
        } else if ctl.intersects(Signals::PEER_CLOSED) {
            return false;
        }
        if let Some(c) = self.comp.as_mut() {
            self.peers.serve(&peer_obs, c, now);
        }
        // 4. Tik.
        self.maybe_tick();
        self.flush();
        true
    }

    fn read_input(&mut self, secure: bool, now: u64) {
        let ch = if secure { self.secure.as_ref() } else { self.focus.as_ref() };
        let Some(ch) = ch else { return };
        let mut recs = Vec::new();
        while let Ok(m) = ch.try_read() {
            if let Some(r) = inputproto::event::decode(&m.data) {
                recs.extend(r);
            }
        }
        if let Some(c) = self.comp.as_mut() {
            c.input(&recs, now);
        }
    }

    fn read_display(&mut self) {
        let Some(sc) = &self.screen else { return };
        while let Ok(m) = sc.display.try_read() {
            match Reply::decode(&m.data) {
                Ok(Reply::Presented { time, .. }) => {
                    if let Some(c) = self.comp.as_mut() {
                        c.presented(time);
                    }
                }
                Ok(Reply::Error { op, code }) => {
                    println!("compd: displej odmitl operaci {op}: {code}");
                    if let Some(c) = self.comp.as_mut() {
                        c.presented(libos::time::now());
                    }
                }
                _ => {}
            }
        }
    }

    fn maybe_tick(&mut self) {
        let Some(c) = self.comp.as_mut() else { return };
        let Some(sc) = self.screen.as_mut() else {
            c.timers(libos::time::now());
            return;
        };
        let now = libos::time::now();
        if c.next_wakeup(now).is_none_or(|t| t > now) {
            return;
        }
        let (w, h) = (sc.w, sc.h);
        let mut canvas = gfx::Canvas::new(sc.pixels(), w, h, w).expect("snimek ma spravnou velikost");
        if let Some(rects) = c.tick(now, &mut canvas) {
            sc.seq += 1;
            let req = Request::Present { seq: sc.seq, output: 0, rects };
            if sc.display.write(&req.encode(), Vec::new()).is_err() {
                // Displej neodpovida: nedrzet snimek "na ceste".
                c.presented(now);
            }
        }
    }

    /// Rozesle zpravy z jadra kompozitoru.
    fn flush(&mut self) {
        self.peers.flush_backlog();
        let Some(c) = self.comp.as_mut() else { return };
        for o in c.take_out() {
            match o {
                Out::SecureRelease => {
                    if let Some(s) = &self.secure {
                        let _ = s.write(inputproto::control::RELEASE.as_bytes(), Vec::new());
                    }
                }
                Out::Device { source, trust } => {
                    if let Some(s) = &self.secure {
                        let cmd = if trust { inputproto::control::TRUST } else { inputproto::control::REJECT };
                        let _ = s.write(format!("{cmd} {source}").as_bytes(), Vec::new());
                    }
                }
                other => self.peers.send(other),
            }
        }
        self.peers.reap(c);
    }

    fn reply(&self, text: &str, handles: Vec<Handle>) {
        let _ = self.control.write(text.as_bytes(), handles);
    }

    fn command(&mut self, data: Vec<u8>, handles: Vec<Handle>, now: u64) {
        if data.starts_with(b"connect\n") {
            let r = ConnectInfo::decode(&data).and_then(|info| {
                let c = self.comp.as_mut().ok_or(Error::NotFound)?;
                let (mine, theirs) = Channel::create()?;
                let name = info.name.clone();
                let id = c.add_client(info, now);
                self.peers.add_client(id, mine);
                println!("compd: klient {id} ({name}) pripojen");
                Ok(theirs)
            });
            return match r {
                Ok(ch) => self.reply("ok", alloc::vec![ch.into_handle()]),
                Err(e) => self.reply(&format!("err {}", e.name()), Vec::new()),
            };
        }
        let text = String::from_utf8_lossy(&data);
        let mut words = text.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let r: Result<(String, Vec<Handle>)> = match cmd {
            control::DISPLAY => self.attach_display(handles),
            control::INPUT => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.focus = Some(Channel::from_handle(h));
                (String::new(), Vec::new())
            }),
            control::SECURE => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.secure = Some(Channel::from_handle(h));
                (String::new(), Vec::new())
            }),
            control::DIALOG => Channel::create().map(|(mine, theirs)| {
                self.peers.add_dialog(mine);
                (String::new(), alloc::vec![theirs.into_handle()])
            }),
            control::ACTIONS => {
                let name = String::from(words.next().unwrap_or("?"));
                // `actions <jmeno> automat`: kanal agenta - vse se potvrzuje.
                let automat = words.next() == Some("automat");
                Channel::create().map(|(mine, theirs)| {
                    self.peers.add_caller(mine, name, automat);
                    (String::new(), alloc::vec![theirs.into_handle()])
                })
            }
            control::INSPECT => Channel::create().map(|(mine, theirs)| {
                self.peers.add_inspect(mine);
                (String::new(), alloc::vec![theirs.into_handle()])
            }),
            control::TASKS => {
                let mut n = || words.next().and_then(|w| w.parse::<u32>().ok());
                match (n(), n(), self.comp.as_mut()) {
                    (Some(run), Some(agent), Some(c)) => {
                        c.set_tasks(run, agent.min(run));
                        Ok((String::new(), Vec::new()))
                    }
                    (_, _, None) => Err(Error::NotFound),
                    _ => Err(Error::InvalidArgs),
                }
            }
            control::STATUS => match &self.comp {
                Some(c) => {
                    let mut s = String::new();
                    let _ = c.status().format(&mut s);
                    Ok((s, Vec::new()))
                }
                None => Err(Error::NotFound),
            },
            _ => Err(Error::NotSupported),
        };
        match r {
            Ok((t, h)) if t.is_empty() => self.reply("ok", h),
            Ok((t, h)) => self.reply(&format!("ok {t}"), h),
            Err(e) => self.reply(&format!("err {}", e.name()), Vec::new()),
        }
    }

    /// `display`: kanal ovladace grafiky. Zjisti vystup, vytvori snimek
    /// a pripoji ho.
    fn attach_display(&mut self, handles: Vec<Handle>) -> Result<(String, Vec<Handle>)> {
        if self.screen.is_some() {
            return Err(Error::InvalidArgs);
        }
        let ch = Channel::from_handle(handles.into_iter().next().ok_or(Error::InvalidArgs)?);
        let deadline = || Some(libos::time::deadline_after(core::time::Duration::from_secs(5)));
        ch.write(&Request::Info.encode(), Vec::new())?;
        let modes = Reply::decode(&ch.read(deadline())?.data)?;
        let Reply::Modes { outputs, caps } = modes else {
            return Err(Error::Corrupted);
        };
        let out = outputs.first().ok_or(Error::NotFound)?;
        let (w, h) = (out.w, out.h);
        let size = w as u64 * h as u64 * 4;
        let vmo = Vmo::create(size)?;
        let map = Mapping::new(&vmo, 0, size, MAP_READ | MAP_WRITE)?;
        // Ovladac dostane snimek jen pro cteni.
        let ro = vmo.handle().duplicate(Some(Rights::READ | Rights::MAP | Rights::BASIC))?;
        let mut screen = Screen {
            display: ch,
            w,
            h,
            _vmo: vmo,
            map,
            seq: 0,
        };
        let mut comp = Comp::new(w, h, Text::system(), Box::new(token));
        // Prvni snimek slozit hned (ovladac ho pri pripojeni zobrazi).
        let now = libos::time::now();
        let mut canvas = gfx::Canvas::new(screen.pixels(), w, h, w).unwrap();
        if comp.tick(now, &mut canvas).is_some() {
            comp.presented(now);
        }
        let req = Request::Attach { output: 0, w, h, stride: w * 4 };
        screen.display.write(&req.encode(), alloc::vec![ro])?;
        match Reply::decode(&screen.display.read(deadline())?.data)? {
            Reply::Attached { .. } => {}
            Reply::Error { code, .. } => return Err(code),
            _ => return Err(Error::Corrupted),
        }
        println!("compd: displej {w}x{h}{}", if caps & guiproto::display::caps::VIRGL != 0 { " (umi virgl)" } else { "" });
        self.peers.frame = Some((screen.map.addr(), w, h));
        self.screen = Some(screen);
        self.comp = Some(comp);
        Ok((format!("{w} {h}"), Vec::new()))
    }
}

/// Token skupiny transakci: 128 nahodnych bitu od jadra.
fn token() -> u128 {
    let mut b = [0u8; 16];
    libos::random::fill(&mut b);
    u128::from_le_bytes(b)
}

impl VmoPixels {
    pub fn map(vmo: Vmo, px: u64) -> Result<Self> {
        let map = Mapping::new(&vmo, 0, px.checked_mul(4).ok_or(Error::InvalidArgs)?, MAP_READ)?;
        Ok(Self { _vmo: vmo, map, len: px as usize })
    }
}
