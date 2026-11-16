//! sshd: server SSH (`docs/cs/ssh.md`). Jen nasloucha a pro kazde
//! spojeni spusti proces `sshd-conn` (data ze site cte az on).
//!
//! Start (kind USER, v tomto poradi):
//! 1. relace site (init ji omezi na naslouchani na portu 22, zadne
//!    odchozi spojeni),
//! 2. kanal `ssh` sluzby uctu (overeni klicem; podpis overuje accountd),
//! a `kind::INITFS` (obraz `sshd-conn`).
//!
//! Na bootstrap kanalu prijde od initu konfigurace `port=22\nkey=<hex>`
//! (soukromy klic hostitele ed25519); sshd odpovi `ok`. Stejnym kanalem
//! pak sshd vyzvedava vzdalene relace u initu (`claim <listek>`) a hlasi
//! jejich konec (`end <relace>`).
//!
//! Na spojeni: proces v jobu s limitem pameti, kanal k sshd. Pozadavky
//! procesu (overeni) sshd preda sluzbe uctu; listek relace si necha
//! a relaci vyzvedne sam, procesu da jen kanal okna terminalu a relaci
//! FS uctu. Audit (kdy, odkud, jak dlouho; nikdy obsah) jde do logu.

#![no_std]
#![no_main]

extern crate alloc;

mod proto;

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use accountproto::client::SshAuth;
use accountproto::ssh::SshAuthReply;
use libos::initfs::Initfs;
use libos::job::Job;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Handle, Process, Signals, Startup, println, time};
use netproto::{Deadline, Net, TcpListener};

use proto::Req;

/// Nejvic soubeznych spojeni (i neoverenych).
const MAX_CONNS: usize = 8;
/// Pamet procesu spojeni (sunset, terminal, buffery).
const CONN_MEMORY: u64 = 24 << 20;
/// Neoverene spojeni smi trvat nejvys tolik (OpenSSH LoginGraceTime).
const LOGIN_GRACE: Duration = Duration::from_secs(120);

struct Conn {
    process: Process,
    chan: Channel,
    peer: String,
    started: u64,
    /// Listek vzdalene relace (od sluzby uctu, procesu se nedava).
    ticket: Option<Vec<u8>>,
    /// Vyzvednuta relace (cislo u initu).
    session: Option<u64>,
    user: String,
}

fn hex_key(cfg: &str) -> Option<[u8; 32]> {
    let line = cfg.lines().find_map(|l| l.strip_prefix("key="))?;
    proto::unhex(line.trim())?.try_into().ok()
}

fn port(cfg: &str) -> u16 {
    cfg.lines().find_map(|l| l.strip_prefix("port=")).and_then(|p| p.trim().parse().ok()).unwrap_or(22)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Spusti proces spojeni.
fn spawn_conn(initfs: &Initfs, stream: Channel, key: &[u8; 32], peer: &str) -> Result<(Process, Channel), Error> {
    let image = initfs.find("sshd-conn").ok_or(Error::NotFound)?;
    let job = Job::create(&libos::job::current()?)?;
    job.set_limit(libos::abi::job::pool::USER, CONN_MEMORY)?;
    let (ours, theirs) = Channel::create()?;
    let handles = vec![StartHandle::user(stream.into_handle()), StartHandle::user(theirs.into_handle())];
    let (process, ctl) = libos::spawn::spawn_with(&job, "sshd-conn", image, handles, Default::default())?;
    let cfg = format!("key={}\npeer={peer}\n", hex(key));
    ctl.write(cfg.as_bytes(), Vec::new())?;
    Ok((process, ours))
}

/// Pozadavek procesu spojeni.
fn request(c: &mut Conn, auth: &SshAuth, init: &Channel, data: &[u8]) -> (Vec<u8>, Vec<Handle>) {
    let req = match Req::decode(data) {
        Ok(r) => r,
        Err(_) => return (b"err INVALID_ARGS".to_vec(), Vec::new()),
    };
    match req {
        Req::Probe { user, key } => {
            let ok = auth.probe(&user, &c.peer, &key).unwrap_or(false);
            (vec![ok as u8], Vec::new())
        }
        Req::Auth { user, key, data, sig } => {
            if c.ticket.is_some() || c.session.is_some() {
                return (vec![proto::AUTH_DENIED], Vec::new());
            }
            let r = match auth.auth(&user, &c.peer, &key, &data, &sig) {
                Ok(SshAuthReply::Ok { ticket, .. }) => {
                    c.ticket = Some(ticket);
                    c.user = user;
                    proto::AUTH_OK
                }
                Ok(SshAuthReply::Locked) => proto::AUTH_LOCKED,
                _ => proto::AUTH_DENIED,
            };
            (vec![r], Vec::new())
        }
        Req::Claim => {
            let Some(ticket) = c.ticket.take() else { return (b"err ACCESS_DENIED".to_vec(), Vec::new()) };
            let mut msg = b"claim ".to_vec();
            msg.extend_from_slice(&ticket);
            let r = init.write(&msg, Vec::new()).and_then(|_| init.read(Some(time::deadline_after(Duration::from_secs(10)))));
            match r {
                Ok(m) if m.data.starts_with(b"ok ") => {
                    c.session = core::str::from_utf8(&m.data[3..]).ok().and_then(|s| s.split_whitespace().next()?.parse().ok());
                    (m.data, m.handles)
                }
                Ok(m) => (m.data, Vec::new()),
                Err(e) => (format!("err {}", e.name()).into_bytes(), Vec::new()),
            }
        }
    }
}

/// Konec spojeni: audit a konec vzdalene relace u initu.
fn finish(c: Conn, init: &Channel) {
    let secs = time::now().saturating_sub(c.started) / 1_000_000_000;
    let _ = c.process.kill();
    match c.session {
        Some(s) => {
            println!("sshd: odpojen {} z {} po {secs} s (relace {s})", c.user, c.peer);
            let _ = init.write(format!("end {s}").as_bytes(), Vec::new());
            let _ = init.read(Some(time::deadline_after(Duration::from_secs(5))));
        }
        None => println!("sshd: spojeni z {} skoncilo bez prihlaseni po {secs} s", c.peer),
    }
}

libos::entry!(main);

fn main(mut startup: Startup) -> i64 {
    let init = startup.bootstrap;
    let initfs = match startup.initfs.as_ref().map(Initfs::from_vmo) {
        Some(Ok(f)) => f,
        _ => {
            let _ = init.write(b"err:chybi initfs", Vec::new());
            return 1;
        }
    };
    let mut handles = core::mem::take(&mut startup.handles).into_iter();
    let (Some(net), Some(auth)) = (handles.next(), handles.next()) else {
        let _ = init.write(b"err:chybi handly", Vec::new());
        return 1;
    };
    let net = Net::new(Channel::from_handle(net));
    let auth = SshAuth(Channel::from_handle(auth));
    let cfg = match init.read(Some(time::deadline_after(Duration::from_secs(5)))) {
        Ok(m) => String::from_utf8(m.data).unwrap_or_default(),
        Err(_) => return 2,
    };
    let Some(key) = hex_key(&cfg) else {
        let _ = init.write(b"err:chybi klic", Vec::new());
        return 3;
    };
    let port = port(&cfg);
    let listener: TcpListener = match net.tcp_listen(port, 8) {
        Ok(l) => l,
        Err(e) => {
            let _ = init.write(format!("err:naslouchani na {port}: {e}").as_bytes(), Vec::new());
            return 4;
        }
    };
    let ready = match listener.readiness() {
        Ok(r) => r,
        Err(e) => {
            let _ = init.write(format!("err:pripravenost: {e}").as_bytes(), Vec::new());
            return 5;
        }
    };
    let _ = init.write(b"ok", Vec::new());
    println!("sshd: posloucham na portu {port}");
    let mut conns: Vec<Conn> = Vec::new();
    let mut waits = match libos::WaitSet::new() {
        Ok(w) => w,
        Err(_) => return 6,
    };
    loop {
        let rd = Signals::READABLE | Signals::PEER_CLOSED;
        let mut items: Vec<(&Handle, Signals)> = vec![(ready.handle(), netproto::ready::READABLE)];
        for c in &conns {
            items.push((c.chan.handle(), rd));
            items.push((c.process.handle(), Signals::TERMINATED));
        }
        // Neoverena spojeni po LOGIN_GRACE ukoncit (probuzeni na prvni
        // termin).
        let grace = LOGIN_GRACE.as_nanos() as u64;
        let deadline = conns.iter().filter(|c| c.session.is_none()).map(|c| c.started + grace).min();
        let observed = match waits.wait(&items, deadline) {
            Ok((_, o)) => o,
            Err(Error::TimedOut) => Vec::new(),
            Err(e) => {
                println!("sshd: cekani: {e}");
                return 7;
            }
        };
        drop(items);
        // Nova spojeni.
        if observed.first().is_some_and(|s| s.intersects(netproto::ready::READABLE)) {
            while let Ok((stream, peer)) = listener.accept_ctx(Deadline::TRY, None) {
                let peer = format!("{}", peer.ip());
                if conns.len() >= MAX_CONNS {
                    println!("sshd: odmitnuto spojeni z {peer} (prilis mnoho spojeni)");
                    continue;
                }
                let ch = match stream.into_channel() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                match spawn_conn(&initfs, ch, &key, &peer) {
                    Ok((process, chan)) => {
                        println!("sshd: spojeni z {peer}");
                        conns.push(Conn { process, chan, peer, started: time::now(), ticket: None, session: None, user: String::new() });
                    }
                    Err(e) => println!("sshd: proces spojeni z {peer}: {e}"),
                }
            }
        }
        // Pozadavky a konce procesu spojeni (od konce, at indexy plati).
        let now = time::now();
        for i in (0..conns.len()).rev() {
            let s_chan = observed.get(1 + 2 * i).copied().unwrap_or(Signals::NONE);
            let s_proc = observed.get(2 + 2 * i).copied().unwrap_or(Signals::NONE);
            if s_chan.contains(Signals::READABLE) {
                while let Ok(m) = conns[i].chan.try_read() {
                    let (reply, hs) = request(&mut conns[i], &auth, &init, &m.data);
                    let _ = conns[i].chan.write(&reply, hs);
                }
            }
            let expired = conns[i].session.is_none() && now >= conns[i].started + grace;
            if s_proc.contains(Signals::TERMINATED) || s_chan.contains(Signals::PEER_CLOSED) || expired {
                if expired {
                    println!("sshd: spojeni z {} bez prihlaseni vyprselo", conns[i].peer);
                }
                let c = conns.swap_remove(i);
                finish(c, &init);
            }
        }
    }
}
