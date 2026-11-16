//! Telo odpovedi po castech: [`Sink`] dostane hlavicky a pak data, jak
//! prichazeji ze spojeni (stahovani do souboru bez celeho tela v pameti).
//! [`VecSink`] je puvodni chovani (cele telo v pameti do
//! [`crate::Limits::max_body`]).

use alloc::vec::Vec;

use crate::{Error, Header, Result};

/// Prijemce tela odpovedi.
pub trait Sink {
    /// Stav a hlavicky odpovedi (pred telem); `len` = `Content-Length`,
    /// je-li znamy. Chyba = pozadavek skonci (napr. malo mista na disku).
    fn head(&mut self, status: u16, headers: &[Header], len: Option<u64>) -> Result<()>;
    /// Dalsi cast tela.
    fn data(&mut self, chunk: &[u8]) -> Result<()>;
}

/// Cele telo v pameti (do `max` bajtu).
pub struct VecSink {
    pub body: Vec<u8>,
    max: usize,
}

impl VecSink {
    pub fn new(max: usize) -> Self {
        Self { body: Vec::new(), max }
    }
}

impl Sink for VecSink {
    fn head(&mut self, _status: u16, _headers: &[Header], len: Option<u64>) -> Result<()> {
        self.body.clear();
        if len.is_some_and(|n| n > self.max as u64) {
            return Err(Error::TooLarge("telo odpovedi"));
        }
        Ok(())
    }

    fn data(&mut self, chunk: &[u8]) -> Result<()> {
        if self.body.len().saturating_add(chunk.len()) > self.max {
            return Err(Error::TooLarge("telo odpovedi"));
        }
        self.body.extend_from_slice(chunk);
        Ok(())
    }
}

/// Pri presmerovani (3xx s `Location`) telo zahodi, jinak preda dal.
pub(crate) struct Redirecting<'a> {
    pub inner: &'a mut dyn Sink,
    pub pass: bool,
}

impl Sink for Redirecting<'_> {
    fn head(&mut self, status: u16, headers: &[Header], len: Option<u64>) -> Result<()> {
        let redirect = matches!(status, 301 | 302 | 303 | 307 | 308) && headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("location"));
        self.pass = !redirect;
        if self.pass { self.inner.head(status, headers, len) } else { Ok(()) }
    }

    fn data(&mut self, chunk: &[u8]) -> Result<()> {
        if self.pass { self.inner.data(chunk) } else { Ok(()) }
    }
}
