//! Testovaci servery HTTP a HTTPS na hostiteli (`cargo xtask test`) a sada
//! korenovych certifikatu pro initfs.
//!
//! Servery jsou nezavisle na kodu systemu: TLS rustls s kryptografii ring,
//! HTTP hyper (HTTP/1.1 i HTTP/2). Certifikaty vydava testovaci CA
//! vygenerovana pri kazdem behu (rcgen); jeji certifikat jde do initfs jako
//! `net-test-ca.pem` a init ho ulozi do systemoveho uloziste.
//!
//! Porty (v `net-test`):
//! - `http`: HTTP/1.1 bez TLS,
//! - `https`: TLS s ALPN `h2` a `http/1.1` (klient vybere HTTP/2),
//! - `https_h1`: TLS jen s `http/1.1`,
//! - `https_untrusted`: certifikat od jine CA (ktere host neveri),
//! - `https_wrongname`: certifikat na jine jmeno nez 10.0.2.2,
//! - `https_revoked`: certifikat odvolany v CRL testovaci CA (`/ca.crl`),
//! - `https_ocsp_revoked`: odpoved OCSP "revoked" v handshaku,
//! - `https_onecrl`: mezilehla CA na seznamu OneCRL (`/onecrl.json`).
//!
//! Certifikat serveru `https` a `https_h1` ma distribucni bod CRL
//! (`http://10.0.2.2:<http>/ca.crl`), cislo v ni neni.
//!
//! Cesty: `/hello` (text), `/ca.crl`, `/onecrl.json`, `/stream?n=N` (N bajtu vzoru [`pattern`] po
//! kusech bez delky), `/echo` (vrati telo, `x-method`), `/redirect?to=URL`.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;

use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::{Body, Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, SanType};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// Sada korenovych certifikatu Mozilla (crate `webpki-root-certs`, verze
/// podle Cargo.lock) jako PEM.
pub fn bundle_pem() -> Vec<u8> {
    let mut out = String::from(
        "# Korenove certifikaty Mozilla (CCADB) z crate webpki-root-certs.\n\
         # Spravuje system: init soubor pri startu prepise. Vlastni CA patri\n\
         # do dalsich souboru, neduveryhodne CA do distrust.txt.\n",
    );
    for cert in webpki_root_certs::TLS_SERVER_ROOT_CERTS {
        out.push_str(&pem(cert.as_ref()));
    }
    out.into_bytes()
}

fn pem(der: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = String::new();
    for chunk in der.chunks(3) {
        let v = chunk.iter().enumerate().fold(0u32, |v, (i, &b)| v | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                b64.push(ALPHABET[(v >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// Bajt `i` tela `/stream` (stejny vzor pocita test v init).
pub fn pattern(i: usize) -> u8 {
    (i.wrapping_mul(7) ^ (i >> 9)) as u8
}

struct Chunks {
    left: usize,
    sent: usize,
}

impl Body for Chunks {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        if self.left == 0 {
            return Poll::Ready(None);
        }
        let n = self.left.min(10_000);
        let data: Vec<u8> = (self.sent..self.sent + n).map(pattern).collect();
        self.left -= n;
        self.sent += n;
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(data)))))
    }
}

type Resp = hyper::Response<BoxBody<Bytes, Infallible>>;

fn full(status: u16, body: impl Into<Bytes>) -> Resp {
    hyper::Response::builder().status(status).body(Full::new(body.into()).boxed()).unwrap()
}

fn query<'a>(uri: &'a hyper::Uri, key: &str) -> Option<&'a str> {
    uri.query()?.split('&').find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

async fn handle(req: hyper::Request<Incoming>) -> Result<Resp, Infallible> {
    let uri = req.uri().clone();
    Ok(match uri.path() {
        "/hello" => full(200, "ahoj z hostitele"),
        "/ca.crl" => full(200, CRL.get().cloned().unwrap_or_default()),
        "/dns-query" => {
            let body = req.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
            match crate::dnshelper::answer(&body) {
                Some(a) => {
                    let mut r = full(200, a);
                    r.headers_mut().insert("content-type", "application/dns-message".parse().unwrap());
                    r
                }
                None => full(400, "neplatny dotaz DNS"),
            }
        }
        "/onecrl.json" => full(200, ONECRL.get().cloned().unwrap_or_default()),
        "/stream" => {
            let n = query(&uri, "n").and_then(|n| n.parse().ok()).unwrap_or(0);
            hyper::Response::new(Chunks { left: n, sent: 0 }.boxed())
        }
        "/echo" => {
            let method = req.method().to_string();
            let body = req.into_body().collect().await.map(|b| b.to_bytes()).unwrap_or_default();
            let mut r = full(200, body);
            r.headers_mut().insert("x-method", method.parse().unwrap());
            r
        }
        "/redirect" => {
            let mut r = full(302, "");
            let to = query(&uri, "to").unwrap_or("/hello");
            r.headers_mut().insert("location", to.parse().unwrap());
            r
        }
        _ => full(404, "nenalezeno"),
    })
}

/// CRL testovaci CA (`/ca.crl`) a dokument OneCRL (`/onecrl.json`).
static CRL: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
static ONECRL: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Cislo certifikatu serveru `https_revoked` (je v CRL).
const REVOKED_SERIAL: u64 = 0x0666;

struct Ca {
    cert: rcgen::Certificate,
    key: KeyPair,
}

fn ca(name: &str) -> Ca {
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name.push(DnType::CommonName, name);
    p.not_before = rcgen::date_time_ymd(2020, 1, 1);
    p.not_after = rcgen::date_time_ymd(2099, 1, 1);
    Ca { cert: p.self_signed(&key).unwrap(), key }
}

/// Mezilehla CA od `parent` s cislem `serial`.
fn intermediate(parent: &Ca, name: &str, serial: u64) -> Ca {
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.distinguished_name.push(DnType::CommonName, name);
    p.serial_number = Some(rcgen::SerialNumber::from(serial));
    p.not_before = rcgen::date_time_ymd(2020, 1, 1);
    p.not_after = rcgen::date_time_ymd(2099, 1, 1);
    Ca { cert: p.signed_by(&key, &parent.cert, &parent.key).unwrap(), key }
}

/// Certifikat serveru od `ca` pro jmena/adresy `sans`.
fn leaf(ca: &Ca, sans: &[SanType]) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    leaf_with(ca, sans, None, None)
}

/// Certifikat serveru s cislem a distribucnim bodem CRL.
fn leaf_with(
    ca: &Ca,
    sans: &[SanType],
    serial: Option<u64>,
    crl_url: Option<&str>,
) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
    let key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
    let mut p = CertificateParams::new(Vec::<String>::new()).unwrap();
    p.subject_alt_names = sans.to_vec();
    p.serial_number = serial.map(rcgen::SerialNumber::from);
    if let Some(u) = crl_url {
        p.crl_distribution_points = vec![rcgen::CrlDistributionPoint { uris: vec![u.to_string()] }];
    }
    p.distinguished_name.push(DnType::CommonName, "MagnumOpus test server");
    p.not_before = rcgen::date_time_ymd(2020, 1, 1);
    p.not_after = rcgen::date_time_ymd(2098, 1, 1);
    let cert = p.signed_by(&key, &ca.cert, &ca.key).unwrap();
    (
        vec![cert.der().clone()],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
    )
}

fn tls_config(chain: Vec<CertificateDer<'static>>, key: PrivateKeyDer<'static>, alpn: &[&[u8]]) -> Arc<rustls::ServerConfig> {
    tls_config_ocsp(chain, key, alpn, Vec::new())
}

/// Konfigurace TLS s odpovedi OCSP v handshaku (prazdna = bez staplingu).
fn tls_config_ocsp(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    alpn: &[&[u8]],
    ocsp: Vec<u8>,
) -> Arc<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut c = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert_with_ocsp(chain, key, ocsp)
        .unwrap();
    c.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Arc::new(c)
}

async fn serve<I>(io: I, h2: bool)
where
    I: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(io);
    let svc = service_fn(handle);
    let _ = if h2 {
        hyper::server::conn::http2::Builder::new(TokioExecutor::new()).serve_connection(io, svc).await
    } else {
        hyper::server::conn::http1::Builder::new().serve_connection(io, svc).await
    };
}

fn listen() -> std::io::Result<std::net::TcpListener> {
    let l = std::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
    l.set_nonblocking(true)?;
    Ok(l)
}

async fn run_server(listener: std::net::TcpListener, tls: Option<Arc<rustls::ServerConfig>>) {
    let listener = tokio::net::TcpListener::from_std(listener).unwrap();
    loop {
        let Ok((tcp, _)) = listener.accept().await else {
            continue;
        };
        let tls = tls.clone();
        tokio::spawn(async move {
            match tls {
                None => serve(tcp, false).await,
                Some(config) => {
                    let Ok(stream) = tokio_rustls::TlsAcceptor::from(config).accept(tcp).await else {
                        return;
                    };
                    let h2 = stream.get_ref().1.alpn_protocol() == Some(b"h2");
                    serve(stream, h2).await;
                }
            }
        });
    }
}

pub struct WebHelper {
    /// Radky pro `net-test`.
    pub config: String,
    /// Kotva DNSSEC testovaci hierarchie (initfs `net-test-dns-anchor`).
    pub dns_anchor: String,
    /// Certifikat testovaci CA (PEM) pro initfs.
    pub ca_pem: String,
}

pub fn start() -> std::io::Result<WebHelper> {
    let http = listen()?;
    let crl_url = format!("http://10.0.2.2:{}/ca.crl", http.local_addr()?.port());
    let ca_ok = ca("MagnumOpus test CA");
    let ca_other = ca("Cizi CA");
    let guest_host = SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)));
    // Bezne servery: certifikat s CRL (revocd ji stahne, cislo v ni neni).
    let (chain, key) = leaf_with(&ca_ok, &[guest_host.clone()], Some(0x100), Some(&crl_url));
    let both = tls_config(chain.clone(), key.clone_key(), &[b"h2", b"http/1.1"]);
    let h1_only = tls_config(chain, key, &[b"http/1.1"]);
    let (chain, key) = leaf(&ca_other, &[guest_host.clone()]);
    let untrusted = tls_config(chain, key, &[b"http/1.1"]);
    let (chain, key) = leaf(&ca_ok, &[SanType::DnsName("other.example".try_into().unwrap())]);
    let wrong_name = tls_config(chain, key, &[b"http/1.1"]);
    // Odvolani: v CRL, v OCSP (stapling), mezilehla CA v OneCRL.
    let (chain, key) = leaf_with(&ca_ok, &[guest_host.clone()], Some(REVOKED_SERIAL), Some(&crl_url));
    let revoked = tls_config(chain, key, &[b"http/1.1"]);
    let (chain, key) = leaf_with(&ca_ok, &[guest_host.clone()], Some(0x0777), None);
    let ocsp = crate::revhelper::ocsp_revoked(ca_ok.cert.der(), &ca_ok.key, &chain[0]);
    let ocsp_revoked = tls_config_ocsp(chain, key, &[b"http/1.1"], ocsp);
    let inter = intermediate(&ca_ok, "Odvolana mezilehla CA", 0x0888);
    let (mut chain, key) = leaf_with(&inter, &[guest_host], Some(0x0999), None);
    chain.push(inter.cert.der().clone());
    let onecrl_revoked = tls_config(chain, key, &[b"http/1.1"]);
    // NTS-KE (testy ntpd): certifikat testovaci CA pro 10.0.2.2.
    let (chain, key) = leaf(&ca_ok, &[SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)))]);
    let nts = crate::ntshelper::start(tls_config(chain, key, &[netproto::nts::ALPN]))?;
    // DoT (testy dnsd); DoH je cesta /dns-query na serveru `https`.
    let (chain, key) = leaf(&ca_ok, &[SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(10, 0, 2, 2)))]);
    let dot = listen()?;
    let dot_port = dot.local_addr()?.port();
    crate::dnshelper::start_dot(dot, tls_config(chain, key, &[]));
    let _ = CRL.set(crate::revhelper::crl(&ca_ok.cert, &ca_ok.key, &[REVOKED_SERIAL]));
    let _ = ONECRL.set(crate::revhelper::onecrl(inter.cert.der()));

    let servers = [
        ("http", http, None),
        ("https", listen()?, Some(both)),
        ("https_h1", listen()?, Some(h1_only)),
        ("https_untrusted", listen()?, Some(untrusted)),
        ("https_wrongname", listen()?, Some(wrong_name)),
        ("https_revoked", listen()?, Some(revoked)),
        ("https_ocsp_revoked", listen()?, Some(ocsp_revoked)),
        ("https_onecrl", listen()?, Some(onecrl_revoked)),
    ];
    let mut config = format!("{}dot = {dot_port}
", nts.config);
    for (name, l, _) in &servers {
        config.push_str(&format!("{name} = {}
", l.local_addr()?.port()));
    }
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_io().build().unwrap();
        rt.block_on(async move {
            let mut tasks = Vec::new();
            for (_, l, tls) in servers {
                tasks.push(tokio::spawn(run_server(l, tls)));
            }
            for t in tasks {
                let _ = t.await;
            }
        });
    });
    Ok(WebHelper { config, ca_pem: ca_ok.cert.pem(), dns_anchor: crate::dnshelper::anchor() })
}
