//! Klient SSH (`client.rs`) proti nasemu serveru v pameti: prikaz
//! s vystupem a kodem, podpis mimo klienta (jako sluzba uctu), odmitnuty
//! klic serveru, neuspesne prihlaseni, stdin s EOF.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::client::{CState, ClientBackend, ClientConn, Failure, Request};
use crate::keys::{PublicKey, ed25519_public};
use crate::server::{Auth, Backend, ServerConn, Start};

const HOST: [u8; 32] = [9; 32];
const ALICE: [u8; 32] = [1; 32];

struct Serv {
    alice: PublicKey,
    started: Vec<(u32, Start)>,
}

impl Backend for Serv {
    fn probe(&mut self, user: &str, key: &[u8]) -> bool {
        user == "alice" && key == self.alice.blob().as_slice()
    }
    fn auth(&mut self, user: &str, key: &[u8], data: &[u8], sig: &[u8]) -> Auth {
        if user == "alice" && key == self.alice.blob().as_slice() && self.alice.verify(data, sig, false).is_ok() {
            Auth::Ok
        } else {
            Auth::Denied
        }
    }
    fn start(&mut self, chan: u32, start: Start) -> bool {
        self.started.push((chan, start));
        true
    }
    fn winch(&mut self, _: u32, _: u32, _: u32) {}
}

struct Cli {
    /// Ocekavany klic serveru (`None` = prijmout cokoli).
    known: Option<PublicKey>,
    seed: [u8; 32],
    signs: u32,
    passwords: u32,
}

impl ClientBackend for Cli {
    fn hostkey(&mut self, key: &PublicKey) -> bool {
        self.known.as_ref().is_none_or(|k| k.blob() == key.blob())
    }
    fn identities(&mut self) -> Vec<PublicKey> {
        vec![ed25519_public(&self.seed)]
    }
    fn sign(&mut self, key: &PublicKey, data: &[u8]) -> Option<[u8; 64]> {
        use ed25519_dalek::Signer;
        self.signs += 1;
        let k = ed25519_dalek::SigningKey::from_bytes(&self.seed);
        assert_eq!(key.key, k.verifying_key().to_bytes());
        Some(k.sign(data).to_bytes())
    }
    fn password(&mut self) -> Option<String> {
        self.passwords += 1;
        Some(String::from("heslo"))
    }
}

fn serve(s: &mut ServerConn, b: &mut Serv, got: &mut Vec<u8>) {
    let mut d = [0u8; 512];
    loop {
        s.progress(b).unwrap();
        let mut any = false;
        while let Some((_, n)) = s.read(&mut d).unwrap() {
            got.extend_from_slice(&d[..n]);
            any = true;
        }
        if !any {
            break;
        }
    }
}

/// Bajty mezi klientem a serverem; vraci data, ktera klient precetl.
fn pump(c: &mut ClientConn, cb: &mut Cli, s: &mut ServerConn, sb: &mut Serv, server_got: &mut Vec<u8>, rounds: usize) -> Vec<u8> {
    let mut buf = vec![0u8; 40_000];
    let mut out = Vec::new();
    let mut rd = [0u8; 512];
    for _ in 0..rounds {
        c.progress(cb).unwrap();
        while let Some((_, n)) = c.read(&mut rd).unwrap() {
            out.extend_from_slice(&rd[..n]);
            c.progress(cb).unwrap();
        }
        let n = c.output(&mut buf);
        let mut off = 0;
        let mut stuck = 0;
        while off < n {
            let k = s.input(&buf[off..n]).unwrap();
            stuck = if k == 0 { stuck + 1 } else { 0 };
            assert!(stuck < 100, "server neprijima vstup");
            off += k;
            serve(s, sb, server_got);
        }
        serve(s, sb, server_got);
        let n = s.output(&mut buf);
        let mut off = 0;
        let mut stuck = 0;
        while off < n && matches!(c.state(), CState::Connecting | CState::Running) {
            match c.input(&buf[off..n]) {
                Ok(0) => {
                    stuck += 1;
                    assert!(stuck < 100, "klient neprijima vstup: {:?}, data kanalu {}", c.state(), c.has_channel_data());
                }
                Ok(k) => off += k,
                Err(_) => return out,
            }
            c.progress(cb).unwrap();
            while let Some((_, n)) = c.read(&mut rd).unwrap() {
                out.extend_from_slice(&rd[..n]);
                c.progress(cb).unwrap();
            }
        }
        if !matches!(c.state(), CState::Connecting | CState::Running) {
            break;
        }
    }
    out
}

fn setup(seed: [u8; 32], known: Option<PublicKey>) -> (ClientConn, Cli, ServerConn, Serv) {
    let c = ClientConn::new("alice", Request::Exec(String::from("echo ahoj")));
    let cb = Cli { known, seed, signs: 0, passwords: 0 };
    let s = ServerConn::new(&HOST).unwrap();
    let sb = Serv { alice: ed25519_public(&ALICE), started: Vec::new() };
    (c, cb, s, sb)
}

#[test]
fn exec_vystup_a_kod() {
    let (mut c, mut cb, mut s, mut sb) = setup(ALICE, Some(ed25519_public(&HOST)));
    let mut got = Vec::new();
    pump(&mut c, &mut cb, &mut s, &mut sb, &mut got, 30);
    assert_eq!(cb.signs, 1, "podpis mimo klienta");
    assert_eq!(cb.passwords, 0);
    let (chan, start) = sb.started.first().cloned().expect("server spustil prikaz");
    assert_eq!(start, Start::Exec { command: String::from("echo ahoj"), pty: None });
    assert_eq!(*c.state(), CState::Running);
    // stdin prikazu a EOF.
    assert_eq!(c.write(b"vstup").unwrap(), 5);
    c.send_eof().unwrap();
    pump(&mut c, &mut cb, &mut s, &mut sb, &mut got, 5);
    assert_eq!(got, b"vstup");
    assert!(s.eof(chan), "server vidi EOF");
    // Vystup, kod a zavreni.
    s.write(chan, b"ahoj\n", false).unwrap();
    s.write(chan, b"chyba\n", true).unwrap();
    s.finish(chan, 3).unwrap();
    let out = pump(&mut c, &mut cb, &mut s, &mut sb, &mut got, 20);
    assert_eq!(out, b"ahoj\nchyba\n");
    assert_eq!(*c.state(), CState::Closed(Some(3)));
}

#[test]
fn cizi_klic_serveru_odmitnut() {
    let (mut c, mut cb, mut s, mut sb) = setup(ALICE, Some(ed25519_public(&[7; 32])));
    let mut got = Vec::new();
    pump(&mut c, &mut cb, &mut s, &mut sb, &mut got, 30);
    assert_eq!(*c.state(), CState::Failed(Failure::Hostkey));
    assert_eq!(cb.signs, 0, "bez overeneho serveru se nic nepodepisuje");
    assert!(sb.started.is_empty());
}

#[test]
fn spatny_klic_a_heslo_neprojdou() {
    let (mut c, mut cb, mut s, mut sb) = setup([2; 32], None);
    let mut got = Vec::new();
    pump(&mut c, &mut cb, &mut s, &mut sb, &mut got, 40);
    // Nas server heslo nenabizi: klient ho nezada.
    assert_eq!(cb.passwords, 0);
    assert_eq!(*c.state(), CState::Failed(Failure::Auth));
    assert!(sb.started.is_empty());
}
