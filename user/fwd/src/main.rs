//! fwd: sluzba firewallu.
//!
//! Spravuje konfiguraci firewallu (profily siti, vyjimky, blokovane site
//! a domeny, rozhodnuti uzivatele), posila ji `netd` a vyrizuje dotazy:
//! kdyz aplikace chce cil mimo pravidla sveho manifestu, `netd` se zepta
//! fwd, ten se zepta agenta uzivatele (UI) a odpoved vrati. Trvala
//! rozhodnuti ulozi do konfigurace na FS.
//!
//! Start (kind USER, v tomto poradi):
//! 1. kanal k `netd` (`netproto::fw`),
//! 2. kanal agenta uzivatele (UI; bez nej plati `ask_default`),
//! 3. (volitelne) relace FS s pripojenou cestou `/config` - bez ni se
//!    konfigurace neuklada (plati do restartu).
//!
//! Na bootstrap kanalu prijde od init vychozi konfigurace (TOML z initfs);
//! fwd odpovi `ok` nebo `err:<popis>`. Kdyz na FS konfigurace neni, ulozi
//! tam vychozi.

#![no_std]
#![no_main]

extern crate alloc;

mod config;
mod store;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::IpAddr;
use core::time::Duration;

use conf::Table;
use libos::{Channel, Error, Signals, Startup, println};
use netproto::fw::{Ask, choice, op};
use netproto::{ReadAddr, Reader, Rule, Target, WriteAddr, Writer};

use crate::config::Service;
use crate::store::Store;

libos::entry!(main);

/// Rozpracovany dotaz (netd ceka na odpoved).
struct Question {
    ask: Ask,
    deadline: u64,
}

struct Fwd {
    netd: Channel,
    agent: Option<Channel>,
    store: Store,
    cfg: Table,
    svc: Service,
    /// Aktualni sit (MAC brany) a jeji brana.
    network: Option<(String, String)>,
    questions: BTreeMap<u32, Question>,
    /// Dotaz agenta na sit, ktery jeste nema odpoved.
    network_question: Option<u32>,
    next_id: u32,
}

fn main(startup: Startup) -> i64 {
    let bootstrap = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let Some(netd) = handles.next().map(Channel::from_handle) else {
        let _ = bootstrap.write(b"err:chybi kanal netd", Vec::new());
        return 1;
    };
    let agent = handles.next().map(Channel::from_handle);
    // Relace FS a listek k ni (obnova po padu fsd).
    let fs = handles.next().map(Channel::from_handle).map(|ch| match handles.next() {
        Some(lease) => fsproto::FsClient::with_lease(ch, Channel::from_handle(lease)),
        None => fsproto::FsClient::new(ch),
    });
    let defaults = match bootstrap.read(Some(libos::time::deadline_after(Duration::from_secs(5)))) {
        Ok(m) => m.data,
        Err(e) => {
            println!("fwd: init neposlal vychozi konfiguraci: {e}");
            let _ = bootstrap.write(b"err:chybi konfigurace", Vec::new());
            return 2;
        }
    };
    let mut store = Store::new(fs);
    let (cfg, svc) = match load(&mut store, &defaults) {
        Ok(c) => c,
        Err(e) => {
            println!("fwd: neplatna konfigurace: {e}");
            let _ = bootstrap.write(format!("err:{e}").as_bytes(), Vec::new());
            return 3;
        }
    };
    let mut f = Fwd {
        netd,
        agent,
        store,
        cfg,
        svc,
        network: None,
        questions: BTreeMap::new(),
        network_question: None,
        next_id: 1,
    };
    f.push_config();
    let _ = bootstrap.write(b"ok", Vec::new());
    println!(
        "fwd: bezim (agent {}, ukladani {})",
        if f.agent.is_some() { "ano" } else { "ne" },
        if f.store.available() { "na FS" } else { "jen v pameti" }
    );
    f.run();
    0
}

/// Konfigurace z FS, jinak vychozi (a ulozi ji na FS).
fn load(store: &mut Store, defaults: &[u8]) -> config::Res<(Table, Service)> {
    let parse = |data: &[u8]| -> config::Res<(Table, Service)> {
        let text = core::str::from_utf8(data).map_err(|_| String::from("neplatne UTF-8"))?;
        let t = conf::parse(text).map_err(|e| format!("{e}"))?;
        let svc = config::service(&t)?;
        let profile = config::profile_for(&t, None);
        config::build(&t, &profile)?;
        Ok((t, svc))
    };
    match store.load() {
        Ok(data) => match parse(&data) {
            Ok(c) => return Ok(c),
            Err(e) => println!("fwd: {} je neplatna ({e}), pouzivam vychozi", store::PATH),
        },
        Err(Error::NotFound) => {}
        Err(e) => println!("fwd: {} nejde precist: {e}", store::PATH),
    }
    let c = parse(defaults)?;
    if store.available() {
        match store.save(defaults) {
            Ok(()) => println!("fwd: vychozi konfigurace ulozena do {}", store::PATH),
            Err(e) => println!("fwd: vychozi konfiguraci nejde ulozit: {e}"),
        }
    }
    Ok(c)
}

impl Fwd {
    fn profile(&self) -> String {
        config::profile_for(&self.cfg, self.network.as_ref().map(|(id, _)| id.as_str()))
    }

    /// Posle netd konfiguraci podle aktualniho profilu.
    fn push_config(&mut self) {
        let profile = self.profile();
        match config::build(&self.cfg, &profile) {
            Ok(c) => {
                let mut w = Writer::op(op::CONFIG);
                c.put(&mut w);
                if self.netd.write(&w.finish(), Vec::new()).is_err() {
                    println!("fwd: konfiguraci nejde poslat netd");
                }
            }
            Err(e) => println!("fwd: konfigurace neplatna: {e}"),
        }
    }

    /// Ulozi konfiguraci; pri souboznem zapisu jinde nacte novou verzi,
    /// zmenu `edit` na ni zopakuje a ulozi znovu.
    fn save(&mut self, edit: &dyn Fn(&mut Table)) {
        if !self.store.available() {
            return;
        }
        for _ in 0..3 {
            match self.store.save(self.cfg.to_toml().as_bytes()) {
                Ok(()) => return,
                Err(Error::Conflict) => {
                    let fresh = self
                        .store
                        .load()
                        .ok()
                        .and_then(|d| String::from_utf8(d).ok())
                        .and_then(|t| conf::parse(&t).ok());
                    if let Some(mut t) = fresh {
                        edit(&mut t);
                        self.cfg = t;
                    }
                }
                Err(e) => {
                    println!("fwd: konfiguraci nejde ulozit: {e}");
                    return;
                }
            }
        }
    }

    fn run(&mut self) {
        loop {
            let deadline = self.questions.values().map(|q| q.deadline).min();
            let mut items: Vec<(&libos::Handle, Signals)> = Vec::with_capacity(2);
            items.push((self.netd.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            if let Some(a) = &self.agent {
                items.push((a.handle(), Signals::READABLE | Signals::PEER_CLOSED));
            }
            let woke = libos::wait_many(&items, deadline);
            drop(items);
            match woke {
                Ok((0, _)) => {
                    if !self.netd_messages() {
                        println!("fwd: netd skoncil");
                        return;
                    }
                }
                Ok(_) => self.agent_messages(),
                Err(Error::TimedOut) => {}
                Err(e) => {
                    println!("fwd: cekani selhalo: {e}");
                    return;
                }
            }
            self.expire();
        }
    }

    /// Dotazy po terminu: vychozi odpoved.
    fn expire(&mut self) {
        let now = libos::time::now();
        let late: Vec<u32> = self
            .questions
            .iter()
            .filter(|(_, q)| q.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in late {
            if let Some(q) = self.questions.remove(&id) {
                println!("fwd: dotaz {} ({}) vyprsel", id, q.ask.app);
                self.answer(id, self.svc.ask_allow);
            }
        }
    }

    fn answer(&self, id: u32, allow: bool) {
        let mut w = Writer::op(op::ANSWER);
        w.u32(id).u8(allow as u8);
        let _ = self.netd.write(&w.finish(), Vec::new());
    }

    /// `false` = netd zavrel kanal.
    fn netd_messages(&mut self) -> bool {
        loop {
            let msg = match self.netd.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return true,
                Err(_) => return false,
            };
            let mut r = Reader::new(&msg.data);
            match r.u32() {
                Ok(op::ASK) => {
                    if let Ok(ask) = Ask::get(&mut r) {
                        self.on_ask(ask);
                    }
                }
                Ok(op::NETWORK) => {
                    if let Ok(gw) = r.ip() {
                        let mac = r.take(6).unwrap_or(&[0; 6]).to_vec();
                        self.on_network(gw, &mac);
                    }
                }
                _ => {}
            }
        }
    }

    fn on_ask(&mut self, ask: Ask) {
        let Some(agent) = &self.agent else {
            self.answer(ask.id, self.svc.ask_allow);
            return;
        };
        let name = String::from(ask.app.as_str());
        let mut w = Writer::op(op::QUESTION);
        w.u32(ask.id)
            .str(&ask.app)
            .str(&name)
            .u8(ask.proto)
            .socket_addr(ask.dest)
            .str(&ask.host);
        if agent.write(&w.finish(), Vec::new()).is_err() {
            self.answer(ask.id, self.svc.ask_allow);
            return;
        }
        let deadline = libos::time::now().saturating_add(self.svc.ask_timeout_ns);
        self.questions.insert(ask.id, Question { ask, deadline });
    }

    fn on_network(&mut self, gateway: IpAddr, mac: &[u8]) {
        let id = mac
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");
        let gw = format!("{gateway}");
        let known = config::network_known(&self.cfg, &id);
        self.network = Some((id.clone(), gw.clone()));
        println!(
            "fwd: sit {id} (brana {gw}), profil {}{}",
            self.profile(),
            if known { "" } else { " (neznama sit)" }
        );
        self.push_config();
        if !known && let Some(agent) = &self.agent {
            let qid = self.next_id;
            self.next_id += 1;
            let mut w = Writer::op(op::NETWORK_QUESTION);
            w.u32(qid).str(&id).str(&gw);
            if agent.write(&w.finish(), Vec::new()).is_ok() {
                self.network_question = Some(qid);
            }
        }
    }

    fn agent_messages(&mut self) {
        loop {
            let Some(agent) = &self.agent else {
                return;
            };
            let msg = match agent.try_read() {
                Ok(m) => m,
                Err(Error::ShouldWait) => return,
                Err(_) => {
                    println!("fwd: agent uzivatele skoncil, dotazy maji vychozi odpoved");
                    self.agent = None;
                    let pending: Vec<u32> = self.questions.keys().copied().collect();
                    for id in pending {
                        self.questions.remove(&id);
                        self.answer(id, self.svc.ask_allow);
                    }
                    return;
                }
            };
            let mut r = Reader::new(&msg.data);
            let Ok(code) = r.u32() else {
                continue;
            };
            match code {
                op::DECIDE => {
                    if let (Ok(id), Ok(c)) = (r.u32(), r.u8()) {
                        self.on_decide(id, c);
                    }
                }
                op::CLASSIFY => {
                    if let (Ok(id), Ok(p)) = (r.u32(), r.str()) {
                        self.on_classify(id, &p);
                    }
                }
                _ => {
                    let reply = self.agent_request(code, &mut r);
                    if let Some(agent) = &self.agent {
                        let _ = agent.write(&netproto::fw::agent_reply(reply), Vec::new());
                    }
                }
            }
        }
    }

    fn on_decide(&mut self, id: u32, c: u8) {
        let Some(q) = self.questions.remove(&id) else {
            return;
        };
        let allow = matches!(c, choice::ALLOW_ONCE | choice::ALLOW_ALWAYS);
        if matches!(c, choice::ALLOW_ALWAYS | choice::DENY_ALWAYS) {
            let rule = decision_rule(&q.ask);
            let app = q.ask.app.clone();
            println!(
                "fwd: {} {} vzdy: {}",
                app,
                if allow { "povolit" } else { "zakazat" },
                manifest::target_text(&rule.target)
            );
            config::add_rule(&mut self.cfg, &app, &rule, allow);
            self.save(&|t| config::add_rule(t, &app, &rule, allow));
            // Nejdriv nova pravidla, pak odpoved: dalsi pozadavky uz se
            // nezeptaji.
            self.push_config();
        }
        self.answer(id, allow);
    }

    fn on_classify(&mut self, id: u32, profile: &str) {
        if self.network_question != Some(id) {
            return;
        }
        self.network_question = None;
        self.set_profile(profile);
    }

    fn set_profile(&mut self, profile: &str) -> bool {
        if !config::has_profile(&self.cfg, profile) {
            return false;
        }
        let Some((id, gw)) = self.network.clone() else {
            return false;
        };
        config::set_network(&mut self.cfg, &id, &gw, profile);
        let p = String::from(profile);
        self.save(&|t| config::set_network(t, &id, &gw, &p));
        self.push_config();
        true
    }

    fn agent_request(&mut self, code: u32, r: &mut Reader) -> libos::Result<Writer> {
        match code {
            op::CONFIG_GET => {
                let mut w = Writer::new();
                w.bytes(self.cfg.to_toml().as_bytes());
                Ok(w)
            }
            op::CONFIG_SET => {
                let text = core::str::from_utf8(r.rest()).map_err(|_| Error::InvalidArgs)?;
                let t = conf::parse(text).map_err(|e| {
                    println!("fwd: nova konfigurace: {e}");
                    Error::InvalidArgs
                })?;
                let svc = config::service(&t).map_err(|_| Error::InvalidArgs)?;
                let profile = config::profile_for(&t, self.network.as_ref().map(|(id, _)| id.as_str()));
                config::build(&t, &profile).map_err(|e| {
                    println!("fwd: nova konfigurace: {e}");
                    Error::InvalidArgs
                })?;
                self.cfg = t;
                self.svc = svc;
                let copy = self.cfg.clone();
                self.save(&|t| *t = copy.clone());
                self.push_config();
                Ok(Writer::new())
            }
            op::PROFILE_SET => {
                let p = r.str()?;
                if self.network.is_none() {
                    return Err(Error::BadState);
                }
                if self.set_profile(&p) {
                    Ok(Writer::new())
                } else {
                    Err(Error::NotFound)
                }
            }
            _ => Err(Error::NotSupported),
        }
    }
}

/// Pravidlo pro trvale rozhodnuti: protokol a port dotazu, cil = jmeno
/// hostitele (kdyz ho relace znala), jinak presna adresa.
fn decision_rule(a: &Ask) -> Rule {
    let target = if a.host.is_empty() {
        let full = if a.dest.is_ipv4() { 32 } else { 128 };
        Target::Net {
            addr: a.dest.ip(),
            prefix: full,
        }
    } else {
        Target::Host(a.host.clone())
    };
    let port = if a.proto == netproto::proto::ICMP {
        (0, u16::MAX)
    } else {
        (a.dest.port(), a.dest.port())
    };
    Rule {
        protos: a.proto,
        target,
        ports: port,
    }
}
