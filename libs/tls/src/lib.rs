//! Klient TLS 1.3 pro programy (bez `std`).
//!
//! - Protokol: rustls (overena implementace TLS v Rustu) v rezimu bez `std`
//!   (`UnbufferedClientConnection`).
//! - Kryptografie: vlastni poskytovatel nad knihovnami RustCrypto
//!   ([`provider`]), cisty Rust bez C a bez SIMD.
//! - Overeni serveru: retez certifikatu do korenove CA ze systemoveho
//!   uloziste ([`roots`]), jmeno serveru (SAN), platnost podle realneho casu.
//! - Odvolani: OCSP stapling, odvolane CA (OneCRL) a CRL ze zdroje
//!   [`revocation::RevocationSource`] (v systemu sluzba `revocd`).
//! - ALPN (vyjednani protokolu, napr. `h2` a `http/1.1`).
//!
//! TLS bezi v procesu aplikace nad libovolnym proudem
//! (`netproto::io::Read + Write`, typicky `TcpStream`): klice a data vidi jen
//! aplikace, sluzba site dal hlida relaci a firewall.
//!
//! ```ignore
//! static PLATFORM: tls::Platform = tls::LIBOS; // nahoda a cas z jadra
//! let roots = tls::roots::Roots::from_files(files.iter().map(|(n, d)| (n.as_str(), &d[..])));
//! let client = tls::Client::new(&PLATFORM, &roots, &[b"h2", b"http/1.1"])?;
//! let tcp = net.tcp_connect_host("example.com", 443, None)?;
//! let mut s = client.connect("example.com", tcp)?;
//! s.write_all(b"...")?;
//! ```

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod ocsp;
pub mod provider;
pub mod revocation;
pub mod roots;
mod stream;
pub mod x509;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub use provider::Platform;
pub use stream::TlsStream;

/// Chyba navazani nebo prubehu spojeni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Chyba proudu pod TLS (TCP).
    Io(abi::Error),
    /// Certifikat serveru neprosel overenim (popis pro uzivatele a log).
    Certificate(String),
    /// Chyba protokolu TLS (popis z rustls).
    Protocol(String),
    /// Spojeni skoncilo bez `close_notify` (mozne zkraceni utocnikem).
    Truncated,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "chyba spojeni: {e}"),
            Self::Certificate(m) => write!(f, "neplatny certifikat serveru: {m}"),
            Self::Protocol(m) => write!(f, "chyba TLS: {m}"),
            Self::Truncated => f.write_str("spojeni skoncilo bez close_notify"),
        }
    }
}

impl From<rustls::Error> for Error {
    fn from(e: rustls::Error) -> Self {
        match e {
            rustls::Error::InvalidCertificate(c) => Self::Certificate(alloc::format!("{c:?}")),
            rustls::Error::NoCertificatesPresented => Self::Certificate(String::from("server neposlal certifikat")),
            other => Self::Protocol(alloc::format!("{other}")),
        }
    }
}

impl From<abi::Error> for Error {
    fn from(e: abi::Error) -> Self {
        Self::Io(e)
    }
}

impl Error {
    /// Kod chyby pro rozhrani proudu (`netproto::io`): certifikat
    /// = `ACCESS_DENIED`, protokol = `CORRUPTED`, zkraceni = `CONNECTION_RESET`.
    pub fn code(&self) -> abi::Error {
        match self {
            Self::Io(e) => *e,
            Self::Certificate(_) => abi::Error::AccessDenied,
            Self::Protocol(_) => abi::Error::Corrupted,
            Self::Truncated => abi::Error::ConnectionReset,
        }
    }
}

/// Nastaveni klienta: poskytovatel kryptografie, korenove certifikaty
/// a nabizene protokoly ALPN. Jedno pro vsechna spojeni programu.
#[derive(Clone)]
pub struct Client {
    config: Arc<rustls::ClientConfig>,
}

struct Time(&'static Platform);

impl core::fmt::Debug for Time {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Time")
    }
}

impl rustls::time_provider::TimeProvider for Time {
    fn current_time(&self) -> Option<pki_types::UnixTime> {
        rustls::time_provider::TimeProvider::current_time(self.0)
    }
}

impl Client {
    /// `alpn` = nabizene protokoly v poradi preference (prazdne = bez ALPN).
    /// Bez zdroje odvolani se kontroluje jen OCSP stapling.
    pub fn new(platform: &'static Platform, roots: &roots::Roots, alpn: &[&[u8]]) -> Result<Self, Error> {
        Self::with_revocation(platform, roots, alpn, None)
    }

    /// Jako [`Client::new`], s kontrolou odvolani ze zdroje `revocation`
    /// (v systemu [`revocation::ChannelSource`] ke sluzbe `revocd`).
    pub fn with_revocation(
        platform: &'static Platform,
        roots: &roots::Roots,
        alpn: &[&[u8]],
        revocation: Option<Arc<dyn revocation::RevocationSource>>,
    ) -> Result<Self, Error> {
        let provider = Arc::new(provider::provider(platform));
        let verifier = revocation::Verifier::new(Arc::new(roots.store()), provider.clone(), revocation)?;
        let mut config = rustls::ClientConfig::builder_with_details(provider, Arc::new(Time(platform)))
            .with_protocol_versions(&[&rustls::version::TLS13])?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect::<Vec<_>>();
        Ok(Self {
            config: Arc::new(config),
        })
    }

    /// Navaze TLS nad proudem `sock` se serverem `name` (jmeno nebo adresa
    /// IP, overi se proti certifikatu).
    pub fn connect<S: netproto::io::Read + netproto::io::Write>(&self, name: &str, sock: S) -> Result<TlsStream<S>, Error> {
        let server = pki_types::ServerName::try_from(String::from(name))
            .map_err(|_| Error::Certificate(alloc::format!("neplatne jmeno serveru {name}")))?;
        let conn = rustls::client::UnbufferedClientConnection::new(self.config.clone(), server)?;
        TlsStream::handshake(conn, sock)
    }
}

/// Platforma z libos: nahoda z generatoru jadra (jen nasazeneho) a realny
/// cas jadra.
#[cfg(feature = "libos")]
pub static LIBOS: Platform = Platform {
    random: |buf| libos::random::fill_seeded(buf).is_ok(),
    now: || libos::time::wall().and_then(|t| u64::try_from(t.secs).ok()),
};
