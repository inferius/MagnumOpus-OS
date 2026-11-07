//! Testovaci resolvery DoH a DoT na hostiteli (`cargo xtask test`) nad
//! podepsanou testovaci hierarchii `dns::testzone` (koren, `test.`,
//! `example.test.` s NSEC3, nepodepsane `unsigned.test.` a `magnumopus.`,
//! podvrzene `bad.test.`).
//!
//! Odpovidaji jako rekurzivni resolver s DO a CD (RRSIG a dukazy
//! neexistence). Kotva (DS korene) jde do initfs `net-test-dns-anchor`.
//! TLS: rustls + ring s certifikatem testovaci CA pro 10.0.2.2.

use std::sync::{Arc, OnceLock};
use std::thread;

use dns::Message;
use dns::testzone::Hierarchy;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static ZONES: OnceLock<Hierarchy> = OnceLock::new();

/// Odpoved na zpravu DNS (bajty); `None` = neplatny dotaz.
pub fn answer(query: &[u8]) -> Option<Vec<u8>> {
    let q = Message::parse(query).ok()?;
    Some(ZONES.get_or_init(Hierarchy::standard).answer(&q).to_wire())
}

/// Kotva pro dnsd: `"tag alg typ otisk"`.
pub fn anchor() -> String {
    let ds = ZONES.get_or_init(Hierarchy::standard).anchor();
    let d = &ds.rdata;
    let hex: String = d[4..].iter().map(|b| format!("{b:02X}")).collect();
    format!("{} {} {} {hex}", u16::from_be_bytes([d[0], d[1]]), d[2], d[3])
}

async fn dot_conn(stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>) {
    let mut s = stream;
    loop {
        let mut len = [0u8; 2];
        if s.read_exact(&mut len).await.is_err() {
            return;
        }
        let mut msg = vec![0u8; u16::from_be_bytes(len) as usize];
        if s.read_exact(&mut msg).await.is_err() {
            return;
        }
        let Some(resp) = answer(&msg) else {
            return;
        };
        let mut out = (resp.len() as u16).to_be_bytes().to_vec();
        out.extend_from_slice(&resp);
        if s.write_all(&out).await.is_err() {
            return;
        }
    }
}

/// Server DoT (trvala spojeni, zpravy s delkou u16).
pub fn start_dot(listener: std::net::TcpListener, tls: Arc<rustls::ServerConfig>) {
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_io().build().unwrap();
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    continue;
                };
                let acceptor = tokio_rustls::TlsAcceptor::from(tls.clone());
                tokio::spawn(async move {
                    if let Ok(s) = acceptor.accept(tcp).await {
                        dot_conn(s).await;
                    }
                });
            }
        });
    });
}
