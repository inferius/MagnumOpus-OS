//! Spolecne kodovani zprav protokolu po kanalech (`fsproto`, `netproto`).
//!
//! Cisla little-endian, retezec = delka u16 + UTF-8, surova data = zbytek
//! zpravy. Cteni mimo zpravu = `INVALID_ARGS` (nikdy panika), takze
//! sluzba muze bezpecne cist zpravy od kohokoli. Typy specificke pro
//! jeden protokol (napr. adresy IP v `netproto`) se pridavaji rozsirujicimi
//! traity v danem protokolu.

#![no_std]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

use abi::{Error, Result};

#[derive(Default)]
pub struct Writer {
    b: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zacatek pozadavku: kod operace u32.
    pub fn op(op: u32) -> Self {
        let mut w = Self::new();
        w.u32(op);
        w
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.b.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.b.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.b.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i32(&mut self, v: i32) -> &mut Self {
        self.b.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.b.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Retezec (nejvys 65535 bajtu, delsi se zkrati na hranici znaku).
    pub fn str(&mut self, s: &str) -> &mut Self {
        let mut n = s.len().min(u16::MAX as usize);
        while !s.is_char_boundary(n) {
            n -= 1;
        }
        self.u16(n as u16);
        self.b.extend_from_slice(&s.as_bytes()[..n]);
        self
    }

    /// Surova data (bez delky; typicky na konci zpravy).
    pub fn bytes(&mut self, d: &[u8]) -> &mut Self {
        self.b.extend_from_slice(d);
        self
    }

    /// Dosavadni delka zpravy v bajtech.
    pub fn len(&self) -> usize {
        self.b.len()
    }

    pub fn is_empty(&self) -> bool {
        self.b.is_empty()
    }

    pub fn finish(self) -> Vec<u8> {
        self.b
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }

    /// Dalsich `n` bajtu; za koncem zpravy `INVALID_ARGS`.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::InvalidArgs)?;
        let s = self.b.get(self.pos..end).ok_or(Error::InvalidArgs)?;
        self.pos = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn str(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(core::str::from_utf8(self.take(n)?)
            .map_err(|_| Error::InvalidArgs)?
            .into())
    }

    /// Zbytek zpravy.
    pub fn rest(&mut self) -> &'a [u8] {
        let r = &self.b[self.pos..];
        self.pos = self.b.len();
        r
    }

    /// Zbyva jeste neco?
    pub fn is_empty(&self) -> bool {
        self.pos >= self.b.len()
    }
}
