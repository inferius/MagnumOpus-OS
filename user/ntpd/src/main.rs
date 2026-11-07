//! ntpd: klient NTP, ktery navrhuje korekce hodin sluzbe casu.
//!
//! Start (kind USER, v tomto poradi):
//! 1. relace site (init ji omezi jen na UDP k nakonfigurovanym serverum
//!    a DNS),
//! 2. kanal k `timed` pro zpravy `sync` (jen navrh korekce; nastavovat
//!    hodiny ntpd nesmi, overeni a provedeni je na `timed`).
//!
//! Na bootstrap kanalu prijde od init konfigurace (TOML):
//!
//! ```toml
//! servers = ["pool.ntp.org", "10.0.2.2:123"]  # jmeno nebo adresa[:port]
//! min_poll = 16                                # s
//! max_poll = 1024
//! samples = 4                                  # mereni v jedne davce
//! ```
//!
//! ntpd odpovi `ok` a hned meri. Kazde kolo: z kazdeho serveru davka
//! mereni, z ni vzorek s nejmensim zpozdenim, ze serveru median; posun
//! (a odhad frekvence) jde do `timed`. Interval se prodluzuje, kdyz jsou
//! hodiny srovnane (`netproto::ntp::Discipline`).
//!
//! Dalsi zpravy od init: `status` -> `ok:` + text stavu.
//!
//! POZOR: NTP neni overene (bez NTS): kdo ovlada sit, muze cas posunout. Proto
//! ntpd nema pravo k hodinam a `timed` korekce omezuje.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

use libos::{Channel, Error, Signals, Startup, println, time};
use netproto::Net;
use netproto::ntp::{self, Discipline, Sample};

libos::entry!(main);

struct Config {
    servers: Vec<String>,
    min_poll: u64,
    max_poll: u64,
    samples: usize,
}

fn parse_config(text: &str) -> Result<Config, String> {
    let t = conf::parse(text).map_err(|e| format!("{e}"))?;
    let servers: Vec<String> = t
        .get_strs("servers")
        .ok_or("chybi servers")?
        .into_iter()
        .map(String::from)
        .collect();
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
    let _ = ctl.write(b"ok", Vec::new());
    println!("ntpd: bezim, servery {:?}", cfg.servers);

    let mut disc = Discipline::new(cfg.min_poll, cfg.max_poll);
    let mut st = State {
        rounds: 0,
        last_offset: 0,
        last_delay: 0,
        last_error: None,
        last_reply: String::new(),
    };
    let mut next = time::now();
    loop {
        if time::now() >= next {
            round(&net, &timed, &cfg, &mut disc, &mut st);
            next = time::now() + disc.poll_secs * 1_000_000_000;
        }
        // Cekat na dalsi kolo, prikazy od init nebo konec.
        match libos::wait_one(ctl.handle(), Signals::READABLE | Signals::PEER_CLOSED, Some(next)) {
            Ok(s) if s.contains(Signals::READABLE) => {
                if let Ok(m) = ctl.try_read() {
                    let reply = if m.data == b"status" {
                        format!(
                            "ok:rounds={} offset_ns={} delay_ns={} freq_ppb={} poll_s={} error={} timed={}",
                            st.rounds,
                            st.last_offset,
                            st.last_delay,
                            disc.freq_ppb,
                            disc.poll_secs,
                            st.last_error.map_or("-", |e| e.name()),
                            st.last_reply
                        )
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

/// Jedno kolo: mereni, vyber, navrh korekce.
fn round(net: &Net, timed: &Channel, cfg: &Config, disc: &mut Discipline, st: &mut State) {
    let mut offsets = Vec::new();
    let mut delay = 0;
    for server in &cfg.servers {
        let addr = match resolve(net, server) {
            Ok(a) => a,
            Err(e) => {
                st.last_error = Some(e);
                continue;
            }
        };
        let mut samples = Vec::new();
        for _ in 0..cfg.samples {
            match net.ntp_query(addr, Duration::from_secs(3)) {
                Ok(s) => samples.push(Sample {
                    offset_ns: s.offset_ns,
                    delay_ns: s.delay_ns,
                }),
                Err(e) => st.last_error = Some(e),
            }
        }
        if let Some(best) = ntp::best_sample(&samples) {
            offsets.push(best.offset_ns);
            delay = best.delay_ns;
        }
    }
    let Some(offset) = ntp::combine(&mut offsets) else {
        println!("ntpd: zadny server neodpovedel ({:?})", st.last_error);
        return;
    };
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
            "ntpd: prvni mereni: posun {} ms, zpozdeni {} ms, timed {}",
            offset / 1_000_000,
            delay / 1_000_000,
            st.last_reply
        );
    }
}
