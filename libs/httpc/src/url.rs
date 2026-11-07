//! Adresy `http://` a `https://` (RFC 3986, jen to, co klient potrebuje).
//!
//! - Jmeno serveru malymi pismeny, IPv6 v hranatych zavorkach; jmena
//!   s ne-ASCII znaky (IDN) zatim ne.
//! - Prihlasovaci udaje v adrese (`user:heslo@`) se odmitnou: v adrese by
//!   skoncily v historii a logech.
//! - Cesta: ne-ASCII bajty a mezera se zakoduji (`%XX`), ridici znaky se
//!   odmitnou, fragment (`#...`) se zahodi (na server se neposila).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub scheme: Scheme,
    /// Jmeno nebo adresa (IPv6 bez zavorek).
    pub host: String,
    pub port: u16,
    /// Cesta a dotaz (`/a/b?c`), vzdy zacina `/`.
    pub path: String,
}

fn bad(m: &'static str) -> Error {
    Error::Url(m)
}

/// Zakoduje ne-ASCII bajty a mezeru, odmitne ridici znaky.
fn encode_path(s: &str) -> Result<String> {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            0..=0x1F | 0x7F => return Err(bad("ridici znak v ceste")),
            b' ' | 0x80..=0xFF => out.push_str(&format!("%{b:02X}")),
            _ => out.push(b as char),
        }
    }
    Ok(out)
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_')
}

/// RFC 3986 5.2.4: odstrani segmenty `.` a `..`.
fn remove_dot_segments(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let segments: Vec<&str> = path.split('/').collect();
    let last = segments.len() - 1;
    for (i, seg) in segments.iter().enumerate().skip(1) {
        match *seg {
            "." => {
                if i == last {
                    out.push("");
                }
            }
            ".." => {
                out.pop();
                if i == last {
                    out.push("");
                }
            }
            s => out.push(s),
        }
    }
    let mut s = String::from("/");
    s.push_str(&out.join("/"));
    s
}

/// Odkaz zacina schematem (`pismeno *(pismeno / cislice / + - .) ":"`).
fn has_scheme(s: &str) -> bool {
    let Some(colon) = s.find(':') else {
        return false;
    };
    let scheme = &s[..colon];
    scheme.bytes().next().is_some_and(|b| b.is_ascii_alphabetic())
        && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
}

impl Url {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let (scheme, rest) = s.split_once("://").ok_or(bad("chybi schema"))?;
        let scheme = match scheme.to_ascii_lowercase().as_str() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(bad("podporovane je jen http a https")),
        };
        let rest = rest.split('#').next().unwrap_or("");
        let split = rest.find(['/', '?']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(split);
        if authority.contains('@') {
            return Err(bad("prihlasovaci udaje v adrese nejsou podporovane"));
        }
        let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
            let (h, after) = v6.split_once(']').ok_or(bad("neuzavrena adresa IPv6"))?;
            h.parse::<core::net::Ipv6Addr>().map_err(|_| bad("neplatna adresa IPv6"))?;
            let port = match after {
                "" => None,
                p => Some(p.strip_prefix(':').ok_or(bad("neplatny port"))?),
            };
            (h.to_ascii_lowercase(), port)
        } else {
            match authority.rsplit_once(':') {
                Some((h, p)) => (h.to_ascii_lowercase(), Some(p)),
                None => (authority.to_ascii_lowercase(), None),
            }
        };
        if !host.contains(':') && !valid_host(&host) {
            return Err(bad("neplatne jmeno serveru"));
        }
        let port = match port {
            None | Some("") => scheme.default_port(),
            Some(p) => match p.parse::<u16>() {
                Ok(p) if p != 0 => p,
                _ => return Err(bad("neplatny port")),
            },
        };
        let path = if path.is_empty() {
            String::from("/")
        } else if path.starts_with('?') {
            encode_path(&format!("/{path}"))?
        } else {
            encode_path(path)?
        };
        Ok(Self { scheme, host, port, path })
    }

    /// Jmeno a port pro `Host` / `:authority` (port jen nestandardni).
    pub fn authority(&self) -> String {
        let host = if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() };
        if self.port == self.scheme.default_port() { host } else { format!("{host}:{}", self.port) }
    }

    /// Stejny puvod (schema, jmeno, port).
    pub fn same_origin(&self, other: &Url) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }

    /// Adresa z hlavicky `Location` vuci teto adrese (absolutni, `//server`,
    /// `/cesta`, relativni cesta, `?dotaz`).
    pub fn join(&self, location: &str) -> Result<Url> {
        let loc = location.trim();
        if has_scheme(loc) {
            return Url::parse(loc);
        }
        if let Some(rest) = loc.strip_prefix("//") {
            return Url::parse(&format!("{}://{rest}", self.scheme.as_str()));
        }
        let loc = loc.split('#').next().unwrap_or("");
        let (base_path, base_query) = self.path.split_once('?').unwrap_or((&self.path, ""));
        let path = if loc.is_empty() {
            if base_query.is_empty() { String::from(base_path) } else { format!("{base_path}?{base_query}") }
        } else if loc.starts_with('?') {
            format!("{base_path}{loc}")
        } else {
            let (p, q) = match loc.split_once('?') {
                Some((p, q)) => (p, Some(q)),
                None => (loc, None),
            };
            let merged = if p.starts_with('/') {
                String::from(p)
            } else {
                let dir = &base_path[..base_path.rfind('/').map_or(0, |i| i + 1)];
                format!("{dir}{p}")
            };
            let mut path = remove_dot_segments(&merged);
            if let Some(q) = q {
                path.push('?');
                path.push_str(q);
            }
            path
        };
        Ok(Url {
            scheme: self.scheme,
            host: self.host.clone(),
            port: self.port,
            path: encode_path(&path)?,
        })
    }
}

impl core::fmt::Display for Url {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}://{}{}", self.scheme.as_str(), self.authority(), self.path)
    }
}
