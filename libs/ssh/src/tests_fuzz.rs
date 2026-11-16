//! Fuzz bez libFuzzeru (bezi v kazdem `cargo test`): nahodne a zmutovane
//! vstupy do casti, ktere ctou data ze site pred prihlasenim, do SFTP
//! a do rozboru klicu. Zadny vstup nesmi zpusobit paniku. Vetsi beh
//! s pokrytim: `libs/ssh/fuzz` (cargo-fuzz, Linux).

use alloc::vec;
use alloc::vec::Vec;

use crate::keys::PublicKey;
use crate::server::{Auth, Backend, ServerConn, Start};
use crate::sftp::{Attrs, Sftp, SftpFs};

/// xorshift64* (deterministicky, opakovatelny beh).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// Mutace: prepsat, vlozit, smazat bajty, zkratit.
fn mutate(r: &mut Rng, src: &[u8]) -> Vec<u8> {
    let mut v = src.to_vec();
    for _ in 0..1 + r.below(8) {
        match r.below(4) {
            0 if !v.is_empty() => {
                let i = r.below(v.len());
                v[i] = r.next() as u8;
            }
            1 => {
                let i = r.below(v.len() + 1);
                v.insert(i, r.next() as u8);
            }
            2 if !v.is_empty() => {
                let i = r.below(v.len());
                v.remove(i);
            }
            _ => {
                let n = r.below(v.len() + 1);
                v.truncate(n);
            }
        }
    }
    v
}

struct Deny;

impl Backend for Deny {
    fn probe(&mut self, _: &str, _: &[u8]) -> bool {
        false
    }
    fn auth(&mut self, _: &str, _: &[u8], _: &[u8], _: &[u8]) -> Auth {
        Auth::Denied
    }
    fn start(&mut self, _: u32, _: Start) -> bool {
        false
    }
    fn winch(&mut self, _: u32, _: u32, _: u32) {}
}

/// Vstup serveru po kouscich; chyba = spojeni se zavre, panika = chyba.
fn feed_server(data: &[u8], chunk: usize) {
    let mut s = ServerConn::new(&[3; 32]).unwrap();
    let mut out = vec![0u8; 40_000];
    for c in data.chunks(chunk.max(1)) {
        let mut off = 0;
        let mut guard = 0;
        while off < c.len() && guard < 64 {
            guard += 1;
            match s.input(&c[off..]) {
                Ok(n) => off += n,
                Err(_) => return,
            }
            if s.progress(&mut Deny).is_err() {
                return;
            }
            while s.output(&mut out) > 0 {}
        }
    }
}

#[test]
fn fuzz_server_pred_prihlasenim() {
    let mut r = Rng(0x5EED_0001);
    let hello = b"SSH-2.0-OpenSSH_9.7\r\n";
    // Nahodne pakety za platnym uvodnim radkem i bez nej.
    for i in 0..2000 {
        let mut d = if i % 2 == 0 { hello.to_vec() } else { Vec::new() };
        let n = r.below(3000);
        d.extend(r.bytes(n));
        feed_server(&d, 1 + r.below(700));
    }
    // Zmutovany zacatek skutecneho handshake (uvod + KEXINIT klienta).
    let kexinit = crate::tests_server::client_hello();
    assert!(kexinit.starts_with(b"SSH-2.0-") && kexinit.len() > 200, "handshake klienta {} B", kexinit.len());
    for _ in 0..5000 {
        let d = mutate(&mut r, &kexinit);
        feed_server(&d, 1 + r.below(900));
    }
}

#[derive(Default)]
struct NullFs;

impl SftpFs for NullFs {
    fn stat(&mut self, _: &str) -> Result<Attrs, abi::Error> {
        Ok(Attrs::default())
    }
    fn list(&mut self, _: &str) -> Result<Vec<(alloc::string::String, Attrs)>, abi::Error> {
        Ok(vec![(alloc::string::String::from("a"), Attrs::default())])
    }
    fn read(&mut self, _: &str, _: u64, len: usize) -> Result<Vec<u8>, abi::Error> {
        Ok(vec![1; len.min(16)])
    }
    fn write(&mut self, _: &str, _: u64, _: &[u8]) -> Result<(), abi::Error> {
        Ok(())
    }
    fn create(&mut self, _: &str) -> Result<(), abi::Error> {
        Ok(())
    }
    fn truncate(&mut self, _: &str, _: u64) -> Result<(), abi::Error> {
        Ok(())
    }
    fn mkdir(&mut self, _: &str) -> Result<(), abi::Error> {
        Err(abi::Error::AccessDenied)
    }
    fn remove(&mut self, _: &str) -> Result<(), abi::Error> {
        Err(abi::Error::NotFound)
    }
    fn rename(&mut self, _: &str, _: &str) -> Result<(), abi::Error> {
        Ok(())
    }
}

#[test]
fn fuzz_sftp() {
    let mut r = Rng(0x5EED_0002);
    for _ in 0..3000 {
        let mut s = Sftp::new("/home/a");
        let mut fs = NullFs;
        // Spravny INIT, pak nahodne pakety se spravnou delkou i bez ni.
        let _ = s.input(&[0, 0, 0, 5, 1, 0, 0, 0, 3], &mut fs);
        for _ in 0..1 + r.below(6) {
            let body_len = r.below(200);
            let mut p = Vec::new();
            if r.below(4) > 0 {
                p.extend_from_slice(&((body_len + 1) as u32).to_be_bytes());
                p.push(1 + r.below(20) as u8);
            }
            p.extend(r.bytes(body_len));
            let _ = s.input(&p, &mut fs);
        }
    }
}

#[test]
fn fuzz_klice() {
    let mut r = Rng(0x5EED_0003);
    let good = crate::keys::ed25519_public(&[7; 32]).blob();
    for _ in 0..20_000 {
        let blob = if r.below(2) == 0 { mutate(&mut r, &good) } else { { let n = r.below(120); r.bytes(n) } };
        if let Ok(k) = PublicKey::from_blob(&blob) {
            let n = r.below(100);
            let sig = r.bytes(n);
            let _ = k.verify(b"data", &sig, r.below(2) == 0);
            let _ = PublicKey::parse_line(&k.line());
        }
        let n = r.below(80);
        let line: alloc::string::String = r.bytes(n).into_iter().map(|b| (b % 96 + 32) as char).collect();
        let _ = PublicKey::parse_line(&line);
    }
}
