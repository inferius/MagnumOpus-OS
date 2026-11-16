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
mod login;
mod notify;
mod status;
mod peers;
mod screen;
mod snapshot;
mod splash;
mod video;

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
    /// Cekani smycky pres port (pocet klientu bez limitu `wait_many`).
    waits: libos::WaitSet,
    /// Ovladac hlasil zmenu konektoru (`HOTPLUG`), vyridi se po kole.
    pending_hotplug: bool,
    /// Ovladac displeje zacal znovu (`RESET`).
    pending_reset: bool,
    /// Skladani na GPU (`backend gpu`); `None` = CPU.
    gpu: Option<gpu::GpuBackend>,
    /// Casove pasmo hodin v liste (od initu s `display`).
    zone: Option<libos::time::zone::Zone>,
    /// Pocet snimku, soucet a maximum casu skladani (ns).
    frame_stats: (u64, u64, u64),
    /// Zdroje stavu v liste: sit a zvuk.
    net_src: Option<status::Source>,
    audio_src: Option<status::Source>,
    /// Kanal na logd pro obrazky diagnostickeho snimku (`snapshots`).
    snapshots: Option<Channel>,
    /// Kanal k initu pro spousteni aplikaci (`launcher`).
    launcher: Option<Channel>,
    /// Prohlizece oznameni relaci (`notify.rs`).
    notify: notify::Viewers,
    /// Kanal `login` ke sluzbe uctu (prihlasovaci obrazovka).
    login: Option<login::Login>,
    /// Prechod ze startovni obrazovky (`splash.rs`): bezici a ohlaseny
    /// (`display ... splash=`, zacne pri pripojeni displeje).
    splash: Option<splash::Handoff>,
    splash_pending: Option<u32>,
    /// Po startu bude prihlasovaci obrazovka (`display ... login`).
    splash_login: bool,
}

impl Service {
    /// Kanal od logd: `nastaveni <klic=hodnota ...>` (soubor
    /// `/system/log/snimek.conf`) a `pad <aplikace>` (automaticky snimek po
    /// padu, kdyz je zapnuty).
    fn read_snapshot_channel(&mut self, now: u64) {
        let Some(ch) = &self.snapshots else { return };
        loop {
            match ch.try_read() {
                Ok(m) => {
                    let text = String::from_utf8_lossy(&m.data);
                    let Some(c) = self.comp.as_mut() else { continue };
                    if let Some(cfg) = text.strip_prefix("nastaveni ") {
                        c.set_snapshot_config(comp::diag::Config::parse(cfg));
                        println!("compd: nastaveni snimku {:?}", c.snapshot_config());
                    } else if let Some(app) = text.strip_prefix("pad ") {
                        c.request_auto_snapshot(true, &format!("pad {app}"), now);
                    }
                }
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    self.snapshots = None;
                    return;
                }
            }
        }
    }
}

fn main(startup: Startup) -> i64 {
    println!("compd: bezim");
    let waits = match libos::WaitSet::new() {
        Ok(w) => w,
        Err(e) => {
            println!("compd: port nejde vytvorit: {e}");
            return 1;
        }
    };
    let mut peers = Peers::default();
    match waits.port().handle().duplicate(None) {
        Ok(h) => peers.set_port(libos::Port::from_handle(h)),
        Err(e) => println!("compd: port pro ploty: {e}"),
    }
    let mut s = Service {
        control: startup.bootstrap,
        screen: None,
        comp: None,
        focus: None,
        secure: None,
        peers,
        waits,
        pending_hotplug: false,
        pending_reset: false,
        gpu: None,
        zone: None,
        frame_stats: (0, 0, 0),
        net_src: None,
        audio_src: None,
        snapshots: None,
        launcher: None,
        notify: notify::Viewers::default(),
        login: None,
        splash: None,
        splash_pending: None,
        splash_login: false,
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
        let next_minute = self.update_clock(now);
        self.read_sources();
        let mut deadline = match (&self.comp, &self.screen) {
            (Some(c), Some(_)) => c.next_wakeup(now),
            // Bez displeje se nesklada; jen casovace (ping, skupiny).
            (Some(_), None) => Some(now + 1_000_000_000),
            _ => None,
        };
        if let Some(m) = next_minute {
            deadline = Some(deadline.map_or(m, |d| d.min(m)));
        }
        if let Some(t) = self.splash_wakeup(now) {
            deadline = Some(deadline.map_or(t, |d| d.min(t)));
        }
        for src in [&mut self.net_src, &mut self.audio_src].into_iter().flatten() {
            if let Some(t) = src.poll(now) {
                deadline = Some(deadline.map_or(t, |d| d.min(t)));
            }
        }
        // Podrzeny commit (plot ziskani): termin ACQUIRE_TIMEOUT.
        if let Some(t) = self.peers.next_deadline() {
            deadline = Some(deadline.map_or(t, |d| d.min(t)));
        }
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
        for src in [&self.net_src, &self.audio_src].into_iter().flatten() {
            items.push((src.ch.handle(), rd));
        }
        if let Some(l) = &self.login {
            items.push((l.handle(), rd));
        }
        if let Some(s) = &self.snapshots {
            items.push((s.handle(), rd));
        }
        for (_, ch) in &self.notify.list {
            items.push((ch.handle(), rd));
        }
        self.peers.wait_items(&mut items);
        let r = self.waits.wait(&items, deadline);
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
        // Zdroje stavu (sit, zvuk) cte `read_sources`; jejich signaly
        // preskocit, jinak by signaly klientu byly posunute (zprava okna
        // by cekala, az se ozve jiny klient: start okna desitky sekund,
        // ztracene odpovedi na ping = "nereaguje" navzdy).
        for _ in [&self.net_src, &self.audio_src].into_iter().flatten() {
            next(&obs);
        }
        let login = if self.login.is_some() { next(&obs) } else { Signals(0) };
        let snap = if self.snapshots.is_some() { next(&obs) } else { Signals(0) };
        let notify_obs: Vec<Signals> = (0..self.notify.list.len()).map(|_| next(&obs)).collect();
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
        // Odpovedi sluzby uctu (prihlasovaci obrazovka).
        if let (Some(l), Some(c)) = (self.login.as_mut(), self.comp.as_mut())
            && l.read(login, c, now).is_err()
        {
            println!("compd: sluzba uctu zavrela kanal login");
            self.login = None;
            c.set_login(false, now);
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
        // Zpravy od logd (nastaveni snimku, zadost po padu aplikace).
        if snap.intersects(Signals::READABLE | Signals::PEER_CLOSED) {
            self.read_snapshot_channel(now);
        }
        // Spusteni aplikaci z listy: zadost initu (init spousti a hlida).
        if let Some(c) = self.comp.as_mut() {
            let mut msgs: Vec<String> = c.take_launch_requests().into_iter().map(|id| format!("launch {id}")).collect();
            // Paleta: vypnout a restartovat provede init (napajeni).
            for r in c.take_system_requests() {
                msgs.push(String::from(match r {
                    comp::palette::SystemRequest::Shutdown => "system shutdown",
                    comp::palette::SystemRequest::Reboot => "system reboot",
                }));
            }
            for m in msgs {
                match &self.launcher {
                    Some(l) => {
                        let _ = l.write(m.as_bytes(), Vec::new());
                    }
                    None => println!("compd: '{m}' nejde: init nedal kanal launcher"),
                }
            }
        }
        // Diagnosticky snimek (zkratka, prikaz, automaticka udalost).
        if let Some(c) = self.comp.as_mut()
            && let Some(why) = c.take_snapshot_request()
        {
            snapshot::take(c, &why, self.peers.frame, self.snapshots.as_ref());
        }
        // Oznameni: zmeny od notifyd do karet, pozadavky karet zpet.
        if let Some(c) = self.comp.as_mut() {
            self.notify.read(&notify_obs, c, now);
            self.notify.send(c.take_notify_requests());
        }
        // Pozadavky prihlasovaci obrazovky na sluzbu uctu.
        if let Some(c) = self.comp.as_mut() {
            let reqs = c.take_login_requests();
            if !reqs.is_empty() {
                match self.login.as_mut() {
                    Some(l) => {
                        if let Err(e) = l.send(reqs) {
                            println!("compd: kanal login: {e}");
                        }
                    }
                    None => println!("compd: prihlasovaci obrazovka bez kanalu login"),
                }
            }
        }
        // 4. Tik.
        self.maybe_tick();
        // Ustaleni (inspekce `Settle`) ceka prilis dlouho: proc.
        if let Some(c) = self.comp.as_mut()
            && let Some(l) = c.settle_report(now)
        {
            println!("{l}");
        }
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
            // Trasovani vstupu (cil logu `vstup`).
            for l in c.take_trace() {
                println!("{l}");
            }
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
        if self.splash_tick(libos::time::now()) {
            return;
        }
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
        let before = c.prof.parts;
        let composed = match self.gpu.as_mut() {
            Some(g) => match g.tick(c, now, sc.pixels()) {
                Ok(r) => r,
                Err(e) => {
                    // GPU selhalo: zpet na CPU, prekreslit vse.
                    println!("compd: skladani na GPU selhalo ({e:?}), zpet na CPU");
                    if let Some(g) = self.gpu.take() {
                        let _ = g.destroy();
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
        // Mereni casu skladani (rozpocet snimku): plne prekresleni hned,
        // ostatni snimky prumer a maximum po 300 snimcich do logu.
        let took = libos::time::now().saturating_sub(now);
        if let Some(rects) = &composed {
            let px: u64 = rects.iter().map(|r| r.w as u64 * r.h as u64).sum();
            let full = px >= w as u64 * h as u64;
            let st = &mut self.frame_stats;
            if full {
                let parts = c.prof.split_off(&before);
                println!("compd: snimek cele obrazovky {w}x{h}: {} us ({}; {parts})", took / 1000, if self.gpu.is_some() { "gpu" } else { "cpu" });
            } else {
                st.0 += 1;
                st.1 += took;
                st.2 = st.2.max(took);
                if st.0 >= 300 {
                    let parts = c.prof.take(st.0);
                    println!("compd: 300 snimku: prumer {} us, nejvic {} us ({parts})", st.1 / st.0 / 1000, st.2 / 1000);
                    *st = (0, 0, 0);
                }
            }
        }
        if let Some(rects) = composed {
            sc.seq += 1;
            let req = Request::Present { seq: sc.seq, output: sc.output, rects };
            if sc.display.write(&req.encode(), Vec::new()).is_err() {
                // Displej neodpovida: nedrzet snimek "na ceste".
                c.presented(now);
            }
        }
    }

    /// Odpovedi zdroju stavu (sit, zvuk) -> ikony v liste. Zdroj, jehoz
    /// sluzba skoncila, se zahodi (ikona zmizi).
    fn read_sources(&mut self) {
        let Some(c) = self.comp.as_mut() else { return };
        if let Some(s) = self.net_src.as_mut() {
            match s.read(status::net_state) {
                Ok(Some((v, addr))) => {
                    c.set_net(Some(v));
                    c.set_net_addr(addr);
                }
                Ok(None) => {}
                Err(()) => {
                    self.net_src = None;
                    c.set_net(None);
                }
            }
        }
        if let Some(s) = self.audio_src.as_mut() {
            match s.read(status::volume) {
                Ok(Some((v, rec, info))) => {
                    c.set_volume(Some(v));
                    c.set_capturing(rec);
                    c.set_audio_info(Some(info));
                    c.audio_ctl = true;
                }
                Ok(None) => {}
                Err(()) => {
                    self.audio_src = None;
                    c.set_volume(None);
                    c.set_capturing(alloc::vec::Vec::new());
                    c.set_audio_info(None);
                    c.audio_ctl = false;
                }
            }
        }
    }

    /// Hodiny v liste: mistni cas v pasmu od initu. Vraci, kdy se zmeni
    /// minuta (probuzeni), `None` = hodiny se neukazuji.
    fn update_clock(&mut self, now: u64) -> Option<u64> {
        /// Svetovy cas v panelu hodin (pasmo, jmeno mesta).
        const WORLD: &[(&str, &str)] = &[("Europe/London", "Londýn"), ("America/New_York", "New York"), ("Asia/Tokyo", "Tokio")];
        let zone = self.zone?;
        let c = self.comp.as_mut()?;
        let Some(utc) = libos::time::wall() else {
            c.set_clock(None);
            return None;
        };
        let l = zone.to_local(utc);
        let civil = l.civil;
        // Den v tydnu: 1. 1. 1970 byl ctvrtek.
        let days = (utc.secs + l.offset.secs as i64).div_euclid(86_400);
        const DAYS: [&str; 7] = ["čt", "pá", "so", "ne", "po", "út", "st"];
        let wd = DAYS[days.rem_euclid(7) as usize];
        c.set_clock(Some((
            format!("{}:{:02}", civil.hour, civil.minute),
            format!("{wd} {}. {}.", civil.day, civil.month),
        )));
        // Panel hodin: dnesni datum a svetovy cas (mesta jineho pasma).
        let mut world = Vec::new();
        for (name, city) in WORLD {
            let Some(z) = libos::time::zone::Zone::by_name(name) else { continue };
            let o = z.to_local(utc);
            if o.offset.secs == l.offset.secs {
                continue;
            }
            let diff = (o.offset.secs - l.offset.secs) / 60;
            let d = if diff % 60 == 0 { format!("{:+} h", diff / 60) } else { format!("{:+} h {} min", diff / 60, (diff % 60).abs()) };
            world.push((String::from(*city), format!("{}:{:02}", o.civil.hour, o.civil.minute), d));
        }
        c.set_calendar(Some(comp::calendar::CalendarInfo { year: civil.year as i32, month: civil.month as u8, day: civil.day as u8, world }));
        // Vzhled podle denni doby: misto pasma a den/noc (kazdou minutu).
        c.set_sun_place(zone.place().map(|p| (p.lat, p.lon)));
        c.sun_tick(utc.secs, l.offset.secs, now);
        let left = 60 - civil.second as u64;
        Some(now + left * 1_000_000_000)
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
                Out::Appearance { uid, pref } => match &self.launcher {
                    Some(l) => {
                        let _ = l.write(format!("{} user {uid} {}", control::APPEARANCE, pref.to_words()).as_bytes(), Vec::new());
                    }
                    None => println!("compd: volbu vzhledu nejde ulozit: init nedal kanal launcher"),
                },
                // Zadost panelu Zvuk: stejnym kanalem jako stav (systemova
                // relace od initu); stav se pak precte hned.
                Out::Audio(req) => match self.audio_src.as_mut() {
                    Some(s) => {
                        if !s.send(&status::audio_control(&req)) {
                            println!("compd: zadost o zvuk se nepodarila poslat");
                        }
                    }
                    None => println!("compd: zadost o zvuk bez kanalu ke sluzbe zvuku"),
                },
                Out::Device { source, trust } => {
                    if let Some(s) = &self.secure {
                        let cmd = if trust { inputproto::control::TRUST } else { inputproto::control::REJECT };
                        let _ = s.write(format!("{cmd} {source}").as_bytes(), Vec::new());
                    }
                }
                other => self.peers.send(other),
            }
        }
        for l in c.take_logs() {
            println!("{l}");
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
                // Limit spojeni na identitu (procesy s dedenou roli Gui).
                if !c.can_add_client(&info) {
                    println!("compd: {} ma prilis mnoho spojeni, dalsi odmitnuto", info.id);
                    return Err(Error::NoResources);
                }
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
            control::DISPLAY => {
                for w in words.by_ref() {
                    if let Some(p) = w.strip_prefix(control::SPLASH_ARG) {
                        self.begin_splash(p.parse().unwrap_or(0));
                    } else if w == control::LOGIN_ARG {
                        self.splash_login = true;
                    } else {
                        self.zone = libos::time::zone::Zone::by_name(w).or(libos::time::zone::Zone::parse(w));
                    }
                }
                self.attach_display(handles)
            }
            control::SNAPSHOTS => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.snapshots = Some(Channel::from_handle(h));
                (String::new(), Vec::new())
            }),
            control::APPS => match self.comp.as_mut() {
                Some(c) => {
                    // Radky `<id>\t<jmeno>[\t<single|multi>]`.
                    let rows: Vec<Vec<&str>> = text.lines().skip(1).map(|l| l.split('\t').collect()).filter(|v: &Vec<&str>| v.len() >= 2).collect();
                    c.set_launchable(rows.iter().map(|v| (String::from(v[0]), String::from(v[1]))).collect());
                    c.set_multi(rows.iter().filter(|v| v.get(2) == Some(&"multi")).map(|v| String::from(v[0])).collect());
                    Ok((String::new(), Vec::new()))
                }
                None => Err(Error::BadState),
            },
            control::NOTIFY => match (words.next().and_then(|u| u.parse::<u64>().ok()), handles.into_iter().next()) {
                (Some(uid), Some(h)) => {
                    self.notify.add(uid, Channel::from_handle(h));
                    Ok((String::new(), Vec::new()))
                }
                _ => Err(Error::InvalidArgs),
            },
            control::LAUNCHER => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.launcher = Some(Channel::from_handle(h));
                (String::new(), Vec::new())
            }),
            control::PANEL => match (self.comp.as_mut(), words.next()) {
                (Some(c), Some(name)) => {
                    if c.open_panel(name) {
                        Ok((String::new(), Vec::new()))
                    } else {
                        Err(Error::InvalidArgs)
                    }
                }
                (None, _) => Err(Error::BadState),
                _ => Err(Error::InvalidArgs),
            },
            control::DND_FULLSCREEN => match (self.comp.as_mut(), words.next()) {
                (Some(c), Some(v @ ("on" | "off"))) => {
                    c.set_dnd_fullscreen(v == "on");
                    Ok((String::new(), Vec::new()))
                }
                (None, _) => Err(Error::BadState),
                _ => Err(Error::InvalidArgs),
            },
            control::SNAPSHOT => match self.comp.as_mut() {
                Some(c) => {
                    c.request_snapshot("prikaz");
                    Ok((String::new(), Vec::new()))
                }
                None => Err(Error::BadState),
            },
            control::SOURCES => {
                let mut hs = handles.into_iter();
                for w in words.by_ref() {
                    let Some(h) = hs.next() else { break };
                    let ch = Channel::from_handle(h);
                    match w {
                        "net" => self.net_src = Some(status::Source::new(ch, status::net_request())),
                        "audio" => self.audio_src = Some(status::Source::new(ch, status::audio_request())),
                        _ => {}
                    }
                }
                Ok((String::new(), Vec::new()))
            }
            control::THEME => {
                let mode = match words.next() {
                    Some("light") => Ok(theme::Mode::Light),
                    Some("dark") => Ok(theme::Mode::Dark),
                    Some("contrast") => Ok(theme::Mode::Contrast),
                    _ => Err(Error::InvalidArgs),
                };
                match (mode, self.comp.as_mut()) {
                    (Ok(m), Some(c)) => {
                        c.set_mode(m);
                        Ok((String::new(), Vec::new()))
                    }
                    (Ok(_), None) => Err(Error::NotFound),
                    (Err(e), _) => Err(e),
                }
            }
            control::REMOTE => match self.comp.as_mut() {
                Some(c) => {
                    c.set_remote(words.map(String::from).collect());
                    Ok((String::new(), Vec::new()))
                }
                None => Err(Error::NotFound),
            },
            control::APPEARANCE => {
                let who = words.next();
                let rest: Vec<&str> = words.by_ref().collect();
                let (uid, w) = match (who, rest.as_slice()) {
                    (Some("system"), w) => (None, w),
                    (Some("user"), [uid, w @ ..]) => (uid.parse::<u64>().ok().map(Some).unwrap_or(Some(u64::MAX)), w),
                    _ => (Some(u64::MAX), &[][..]),
                };
                let pref = theme::Pref::from_words(&w.join(" "));
                match (self.comp.as_mut(), uid, pref) {
                    (_, Some(u64::MAX), _) | (_, _, None) => Err(Error::InvalidArgs),
                    (None, ..) => Err(Error::NotFound),
                    (Some(c), None, Some(p)) => {
                        c.set_appearance_system(p, now);
                        Ok((String::new(), Vec::new()))
                    }
                    (Some(c), Some(uid), Some(p)) => {
                        c.set_appearance_user(uid, p, now);
                        Ok((String::new(), Vec::new()))
                    }
                }
            }
            control::INPUT => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.focus = Some(Channel::from_handle(h));
                (String::new(), Vec::new())
            }),
            control::SECURE => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.secure = Some(Channel::from_handle(h));
                (String::new(), Vec::new())
            }),
            // Kanal login ke sluzbe uctu: prihlasovaci obrazovka, kdyz nikdo
            // neni prihlaseny.
            control::LOGIN => handles.into_iter().next().ok_or(Error::InvalidArgs).map(|h| {
                self.login = Some(login::Login::new(Channel::from_handle(h)));
                if let Some(c) = self.comp.as_mut() {
                    c.set_login(true, now);
                }
                (String::new(), Vec::new())
            }),
            // Relace od initu: `session open <id> <uid> <jmeno>`,
            // `session lock|unlock|close <id>`.
            control::SESSION => {
                let what = words.next();
                let id = words.next().and_then(|v| v.parse::<u64>().ok());
                match (self.comp.as_mut(), what, id) {
                    (Some(c), Some("open"), Some(id)) => {
                        let uid = words.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        c.session_opened(id, uid, words.next().unwrap_or(""), now);
                        Ok((String::new(), Vec::new()))
                    }
                    (Some(c), Some(w @ ("lock" | "unlock")), Some(id)) => {
                        c.session_locked(id, w == "lock", now);
                        Ok((String::new(), Vec::new()))
                    }
                    (Some(c), Some("close"), Some(id)) => {
                        c.session_closed(id, now);
                        Ok((String::new(), Vec::new()))
                    }
                    (None, _, _) => Err(Error::NotFound),
                    _ => Err(Error::InvalidArgs),
                }
            }
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
            control::BACKEND => self.set_backend(words.next().unwrap_or(""), handles.into_iter().next()),
            control::REVOKE => {
                let app = words.next().unwrap_or("");
                match self.comp.as_mut() {
                    Some(c) if !app.is_empty() => {
                        let ids = c.clients_of(app);
                        self.peers.close_clients(&ids, c);
                        println!("compd: GUI pravo {app} odvolano ({} spojeni zavreno)", ids.len());
                        Ok((format!("{}", ids.len()), Vec::new()))
                    }
                    Some(_) => Err(Error::InvalidArgs),
                    None => Err(Error::NotFound),
                }
            }
            control::STATUS => match &self.comp {
                Some(c) => {
                    let mut s = String::new();
                    let mut st = c.status();
                    st.splash = self.splash.is_some() || self.splash_pending.is_some();
                    let _ = st.format(&mut s);
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

    /// `backend gpu|cpu`: prepnuti skladani (viz `gpu.rs`); `gpu` nese
    /// v `render` kanal render uzlu.
    fn set_backend(&mut self, which: &str, render: Option<Handle>) -> Result<(String, Vec<Handle>)> {
        match which {
            "cpu" => {
                if let Some(g) = self.gpu.take() {
                    let _ = g.destroy();
                }
            }
            "gpu" => {
                if self.gpu.is_some() {
                    return Ok((String::new(), Vec::new()));
                }
                let sc = self.screen.as_ref().ok_or(Error::NotFound)?;
                let (w, h) = (sc.w, sc.h);
                // Render uzel od gpud (otevre ho init a posle s prikazem);
                // vlastnik displeje 3D sam nerozdava.
                let render = render.ok_or(Error::NotSupported)?;
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

/// Monotonni cas (pro moduly bez primeho `libos::time`).
pub(crate) fn compd_now() -> u64 {
    libos::time::now()
}
