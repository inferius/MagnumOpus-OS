//! Cteni z proudu s vyrovnavaci pameti (radky HTTP/1.1, ramce HTTP/2).

use alloc::vec;
use alloc::vec::Vec;

use netproto::io::Read;

use crate::{Error, Result};

const CHUNK: usize = 16 * 1024;

pub struct Input<S> {
    pub stream: S,
    buf: Vec<u8>,
    start: usize,
    end: usize,
    eof: bool,
}

impl<S: Read> Input<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: vec![0; CHUNK],
            start: 0,
            end: 0,
            eof: false,
        }
    }

    pub fn buffered(&self) -> &[u8] {
        &self.buf[self.start..self.end]
    }

    pub fn consume(&mut self, n: usize) {
        self.start += n;
        debug_assert!(self.start <= self.end);
    }

    /// Docte dalsi data; `false` = konec proudu.
    pub fn fill(&mut self) -> Result<bool> {
        if self.eof {
            return Ok(false);
        }
        if self.start > 0 {
            self.buf.copy_within(self.start..self.end, 0);
            self.end -= self.start;
            self.start = 0;
        }
        if self.end == self.buf.len() {
            let len = self.buf.len();
            self.buf.resize(len + CHUNK, 0);
        }
        let n = self.stream.read(&mut self.buf[self.end..]).map_err(Error::Io)?;
        if n == 0 {
            self.eof = true;
            return Ok(false);
        }
        self.end += n;
        Ok(true)
    }

    /// Zajisti aspon `n` bajtu v bufferu; predcasny konec = chyba protokolu.
    pub fn need(&mut self, n: usize) -> Result<()> {
        while self.end - self.start < n {
            if !self.fill()? {
                return Err(crate::protocol("spojeni skoncilo uprostred zpravy"));
            }
        }
        Ok(())
    }

    /// Precte radek zakonceny LF (CRLF; samotne LF se toleruje) bez
    /// zakonceni. `limit` = nejvetsi delka; `None` = konec proudu pred
    /// prvnim bajtem.
    pub fn line(&mut self, limit: usize) -> Result<Option<Vec<u8>>> {
        let mut scanned = 0;
        loop {
            let avail = self.buffered();
            if let Some(i) = avail[scanned..].iter().position(|&b| b == b'\n') {
                let end = scanned + i;
                if end > limit {
                    return Err(Error::TooLarge("radek hlavicky"));
                }
                let mut line = avail[..end].to_vec();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                self.consume(end + 1);
                return Ok(Some(line));
            }
            scanned = avail.len();
            if scanned > limit {
                return Err(Error::TooLarge("radek hlavicky"));
            }
            if !self.fill()? {
                if scanned == 0 {
                    return Ok(None);
                }
                return Err(crate::protocol("spojeni skoncilo uprostred radku"));
            }
        }
    }

    /// Presune az `max` bajtu do `out`; `0` = konec proudu.
    pub fn read_into(&mut self, out: &mut Vec<u8>, max: usize) -> Result<usize> {
        if self.start == self.end && !self.fill()? {
            return Ok(0);
        }
        let n = max.min(self.end - self.start);
        out.extend_from_slice(&self.buf[self.start..self.start + n]);
        self.consume(n);
        Ok(n)
    }
}
