//! ntpd: klient NTP, ktery navrhuje korekce hodin sluzbe casu.
//!
//! Start (kind USER, v tomto poradi):
//! 1. relace site (init ji omezi na nakonfigurovane servery: TCP na NTS-KE,
//!    UDP na NTP, a DNS),
//! 2. kanal k `timed` pro zpravy `sync` (jen navrh korekce; nastavovat
//!    hodiny ntpd nesmi, overeni a provedeni je na `timed`),
//! 3. (volitelne) relace FS s ulozistem certifikatu `/certs` (TLS pro NTS-KE),
//! 4. (volitelne) kanal k `revocd` (odvolani certifikatu serveru NTS-KE).
//!
//! Na bootstrap kanalu prijde od init konfigurace (TOML):
//!
//! ```toml
//! # nts://jmeno[:port] = overene NTS (RFC 8915, NTS-KE port 4460),
//! # jmeno nebo adresa[:port] = neoverene NTP (port 123)
//! servers = ["nts://time.cloudflare.com", "pool.ntp.org"]
//! unauthenticated = "profile" # neoverene servery: "never" / "fallback" /
//!                             # "profile" (podle profilu site, vychozi)
//! min_poll = 16               # s
//! max_poll = 1024
//! samples = 4                 # mereni v jedne davce
//! ```
//!
//! S `never` se neoverene servery nepouzijou vubec; s `fallback` jen, kdyz
//! v kole neodpovi zadny server NTS; s `profile` podle profilu site (pravidlo
//! `unauthenticated` profilu ve firewallu, verejna sit = never). Zprava
//! `policy ...` pravidlo zmeni za behu.
//!
//! ntpd odpovi `ok` a hned meri. Kazde kolo: z kazdeho serveru davka
//! mereni, z ni vzorek s nejmensim zpozdenim, ze serveru median; posun
//! (a odhad frekvence) jde do `timed`. Interval se prodluzuje, kdyz jsou
//! hodiny srovnane (`netproto::ntp::Discipline`). Kolo bez mereni (pri
//! startu jeste nema sit adresu nebo DNS) se opakuje za 2, 4, 8, ... s,
//! nejvys za `min_poll` - hodiny se tak nastavi do par sekund po startu
//! site, ne az za `min_poll`.
//!
//! Dalsi zpravy od init: `status` -> `ok:` + text stavu, `policy ...`.
//!
//! Neoverene NTP muze podvrhnout kazdy, kdo ovlada sit. Proto ntpd nema
//! pravo k hodinam a `timed` korekce omezuje i u NTS.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

use fsproto::FsClient;
use libos::{Channel, Error, Signals, Startup, println, time};
use netproto::Net;
use netproto::ntp::{self, Discipline, Sample};

mod nts;

libos::entry!(main);

/// Server z konfigurace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Server {
    /// NTS-KE na jmenu a portu.
    Nts(String, u16),
    /// Neoverene NTP (`jmeno`, `adresa` nebo `adresa:port`).
    Plain(String),
}

/// Kdy pouzit neoverene servery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unauthenticated {
    Never,
    Fallback,
    /// Podle profilu site (`Net::info`, pravidlo firewallu).
    Profile,
}

impl Unauthenticated {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "never" => Some(Self::Never),
            "fallback" => Some(Self::Fallback),
            "profile" => Some(Self::Profile),
            _ => None,
        }
    }
}

struct Config {
    servers: Vec<Server>,
    unauthenticated: Unauthenticated,
    min_poll: u64,
    max_poll: u64,
    samples: usize,
}

pub fn parse_server(s: &str) -> Option<Server> {
    let Some(rest) = s.strip_prefix("nts://") else {
        return (!s.is_empty()).then(|| Server::Plain(String::from(s)));
    };
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse().ok()?),
        _ => (rest, netproto::nts::KE_PORT),
    };
    (!host.is_empty()).then(|| Server::Nts(String::from(host), port))
}

fn parse_config(text: &str) -> Result<Config, String> {
    let t = conf::parse(text).map_err(|e| format!("{e}"))?;
    let mut servers = Vec::new();
    for s in t.get_strs("servers").ok_or("chybi servers")? {
        servers.push(parse_server(s).ok_or_else(|| format!("neplatny server {s}"))?);
    }
    let unauthenticated = match t.get_str("unauthenticated") {
        None => Unauthenticated::Profile,
        Some(v) => Unauthenticated::parse(v).ok_or("unauthenticated: never, fallback nebo profile")?,
    };
    if servers.is_empty() {
        return Err(String::from("prazdny seznam serveru"));
    }
    let int = |k: &str, d: i64, lo: i64, hi: i64| -> Result<i64, String> {
        let v = t.get_int(k).unwrap_or(d);
        if (lo..=hi).contains(&v) {
            Ok(v)
        } else {
            Err(format!("{k}: {lo}..={hi}"))
        }
    };
    let min_poll = int("min_poll", 16, 1, 36 * 3600)? as u64;
    let max_poll = int("max_poll", 1024, min_poll as i64, 36 * 3600)? as u64;
    Ok(Config {
        servers,
        unauthenticated,
        min_poll,
        max_poll,
        samples: int("samples", 4, 1, 16)? as usize,
    })
}

/// `jmeno`, `adresa` nebo `adresa:port` (vychozi port 123).
pub fn resolve(net: &Net, server: &str) -> Result<SocketAddr, Error> {
    if let Ok(sa) = server.parse::<SocketAddr>() {
        return Ok(sa);
    }
    let (host, port) = match server.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse().map_err(|_| Error::InvalidArgs)?),
        _ => (server, ntp::PORT),
    };
    let addrs = net.resolve(host)?;
    let ip = addrs.first().ok_or(Error::NotFound)?;
    Ok(SocketAddr::new(*ip, port))
}

struct State {
    rounds: u64,
    last_offset: i128,
    last_delay: i128,
    last_error: Option<Error>,
    last_reply: String,
    /// Odkud bylo posledni mereni: "nts", "plain" nebo "-".
    last_auth: &'static str,
    last_nts_error: String,
    /// Posledni vypsany duvod kola bez mereni.
    last_failure: String,
}

fn main(startup: Startup) -> i64 {
    let ctl = startup.bootstrap;
    let mut handles = startup.handles.into_iter();
    let (Some(net), Some(timed)) = (handles.next(), handles.next()) else {
        let _ = ctl.write(b"err:chybi handly", Vec::new());
        return 1;
    };
    let net = Net::new(Channel::from_handle(net));
    let timed = Channel::from_handle(timed);
    let fs = handles.next().map(|h| FsClient::new(Channel::from_handle(h)));
    let revocation = handles.next().map(|h| {
        alloc::sync::Arc::new(tls::revocation::ChannelSource::new(Channel::from_handle(h), Duration::from_secs(20)))
            as alloc::sync::Arc<dyn tls::revocation::RevocationSource>
    });
    let roots = web::load_roots(fs.as_ref());
    let tls = match tls::Client::with_revocation(&tls::LIBOS, &roots, &[netproto::nts::ALPN], revocation) {
        Ok(c) => c,
        Err(e) => {
            println!("ntpd: TLS: {e}");
            return 4;
        }
    };
    let cfg = match ctl
        .read(Some(time::deadline_after(Duration::from_secs(5))))
        .map_err(|e| format!("{e}"))
        .and_then(|m| String::from_utf8(m.data).map_err(|_| String::from("neni UTF-8")))
        .and_then(|t| parse_config(&t))
    {
        Ok(c) => c,
        Err(e) => {
            println!("ntpd: konfigurace: {e}");
            let _ = ctl.write(format!("err:{e}").as_bytes(), Vec::new());
            return 2;
        }
    };
    let mut cfg = cfg;
    let _ = ctl.write(b"ok", Vec::new());
    println!(
        "ntpd: bezim, servery {:?}, neoverene {:?}, CA {}",
        cfg.servers,
        cfg.unauthenticated,
        roots.len()
    );
    let mut sessions: Vec<Option<nts::Session>> = cfg.servers.iter().map(|_| None).collect();

    let mut disc = Discipline::new(cfg.min_poll, cfg.max_poll);
    let mut st = State {
        rounds: 0,
        last_offset: 0,
        last_delay: 0,
        last_error: None,
        last_reply: String::new(),
        last_auth: "-",
        last_nts_error: String::new(),
        last_failure: String::new(),
    };
    let mut next = time::now();
    let mut retry_s = RETRY_FIRST_S;
    loop {
        if time::now() >= next {
            let wait_s = if round(&net, &tls, &timed, &cfg, &mut sessions, &mut disc, &mut st) {
                retry_s = RETRY_FIRST_S;
                disc.poll_secs
            } else {
                let w = retry_s.min(cfg.min_poll);
                retry_s = (retry_s * 2).min(cfg.min_poll);
                w
            };
            next = time::now() + wait_s * 1_000_000_000;
        }
        // Cekat na dalsi kolo, prikazy od init nebo konec.
        match libos::wait_one(ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED, Some(next)) {
            Ok(s) if s.contains(Signals::READABLE) => {
                if let Ok(m) = ctl.try_read() {
                    let reply = if m.data == b"status" {
                        format!(
                            "ok:rounds={} offset_ns={} delay_ns={} freq_ppb={} poll_s={} error={} timed={} auth={} policy={:?} nts_error={}",
                            st.rounds,
                            st.last_offset,
                            st.last_delay,
                            disc.freq_ppb,
                            disc.poll_secs,
                            st.last_error.map_or("-", |e| e.name()),
                            st.last_reply,
                            st.last_auth,
                            cfg.unauthenticated,
                            if st.last_nts_error.is_empty() { "-" } else { &st.last_nts_error }
                        )
                    } else if let Some(p) = m.data.strip_prefix(b"policy ") {
                        match core::str::from_utf8(p).ok().and_then(Unauthenticated::parse) {
                            Some(u) => {
                                if u != cfg.unauthenticated {
                                    println!("ntpd: neoverene servery: {u:?}");
                                    cfg.unauthenticated = u;
                                    // Nove pravidlo plati hned.
                                    next = time::now();
                                }
                                String::from("ok")
                            }
                            None => String::from("err:never, fallback nebo profile"),
                        }
                    } else {
                        String::from("err:neznama zprava")
                    };
                    let _ = ctl.write(reply.as_bytes(), Vec::new());
                }
            }
            Ok(s) if s.contains(Signals::PEER_CLOSED) => return 0,
            Ok(_) | Err(Error::TimedOut) => {}
            Err(e) => {
                println!("ntpd: cekani: {e}");
                return 3;
            }
        }
    }
}

/// Mereni pres NTS (pripadne nove NTS-KE).
fn measure_nts(
    net: &Net,
    tls: &tls::Client,
    host: &str,
    port: u16,
    session: &mut Option<nts::Session>,
    samples: usize,
    st: &mut State,
) -> Vec<Sample> {
    let mut out = Vec::new();
    for _ in 0..samples {
        if session.as_ref().is_none_or(|s| !s.has_cookie()) {
            match nts::key_exchange(net, tls, host, port) {
                Ok(s) => *session = Some(s),
                Err(e) => {
                    if st.last_nts_error != e {
                        println!("ntpd: {e}");
                    }
                    st.last_nts_error = e;
                    *session = None;
                    return out;
                }
            }
        }
        let s = session.as_mut().expect("sezeni po NTS-KE");
        match s.query(net, Duration::from_secs(3)) {
            Ok(m) => out.push(Sample {
                offset_ns: m.offset_ns,
                delay_ns: m.delay_ns,
            }),
            Err(netproto::nts::Error::CookieRejected) => *session = None,
            Err(e) => st.last_nts_error = format!("{host}: {e:?}"),
        }
    }
    out
}

fn measure_plain(net: &Net, server: &str, samples: usize, st: &mut State) -> Vec<Sample> {
    let addr = match resolve(net, server) {
        Ok(a) => a,
        Err(e) => {
            st.last_error = Some(e);
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for _ in 0..samples {
        match net.ntp_query(addr, Duration::from_secs(3)) {
            Ok(s) => out.push(Sample {
                offset_ns: s.offset_ns,
                delay_ns: s.delay_ns,
            }),
            Err(e) => st.last_error = Some(e),
        }
    }
    out
}

/// Prvni opakovani kola bez mereni (s); kazde dalsi dvojnasob, nejvys
/// `min_poll`.
const RETRY_FIRST_S: u64 = 2;

/// Jedno kolo: mereni (nejdriv NTS, neoverene jen podle pravidla), vyber,
/// navrh korekce. `false` = zadne mereni.
fn round(
    net: &Net,
    tls: &tls::Client,
    timed: &Channel,
    cfg: &Config,
    sessions: &mut [Option<nts::Session>],
    disc: &mut Discipline,
    st: &mut State,
) -> bool {
    let mut offsets = Vec::new();
    let mut delay = 0;
    for (i, server) in cfg.servers.iter().enumerate() {
        if let Server::Nts(host, port) = server
            && let Some(best) = ntp::best_sample(&measure_nts(net, tls, host, *port, &mut sessions[i], cfg.samples, st))
        {
            offsets.push(best.offset_ns);
            delay = best.delay_ns;
        }
    }
    let has_nts = cfg.servers.iter().any(|s| matches!(s, Server::Nts(..)));
    let fallback = match cfg.unauthenticated {
        Unauthenticated::Never => false,
        Unauthenticated::Fallback => true,
        // Pravidlo profilu site (verejna sit = ne); bez odpovedi = ne.
        Unauthenticated::Profile => net.info().is_ok_and(|i| i.unauthenticated_fallback),
    };
    let auth = if !offsets.is_empty() {
        "nts"
    } else if fallback || !has_nts {
        for server in &cfg.servers {
            if let Server::Plain(name) = server
                && let Some(best) = ntp::best_sample(&measure_plain(net, name, cfg.samples, st))
            {
                offsets.push(best.offset_ns);
                delay = best.delay_ns;
            }
        }
        "plain"
    } else {
        "-"
    };
    let Some(offset) = ntp::combine(&mut offsets) else {
        // Pri rychlem opakovani (start) stejnou hlasku jen jednou.
        let why = format!("{:?}, NTS: {}", st.last_error, st.last_nts_error);
        if why != st.last_failure {
            println!("ntpd: zadny server neodpovedel ({why})");
            st.last_failure = why;
        }
        return false;
    };
    st.last_auth = auth;
    let freq = disc.update(time::now(), offset);
    let mut msg = Vec::with_capacity(21);
    msg.extend_from_slice(b"sync");
    msg.extend_from_slice(&(offset.clamp(i64::MIN as i128, i64::MAX as i128) as i64).to_le_bytes());
    msg.extend_from_slice(&freq.unwrap_or(0).to_le_bytes());
    msg.push(freq.is_some() as u8);
    let reply = timed
        .write(&msg, Vec::new())
        .and_then(|_| timed.read(Some(time::deadline_after(Duration::from_secs(5)))));
    st.rounds += 1;
    st.last_failure.clear();
    st.last_offset = offset;
    st.last_delay = delay;
    st.last_reply = match reply {
        // "ok:step" / "ok:slew" + 24 B stavu, nebo "err:<popis>".
        Ok(m) if m.data.starts_with(b"ok:") => String::from_utf8_lossy(&m.data[..m.data.len().min(7)]).into_owned(),
        Ok(m) => String::from_utf8_lossy(&m.data).into_owned(),
        Err(e) => format!("err:{e}"),
    };
    if st.rounds == 1 {
        println!(
            "ntpd: prvni mereni ({}): posun {} ms, zpozdeni {} ms, timed {}",
            if auth == "nts" { "NTS" } else { "neoverene" },
            offset / 1_000_000,
            delay / 1_000_000,
            st.last_reply
        );
    }
    true
}
