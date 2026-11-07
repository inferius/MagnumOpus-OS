//! Testy na hostiteli: nas klient (RustCrypto) proti nezavislemu serveru
//! (rustls + ring) pres TCP na 127.0.0.1, testovaci CA z rcgen.
//!
//! Pokryva vsechny sady sifer, skupiny vymeny klicu a typy klicu serveru
//! (ECDSA P-256/P-384, Ed25519, RSA), ALPN, chyby certifikatu (cizi CA,
//! spatne jmeno, propadly), zkraceni bez `close_notify` a uloziste korenu.
//! Kontrolu odvolani testuje [`revocation`], CRL "jen CA" (i skutecne CRL
//! Let's Encrypt) [`crl`].

mod crl;
mod revocation;

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::string::{String, ToString};
use std::sync::Arc;
use std::thread;
use std::vec;
use std::vec::Vec;

use netproto::io::{Read, Write};
use pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SignatureAlgorithm};

use crate::roots::{self, Roots};
use crate::{Client, Error, Platform};

fn ring_random(buf: &mut [u8]) -> bool {
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), buf).is_ok()
}

fn now_secs() -> Option<u64> {
    Some(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs())
}

static PLATFORM: Platform = Platform {
    random: ring_random,
    now: now_secs,
};

/// Rok 2100: certifikat platny 1 rok uz propadl.
static FUTURE: Platform = Platform {
    random: ring_random,
    now: || Some(4_102_444_800),
};

/// Bez nahody se spojeni nesmi navazat.
static NO_RANDOM: Platform = Platform {
    random: |_| false,
    now: now_secs,
};

/// Proud `std` pro `netproto::io`.
struct Sock(TcpStream);

impl Read for Sock {
    fn read(&mut self, buf: &mut [u8]) -> abi::Result<usize> {
        self.0.read(buf).map_err(|_| abi::Error::ConnectionReset)
    }
}

impl Write for Sock {
    fn write(&mut self, buf: &[u8]) -> abi::Result<usize> {
        self.0.write(buf).map_err(|_| abi::Error::ConnectionReset)
    }

    fn flush(&mut self) -> abi::Result<()> {
        self.0.flush().map_err(|_| abi::Error::ConnectionReset)
    }
}

const RSA_KEY: &[u8] = include_bytes!("../../testdata/rsa2048.pk8.der");

#[derive(Clone, Copy)]
enum KeyKind {
    P256,
    P384,
    Ed25519,
    Rsa,
}

fn key(kind: KeyKind) -> (KeyPair, &'static SignatureAlgorithm) {
    match kind {
        KeyKind::P256 => (KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap(), &rcgen::PKCS_ECDSA_P256_SHA256),
        KeyKind::P384 => (KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).unwrap(), &rcgen::PKCS_ECDSA_P384_SHA384),
        KeyKind::Ed25519 => (KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap(), &rcgen::PKCS_ED25519),
        KeyKind::Rsa => {
            let der = PrivatePkcs8KeyDer::from(RSA_KEY.to_vec());
            (KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_RSA_SHA256).unwrap(), &rcgen::PKCS_RSA_SHA256)
        }
    }
}

/// Testovaci PKI: CA a certifikat serveru podepsany CA.
struct Pki {
    ca_pem: String,
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
}

static CA_SERIAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn pki(kind: KeyKind, names: &[&str], expired_in_2100: bool) -> Pki {
    let (ca_key, _) = key(kind);
    // Kazda CA jine jmeno: cizi CA se pak pozna uz podle vystavitele.
    let serial = CA_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.distinguished_name.push(DnType::CommonName, std::format!("MagnumOpus test CA {serial}"));
    ca.not_before = rcgen::date_time_ymd(2020, 1, 1);
    ca.not_after = rcgen::date_time_ymd(2200, 1, 1);
    let ca = ca.self_signed(&ca_key).unwrap();
    let (leaf_key, _) = key(kind);
    let mut leaf = CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>()).unwrap();
    leaf.distinguished_name.push(DnType::CommonName, names[0]);
    leaf.not_before = rcgen::date_time_ymd(2020, 1, 1);
    leaf.not_after = if expired_in_2100 { rcgen::date_time_ymd(2099, 1, 1) } else { rcgen::date_time_ymd(2199, 1, 1) };
    let leaf = leaf.signed_by(&leaf_key, &ca, &ca_key).unwrap();
    Pki {
        ca_pem: ca.pem(),
        chain: vec![leaf.der().clone()],
        key: PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    }
}

#[derive(Clone, Copy)]
enum Mode {
    /// Prijme `[delka u32][data]`, odpovi SHA-256 dat, pak `big` bajtu
    /// vzoru a `close_notify`.
    Exchange { big: usize },
    /// Posle "ahoj" a zavre TCP bez `close_notify`.
    Truncate,
    /// Jen handshake, pak ceka na `close_notify` klienta.
    Handshake,
}

struct ServerOpts {
    suite: Option<rustls::CipherSuite>,
    group: Option<rustls::NamedGroup>,
    alpn: Vec<Vec<u8>>,
    mode: Mode,
    /// Odpoved OCSP poslana v handshaku (prazdna = bez staplingu).
    ocsp: Vec<u8>,
}

impl Default for ServerOpts {
    fn default() -> Self {
        Self {
            suite: None,
            group: None,
            alpn: Vec::new(),
            mode: Mode::Exchange { big: 0 },
            ocsp: Vec::new(),
        }
    }
}

fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(31) ^ (i >> 8)) as u8
}

/// Spusti server na nahodnem portu; vrati port a vlakno (vysledek serveru).
fn server(pki: &Pki, opts: ServerOpts) -> (u16, thread::JoinHandle<Result<(), String>>) {
    let mut provider = rustls::crypto::ring::default_provider();
    if let Some(s) = opts.suite {
        provider.cipher_suites.retain(|c| c.suite() == s);
    }
    if let Some(g) = opts.group {
        provider.kx_groups.retain(|k| k.name() == g);
    }
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert_with_ocsp(pki.chain.clone(), pki.key.clone_key(), opts.ocsp)
        .unwrap();
    config.alpn_protocols = opts.alpn;
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mode = opts.mode;
    let handle = thread::spawn(move || -> Result<(), String> {
        let (tcp, _) = listener.accept().map_err(|e| e.to_string())?;
        let conn = rustls::ServerConnection::new(config).map_err(|e| e.to_string())?;
        let mut s = rustls::StreamOwned::new(conn, tcp);
        let e = |e: std::io::Error| e.to_string();
        // Dokoncit handshake (precist Finished klienta): neprectena data
        // pri zavreni soketu = RST misto FIN.
        while s.conn.is_handshaking() {
            s.conn.complete_io(&mut s.sock).map_err(e)?;
        }
        match mode {
            Mode::Exchange { big } => {
                let mut len = [0u8; 4];
                s.read_exact(&mut len).map_err(e)?;
                let mut data = vec![0u8; u32::from_le_bytes(len) as usize];
                s.read_exact(&mut data).map_err(e)?;
                // Primo pres ServerConnection (StreamOwned::write se pokousi
                // i cist a s blokujicim soketem by cekal na klienta).
                let mut out = ring::digest::digest(&ring::digest::SHA256, &data).as_ref().to_vec();
                out.extend((0..big).map(pattern));
                s.conn.set_buffer_limit(None);
                s.conn.writer().write_all(&out).map_err(e)?;
                s.conn.send_close_notify();
                while s.conn.wants_write() {
                    s.conn.write_tls(&mut s.sock).map_err(e)?;
                }
            }
            Mode::Truncate => {
                s.conn.writer().write_all(b"ahoj").map_err(e)?;
                while s.conn.wants_write() {
                    s.conn.write_tls(&mut s.sock).map_err(e)?;
                }
                s.sock.shutdown(std::net::Shutdown::Write).map_err(e)?;
            }
            Mode::Handshake => {
                // Cte do konce (close_notify klienta) nebo chyby.
                let mut buf = [0u8; 64];
                loop {
                    match s.read(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(err) => return Err(err.to_string()),
                    }
                }
            }
        }
        Ok(())
    });
    (port, handle)
}

fn client(platform: &'static Platform, ca_pem: &str, alpn: &[&[u8]]) -> Client {
    let roots = Roots::from_files([("ca.pem", ca_pem.as_bytes())]);
    assert_eq!(roots.len(), 1);
    Client::new(platform, &roots, alpn).unwrap()
}

fn connect(port: u16) -> Sock {
    Sock(TcpStream::connect(("127.0.0.1", port)).unwrap())
}

/// Posle data, overi otisk a `big` bajtu odpovedi az do `close_notify`.
fn exchange(c: &Client, name: &str, port: u16, payload: &[u8], big: usize) {
    let mut s = c.connect(name, connect(port)).unwrap();
    s.write_all(&(payload.len() as u32).to_le_bytes()).unwrap();
    s.write_all(payload).unwrap();
    let mut digest = [0u8; 32];
    s.read_exact(&mut digest).unwrap();
    assert_eq!(&digest[..], ring::digest::digest(&ring::digest::SHA256, payload).as_ref());
    let mut body = Vec::new();
    s.read_to_end(&mut body).unwrap();
    assert_eq!(body.len(), big);
    assert!(body.iter().enumerate().all(|(i, &b)| b == pattern(i)));
}

#[test]
fn all_suites_and_groups() {
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let suites = [
        rustls::CipherSuite::TLS13_AES_128_GCM_SHA256,
        rustls::CipherSuite::TLS13_AES_256_GCM_SHA384,
        rustls::CipherSuite::TLS13_CHACHA20_POLY1305_SHA256,
    ];
    let groups = [rustls::NamedGroup::X25519, rustls::NamedGroup::secp256r1, rustls::NamedGroup::secp384r1];
    for suite in suites {
        for group in groups {
            let (port, h) = server(
                &pki,
                ServerOpts {
                    suite: Some(suite),
                    group: Some(group),
                    mode: Mode::Exchange { big: 1000 },
                    ..Default::default()
                },
            );
            let c = client(&PLATFORM, &pki.ca_pem, &[]);
            exchange(&c, "localhost", port, b"zprava pro server", 1000);
            h.join().unwrap().unwrap_or_else(|e| panic!("{suite:?} {group:?}: {e}"));
        }
    }
}

#[test]
fn server_key_types() {
    for kind in [KeyKind::P256, KeyKind::P384, KeyKind::Ed25519, KeyKind::Rsa] {
        let pki = pki(kind, &["test.example"], false);
        let c = client(&PLATFORM, &pki.ca_pem, &[]);
        let (port, h) = server(&pki, ServerOpts { mode: Mode::Exchange { big: 10 }, ..Default::default() });
        exchange(&c, "test.example", port, b"x", 10);
        h.join().unwrap().unwrap();
    }
}

#[test]
fn large_transfer() {
    // Vic zaznamu v obou smerech (zaznam = nejvys 16 KiB).
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let c = client(&PLATFORM, &pki.ca_pem, &[]);
    let (port, h) = server(&pki, ServerOpts { mode: Mode::Exchange { big: 1 << 20 }, ..Default::default() });
    let payload: Vec<u8> = (0..300_000).map(|i| pattern(i * 7)).collect();
    exchange(&c, "localhost", port, &payload, 1 << 20);
    h.join().unwrap().unwrap();
}

#[test]
fn alpn() {
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let c = client(&PLATFORM, &pki.ca_pem, &[b"h2", b"http/1.1"]);
    let (port, h) = server(
        &pki,
        ServerOpts {
            alpn: vec![b"http/1.1".to_vec()],
            mode: Mode::Handshake,
            ..Default::default()
        },
    );
    let mut s = c.connect("localhost", connect(port)).unwrap();
    assert_eq!(s.alpn_protocol(), Some(&b"http/1.1"[..]));
    s.close().unwrap();
    h.join().unwrap().unwrap();

    // Server bez ALPN: protokol neni vyjednany.
    let (port, h) = server(&pki, ServerOpts { mode: Mode::Handshake, ..Default::default() });
    let mut s = c.connect("localhost", connect(port)).unwrap();
    assert_eq!(s.alpn_protocol(), None);
    s.close().unwrap();
    h.join().unwrap().unwrap();
}

fn expect_cert_error(platform: &'static Platform, pki_server: &Pki, ca_pem: &str, name: &str) -> String {
    let c = client(platform, ca_pem, &[]);
    let (port, h) = server(pki_server, ServerOpts { mode: Mode::Handshake, ..Default::default() });
    let err = c.connect(name, connect(port)).err().expect("spojeni se nemelo navazat");
    // Server dostane alert od klienta.
    assert!(h.join().unwrap().is_err());
    match err {
        Error::Certificate(m) => m,
        other => panic!("cekana chyba certifikatu, prislo {other:?}"),
    }
}

#[test]
fn rejects_untrusted_ca() {
    let server_pki = pki(KeyKind::P256, &["localhost"], false);
    let other = pki(KeyKind::P256, &["localhost"], false);
    let m = expect_cert_error(&PLATFORM, &server_pki, &other.ca_pem, "localhost");
    assert!(m.contains("UnknownIssuer"), "{m}");
}

#[test]
fn rejects_wrong_name() {
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let m = expect_cert_error(&PLATFORM, &pki, &pki.ca_pem.clone(), "example.com");
    assert!(m.contains("NotValidForName"), "{m}");
}

#[test]
fn rejects_expired() {
    let pki = pki(KeyKind::P256, &["localhost"], true);
    let m = expect_cert_error(&FUTURE, &pki, &pki.ca_pem.clone(), "localhost");
    assert!(m.contains("Expired"), "{m}");
}

#[test]
fn ip_address_name() {
    let pki = pki(KeyKind::P256, &["127.0.0.1"], false);
    let c = client(&PLATFORM, &pki.ca_pem, &[]);
    let (port, h) = server(&pki, ServerOpts { mode: Mode::Exchange { big: 1 }, ..Default::default() });
    exchange(&c, "127.0.0.1", port, b"ip", 1);
    h.join().unwrap().unwrap();
}

#[test]
fn truncation_detected() {
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let c = client(&PLATFORM, &pki.ca_pem, &[]);
    let (port, h) = server(&pki, ServerOpts { mode: Mode::Truncate, ..Default::default() });
    let mut s = c.connect("localhost", connect(port)).unwrap();
    let mut buf = [0u8; 4];
    s.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"ahoj");
    // Konec TCP bez close_notify je chyba, ne konec proudu.
    assert_eq!(s.read_tls(&mut buf), Err(Error::Truncated));
    assert_eq!(Read::read(&mut s, &mut buf), Err(abi::Error::ConnectionReset));
    h.join().unwrap().unwrap();
}

#[test]
fn no_randomness_no_connection() {
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let roots = Roots::from_files([("ca.pem", pki.ca_pem.as_bytes())]);
    let c = Client::new(&NO_RANDOM, &roots, &[]).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    assert!(c.connect("localhost", connect(port)).is_err());
}

#[test]
fn roots_parsing() {
    let a = pki(KeyKind::P256, &["a"], false);
    let b = pki(KeyKind::Ed25519, &["b"], false);
    let bundle = std::format!("# komentar\n{}\nnesmysl\n{}", a.ca_pem, b.ca_pem);
    let r = Roots::from_files([("bundle.pem", bundle.as_bytes())]);
    assert_eq!((r.len(), r.rejected, r.distrusted), (2, 0, 0));

    // DER, neznamy soubor se preskoci, poskozeny PEM se pocita.
    let der = roots::parse_pem(&a.ca_pem).0.remove(0);
    let broken = "-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----\n";
    let r = Roots::from_files([
        ("a.der", &der[..]),
        ("readme.txt", b"ignoruj"),
        ("broken.pem", broken.as_bytes()),
        ("garbage.der", b"\x30\x03\x01\x01\x00"),
    ]);
    assert_eq!((r.len(), r.rejected), (1, 2));

    // distrust.txt vyradi CA podle otisku SHA-256 (hex, dvojtecky ok).
    let fp = roots::fingerprint(&der);
    let hex: Vec<String> = fp.iter().map(|b| std::format!("{b:02X}")).collect();
    let distrust = std::format!("# zneuzita CA\n{}\n", hex.join(":"));
    let r = Roots::from_files([("bundle.pem", bundle.as_bytes()), ("distrust.txt", distrust.as_bytes())]);
    assert_eq!((r.len(), r.distrusted), (1, 1));
}

#[test]
fn base64() {
    assert_eq!(roots::base64_decode(b"aGVsbG8=").unwrap(), b"hello");
    assert_eq!(roots::base64_decode(b"aGVs\nbG8h").unwrap(), b"hello!");
    assert_eq!(roots::base64_decode(b"aA==").unwrap(), b"h");
    assert!(roots::base64_decode(b"aGVsbG8").is_none());
    assert!(roots::base64_decode(b"a===").is_none());
    assert!(roots::base64_decode(b"a*==").is_none());
}

#[test]
fn resumption() {
    // Jeden server (jedna konfigurace a cache relaci) pro dve spojeni:
    // druhe se obnovi pres PSK (overuje HKDF/HMAC poskytovatele).
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(pki.chain.clone(), pki.key.clone_key())
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = thread::spawn(move || {
        for _ in 0..2 {
            let (tcp, _) = listener.accept().unwrap();
            let conn = rustls::ServerConnection::new(config.clone()).unwrap();
            let mut s = rustls::StreamOwned::new(conn, tcp);
            let mut buf = [0u8; 16];
            // Cte do close_notify klienta (cestou odejdou i listky relace).
            while s.read(&mut buf).unwrap() > 0 {}
            s.conn.send_close_notify();
            s.flush().unwrap();
        }
    });
    let c = client(&PLATFORM, &pki.ca_pem, &[]);
    for expect_resumed in [false, true] {
        let mut s = c.connect("localhost", connect(port)).unwrap();
        s.write_all(b"ping").unwrap();
        assert_eq!(s.resumed(), expect_resumed);
        s.close().unwrap();
        // Po vlastnim close_notify jde cist dal (polouzavreni) az do
        // close_notify serveru; cestou se zpracuji listky relace.
        let mut rest = Vec::new();
        s.read_to_end(&mut rest).unwrap();
        assert!(rest.is_empty());
    }
    h.join().unwrap();
}

#[test]
fn key_update() {
    // Server obnovi klice a pozada o totez klienta (KeyUpdate
    // update_requested): klient musi odpovedet vlastnim KeyUpdate, jinak
    // server jeho dalsi data nedesifruje.
    let pki = pki(KeyKind::P256, &["localhost"], false);
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(pki.chain.clone(), pki.key.clone_key())
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let h = thread::spawn(move || -> Result<(), String> {
        let (tcp, _) = listener.accept().unwrap();
        let mut s = rustls::StreamOwned::new(rustls::ServerConnection::new(config).unwrap(), tcp);
        let e = |e: std::io::Error| e.to_string();
        while s.conn.is_handshaking() {
            s.conn.complete_io(&mut s.sock).map_err(e)?;
        }
        for round in 0..3u8 {
            s.conn.refresh_traffic_keys().map_err(|e| e.to_string())?;
            s.conn.writer().write_all(&[b'a' + round]).map_err(e)?;
            while s.conn.wants_write() {
                s.conn.write_tls(&mut s.sock).map_err(e)?;
            }
            let mut b = [0u8; 1];
            s.read_exact(&mut b).map_err(e)?;
            if b[0] != b'A' + round {
                return Err(std::format!("kolo {round}: prislo {b:?}"));
            }
        }
        s.conn.send_close_notify();
        while s.conn.wants_write() {
            s.conn.write_tls(&mut s.sock).map_err(e)?;
        }
        Ok(())
    });
    let c = client(&PLATFORM, &pki.ca_pem, &[]);
    let mut s = c.connect("localhost", connect(port)).unwrap();
    for round in 0..3u8 {
        let mut b = [0u8; 1];
        s.read_exact(&mut b).unwrap();
        assert_eq!(b[0], b'a' + round);
        s.write_all(&[b'A' + round]).unwrap();
    }
    let mut rest = Vec::new();
    s.read_to_end(&mut rest).unwrap();
    h.join().unwrap().unwrap();
}

#[test]
fn keying_material_export() {
    // Export klicu (NTS) musi dat stejne bajty jako rustls+ring na serveru.
    let pki = pki(KeyKind::P256, &["localhost"], false);
    for suite in [rustls::CipherSuite::TLS13_AES_128_GCM_SHA256, rustls::CipherSuite::TLS13_AES_256_GCM_SHA384] {
        let mut provider = rustls::crypto::ring::default_provider();
        provider.cipher_suites.retain(|c| c.suite() == suite);
        let config = Arc::new(
            rustls::ServerConfig::builder_with_provider(Arc::new(provider))
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(pki.chain.clone(), pki.key.clone_key())
                .unwrap(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let h = thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            let mut s = rustls::StreamOwned::new(rustls::ServerConnection::new(config).unwrap(), tcp);
            while s.conn.is_handshaking() {
                s.conn.complete_io(&mut s.sock).unwrap();
            }
            let key = s
                .conn
                .export_keying_material([0u8; 32], b"EXPORTER-network-time-security", Some(&[0, 0, 0, 15, 0]))
                .unwrap();
            let mut buf = [0u8; 1];
            let _ = s.read(&mut buf);
            key
        });
        let c = client(&PLATFORM, &pki.ca_pem, &[]);
        let mut s = c.connect("localhost", connect(port)).unwrap();
        let mut ours = [0u8; 32];
        s.export_keying_material(b"EXPORTER-network-time-security", &[0, 0, 0, 15, 0], &mut ours).unwrap();
        s.close().unwrap();
        assert_eq!(ours, h.join().unwrap(), "{suite:?}");
    }
}
