//! Klient NTS (RFC 8915): NTS-KE pres TLS a overene dotazy NTP.
//!
//! Pro kazdy server `nts://jmeno[:port]` ntpd drzi klice (C2S, S2C) a sadu
//! cookies. Kazdy dotaz spotrebuje jednu cookie a zastupci si rekne
//! o doplneni na [`nts::COOKIES`]. Kdyz cookies dojdou nebo server cookie
//! odmitne (NTSN), probehne nove NTS-KE.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::time::Duration;

use libos::{Error, time};
use netproto::Net;
use netproto::io::{Read, Write};
use netproto::nts::{self, KEY_LEN};

/// Sezeni se serverem NTS.
pub struct Session {
    pub ntp: SocketAddr,
    c2s: [u8; KEY_LEN],
    s2c: [u8; KEY_LEN],
    cookies: VecDeque<Vec<u8>>,
}

/// Mereni pres NTS.
pub struct Measured {
    pub offset_ns: i128,
    pub delay_ns: i128,
}

fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut b = [0u8; N];
    libos::random::fill_seeded(&mut b)?;
    Ok(b)
}

/// NTS-KE s `host:port`: klice a cookies. Chyba = popis pro log a stav.
pub fn key_exchange(net: &Net, tls: &tls::Client, host: &str, port: u16) -> Result<Session, String> {
    let tcp = web::tcp(net, host, port, Duration::from_secs(10)).map_err(|e| format!("NTS-KE {host}:{port}: {e}"))?;
    let mut s = tls.connect(host, tcp).map_err(|e| format!("NTS-KE {host}: {e}"))?;
    if s.alpn_protocol() != Some(nts::ALPN) {
        return Err(format!("NTS-KE {host}: server nevybral ALPN ntske/1"));
    }
    s.write_all(&nts::ke_request()).map_err(|e| format!("NTS-KE {host}: {e}"))?;
    let mut data = Vec::new();
    let reply = loop {
        match nts::parse_ke_reply(&data) {
            Ok(Some(r)) => break r,
            Ok(None) => {}
            Err(e) => return Err(format!("NTS-KE {host}: {e:?}")),
        }
        if data.len() > nts::MAX_KE_REPLY {
            return Err(format!("NTS-KE {host}: prilis dlouha odpoved"));
        }
        let mut buf = [0u8; 4096];
        match s.read(&mut buf) {
            Ok(0) => return Err(format!("NTS-KE {host}: spojeni skoncilo pred koncem odpovedi")),
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(e) => return Err(format!("NTS-KE {host}: {e}")),
        }
    };
    let mut c2s = [0u8; KEY_LEN];
    let mut s2c = [0u8; KEY_LEN];
    s.export_keying_material(nts::EXPORTER_LABEL, &nts::key_context(false), &mut c2s)
        .and_then(|_| s.export_keying_material(nts::EXPORTER_LABEL, &nts::key_context(true), &mut s2c))
        .map_err(|e| format!("NTS-KE {host}: {e}"))?;
    let _ = s.close();
    let ntp_host = reply.server.as_deref().unwrap_or(host);
    let ntp_port = reply.port.unwrap_or(netproto::ntp::PORT);
    let ip = match ntp_host.parse::<core::net::IpAddr>() {
        Ok(ip) => ip,
        Err(_) => *net
            .resolve(ntp_host)
            .map_err(|e| format!("NTS: server NTP {ntp_host}: {e}"))?
            .first()
            .ok_or_else(|| format!("NTS: server NTP {ntp_host}: bez adresy"))?,
    };
    let ntp = SocketAddr::new(ip, ntp_port);
    Ok(Session {
        ntp,
        c2s,
        s2c,
        cookies: reply.cookies.into(),
    })
}

impl Session {
    pub fn has_cookie(&self) -> bool {
        !self.cookies.is_empty()
    }

    /// Jeden overeny dotaz. `Err(CookieRejected)` = sezeni zahodit.
    pub fn query(&mut self, net: &Net, timeout: Duration) -> Result<Measured, nts::Error> {
        let cookie = self.cookies.pop_front().ok_or(nts::Error::CookieRejected)?;
        let placeholders = nts::COOKIES.saturating_sub(self.cookies.len() + 1);
        let uid: [u8; 32] = random().map_err(|_| nts::Error::Protocol("bez nahody"))?;
        let nonce: [u8; 16] = random().map_err(|_| nts::Error::Protocol("bez nahody"))?;
        let sock = net.udp_bind(0).map_err(|_| nts::Error::Protocol("socket"))?;
        let t1 = time::wall().ok_or(nts::Error::Protocol("cas neni znamy"))?;
        let req = nts::build_request(t1, &uid, &cookie, placeholders, &self.c2s, &nonce);
        sock.send_to(&req, self.ntp).map_err(|_| nts::Error::Protocol("odeslani"))?;
        let deadline = time::now() + timeout.as_nanos() as u64;
        loop {
            let now = time::now();
            if now >= deadline {
                return Err(nts::Error::NotOurs);
            }
            let mut buf = [0u8; 2048];
            let Ok((n, from)) = sock.recv_from(&mut buf, Some(Duration::from_nanos(deadline - now))) else {
                return Err(nts::Error::NotOurs);
            };
            let t4 = time::wall().ok_or(nts::Error::Protocol("cas neni znamy"))?;
            if from != self.ntp {
                continue;
            }
            match nts::parse_reply(t1, &uid, &self.s2c, &buf[..n]) {
                Ok(r) => {
                    for c in r.cookies {
                        if self.cookies.len() < nts::COOKIES {
                            self.cookies.push_back(c);
                        }
                    }
                    let (offset, delay) = netproto::ntp::offset_delay(t1, &r.ntp, t4);
                    return Ok(Measured {
                        offset_ns: offset,
                        delay_ns: delay,
                    });
                }
                // Cizi nebo podvrzena odpoved: cekat dal na pravou.
                Err(nts::Error::NotOurs | nts::Error::Unauthenticated) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}
