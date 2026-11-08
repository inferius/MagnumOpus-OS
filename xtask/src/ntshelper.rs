//! Testovaci server NTS (RFC 8915) na hostiteli: NTS-KE pres TLS
//! (rustls + ring, ALPN `ntske/1`, certifikat testovaci CA pro 10.0.2.2)
//! a NTP s rozsirenimi NTS na UDP.
//!
//! Klice NTP z TLS exportu pocita rustls na serveru (nezavisle na
//! `libs/tls` klienta); zaznamy a AES-SIV jsou ze `netproto::nts`.
//! Odpoved NTS-KE nese 8 cookies a port NTP (zaznam Port Negotiation).
//! Pozadavek s neplatnou cookie dostane NTSN ("kiss of death").

use std::io::{Read, Write};
use std::net::UdpSocket;
use std::sync::Arc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use netproto::nts::{self, server};
use netproto::ntp;

pub struct NtsHelper {
    /// Radky pro `net-test` (`nts_ke`, `nts_ntp`).
    pub config: String,
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b).unwrap();
    b
}

fn wall_now() -> abi::time::WallTime {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    abi::time::WallTime {
        secs: d.as_secs() as i64,
        nanos: d.subsec_nanos(),
    }
}

fn key(conn: &rustls::ServerConnection, s2c: bool) -> Option<[u8; nts::KEY_LEN]> {
    conn.export_keying_material([0u8; nts::KEY_LEN], nts::EXPORTER_LABEL, Some(&nts::key_context(s2c)))
        .ok()
}

fn ke_conn(tcp: std::net::TcpStream, config: Arc<rustls::ServerConfig>, master: [u8; 32], ntp_port: u16) -> Option<()> {
    let conn = rustls::ServerConnection::new(config).ok()?;
    let mut s = rustls::StreamOwned::new(conn, tcp);
    let mut req = Vec::new();
    while !server::ke_request_complete(&req) {
        let mut buf = [0u8; 1024];
        let n = s.read(&mut buf).ok()?;
        if n == 0 || req.len() > 4096 {
            return None;
        }
        req.extend_from_slice(&buf[..n]);
    }
    if s.conn.alpn_protocol() != Some(nts::ALPN) {
        return None;
    }
    let (c2s, s2c) = (key(&s.conn, false)?, key(&s.conn, true)?);
    let cookies: Vec<Vec<u8>> = (0..nts::COOKIES)
        .map(|_| server::make_cookie(&master, &random(), &c2s, &s2c))
        .collect();
    s.write_all(&server::ke_reply(&cookies, Some(ntp_port))).ok()?;
    s.conn.send_close_notify();
    s.flush().ok()?;
    Some(())
}

fn ntp_loop(sock: UdpSocket, master: [u8; 32]) {
    let mut buf = [0u8; 2048];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf) else {
            continue;
        };
        let receive = wall_now();
        let data = &buf[..n];
        let reply = match server::verify_request(&master, data) {
            Some(req) => {
                let Ok(header) = ntp::build_reply(data, receive, wall_now()) else {
                    continue;
                };
                let cookies: Vec<Vec<u8>> = (0..=req.placeholders.min(7))
                    .map(|_| {
                        // Nove cookies se stejnymi klici (z overene cookie).
                        let (c2s, s2c) = (req_c2s(&master, data), req.s2c);
                        server::make_cookie(&master, &random(), &c2s, &s2c)
                    })
                    .collect();
                server::reply(&header, &req, &cookies, &random())
            }
            None if n >= ntp::LEN => server::nak(data, &uid(data)),
            None => continue,
        };
        let _ = sock.send_to(&reply, from);
    }
}

/// Klic C2S z cookie pozadavku (uz overeneho).
fn req_c2s(master: &[u8; 32], data: &[u8]) -> [u8; 32] {
    cookie(data).and_then(|c| server::open_cookie(master, c)).map(|(c, _)| c).unwrap_or([0; 32])
}

fn field(data: &[u8], kind: u16) -> Option<&[u8]> {
    let mut pos = ntp::LEN;
    while pos + 4 <= data.len() {
        let t = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if len < 4 || pos + len > data.len() {
            return None;
        }
        if t == kind {
            return Some(&data[pos + 4..pos + len]);
        }
        pos += len;
    }
    None
}

fn cookie(data: &[u8]) -> Option<&[u8]> {
    field(data, nts::ef::COOKIE)
}

fn uid(data: &[u8]) -> Vec<u8> {
    field(data, nts::ef::UNIQUE_ID).map(<[u8]>::to_vec).unwrap_or_default()
}

pub fn start(tls: Arc<rustls::ServerConfig>) -> std::io::Result<NtsHelper> {
    let master: [u8; 32] = random();
    let ke = crate::nethelper::tcp_listener("nts_ke")?;
    let udp = crate::nethelper::udp_socket("nts_ntp")?;
    let ntp_port = udp.local_addr()?.port();
    let config = format!("nts_ke = {}\nnts_ntp = {ntp_port}\n", ke.local_addr()?.port());
    thread::spawn(move || ntp_loop(udp, master));
    thread::spawn(move || {
        for tcp in ke.incoming().flatten() {
            let tls = tls.clone();
            thread::spawn(move || ke_conn(tcp, tls, master, ntp_port));
        }
    });
    Ok(NtsHelper { config })
}
