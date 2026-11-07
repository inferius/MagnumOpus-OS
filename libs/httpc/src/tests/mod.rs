//! Testy na hostiteli.
//!
//! - `unit`: URL, HPACK (priklady z RFC 7541), HTTP/1.1 a HTTP/2 proti
//!   skriptovanemu proudu (chybne odpovedi serveru, limity).
//! - `interop`: proti nezavislemu serveru hyper (HTTP/1.1 i HTTP/2 h2c)
//!   pres TCP na 127.0.0.1.

mod interop;
mod unit;

use std::vec::Vec;

use netproto::io::{Read, Write};

/// Proud pro testy: cte pripravena data, zapsana si pamatuje.
pub struct Script {
    pub input: Vec<u8>,
    pub pos: usize,
    pub written: Vec<u8>,
    /// Kolik bajtu nejvys vratit jednim `read` (deleni na kusy).
    pub step: usize,
}

impl Script {
    pub fn new(input: &[u8]) -> Self {
        Self {
            input: input.to_vec(),
            pos: 0,
            written: Vec::new(),
            step: usize::MAX,
        }
    }
}

impl Read for Script {
    fn read(&mut self, buf: &mut [u8]) -> abi::Result<usize> {
        let n = buf.len().min(self.input.len() - self.pos).min(self.step);
        buf[..n].copy_from_slice(&self.input[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Write for Script {
    fn write(&mut self, buf: &[u8]) -> abi::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> abi::Result<()> {
        Ok(())
    }
}
