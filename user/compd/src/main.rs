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

mod gpu;
mod peers;
mod screen;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use comp::{Comp, Out};
use guiproto::control::{self, ConnectInfo};
use guiproto::display::{Reply, Request};
use libos::abi::vm::MAP_READ;
use libos::{Channel, Error, Handle, Mapping, Result, Signals, Startup, Vmo, println};

use peers::{Peers, VmoPixels};
use screen::Screen;

libos::entry!(main);

struct Service {
    control: Channel,
    screen: Option<Screen>,
    comp: Option<Comp>,
    focus: Option<Channel>,
    secure: Option<Channel>,
    peers: Peers,
    /// Ovladac hlasil zmenu konektoru (`HOTPLUG`), vyridi se po kole.
    pending_hotplug: bool,
    /// Ovladac displeje zacal znovu (`RESET`).
    pending_reset: bool,
    /// Skladani na GPU (`backend gpu`); `None` = CPU.
    gpu: Option<gpu::GpuBackend>,
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
        pending_hotplug: false,
        pending_reset: false,
        gpu: None,
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
        if self.pending_reset {
            self.handle_reset();
        } else if self.pending_hotplug {
            self.handle_hotplug();
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
        let mut hotplug = false;
        let mut reset = false;
        while let Ok(m) = sc.display.try_read() {
            match Reply::decode(&m.data) {
                Ok(Reply::Presented { time, .. }) => {
                    if let Some(c) = self.comp.as_mut() {
                        c.presented(time);
                    }
                }
                Ok(Reply::Hotplug { output }) => {
                    println!("compd: displej: zmena konektoru {output}");
                    hotplug = true;
                }
                Ok(Reply::Reset) => reset = true,
                Ok(Reply::Error { op, code }) => {
                    println!("compd: displej odmitl operaci {op}: {code}");
                    if let Some(c) = self.comp.as_mut() {
                        c.presented(libos::time::now());
                    }
                }
                _ => {}
            }
        }
        self.pending_hotplug |= hotplug;
        self.pending_reset |= reset;
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
        let composed = match self.gpu.as_mut() {
            Some(g) => match g.tick(c, now, sc.pixels()) {
                Ok(r) => r,
                Err(e) => {
                    // GPU selhalo: zpet na CPU, prekreslit vse.
                    println!("compd: skladani na GPU selhalo ({e:?}), zpet na CPU");
                    if let Some(g) = self.gpu.take() {
                        g.destroy();
                    }
                    c.redraw_all();
                    None
                }
            },
            None => {
                let mut canvas = gfx::Canvas::new(sc.pixels(), w, h, w).expect("snimek ma spravnou velikost");
                c.tick(now, &mut canvas)
            }
        };
        if let Some(rects) = composed {
            sc.seq += 1;
            let req = Request::Present { seq: sc.seq, output: sc.output, rects };
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
            control::MODE => {
                let mut n = || words.next().and_then(|w| w.parse::<u32>().ok());
                match (n(), n()) {
                    (Some(w), Some(h)) => {
                        let hz = n().unwrap_or(0);
                        self.change_mode(w, h, hz).map(|(w, h, hz)| (format!("{w} {h} {hz}"), Vec::new()))
                    }
                    _ => Err(Error::InvalidArgs),
                }
            }
            control::OUTPUTS => self.outputs_text().map(|t| (t, Vec::new())),
            control::BACKEND => self.set_backend(words.next().unwrap_or("")),
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

    /// `backend gpu|cpu`: prepnuti skladani (viz `gpu.rs`).
    fn set_backend(&mut self, which: &str) -> Result<(String, Vec<Handle>)> {
        match which {
            "cpu" => {
                if let Some(g) = self.gpu.take() {
                    g.destroy();
                }
            }
            "gpu" => {
                if self.gpu.is_some() {
                    return Ok((String::new(), Vec::new()));
                }
                let sc = self.screen.as_ref().ok_or(Error::NotFound)?;
                let (w, h) = (sc.w, sc.h);
                // Render uzel z kanalu displeje; odpovedi PRESENTED, ktere
                // mezitim prijdou, se zpracuji hned.
                sc.display.write(&Request::OpenRender.encode(), Vec::new())?;
                let end = libos::time::deadline_after(core::time::Duration::from_secs(5));
                let render = loop {
                    let m = sc.display.read(Some(end))?;
                    match Reply::decode(&m.data)? {
                        Reply::Render => break m.handles.into_iter().next().ok_or(Error::Corrupted)?,
                        Reply::Error { op: guiproto::display::op::OPEN_RENDER, code } => return Err(code),
                        Reply::Presented { time, .. } => {
                            if let Some(c) = self.comp.as_mut() {
                                c.presented(time);
                            }
                        }
                        Reply::Hotplug { .. } => self.pending_hotplug = true,
                        Reply::Reset => self.pending_reset = true,
                        _ => {}
                    }
                };
                let g = gpu::GpuBackend::new(Channel::from_handle(render), w, h).map_err(|e| {
                    println!("compd: GPU backend nejde: {e:?}");
                    match e {
                        venus::client::Error::Os(code) => code,
                        _ => Error::NotSupported,
                    }
                })?;
                self.gpu = Some(g);
            }
            _ => return Err(Error::InvalidArgs),
        }
        if let Some(c) = self.comp.as_mut() {
            c.redraw_all();
        }
        println!("compd: skladani: {which}");
        Ok((String::new(), Vec::new()))
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
