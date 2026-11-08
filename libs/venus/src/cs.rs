//! Proud prikazu Venusu (command stream): zakladni kodovani, ktere
//! pouziva generovany kod (`vk`).
//!
//! Pravidla (venus-protocol, `vn_protocol_driver_types.h`):
//! - vse little-endian, kazda polozka zarovnana na 4 bajty,
//! - skalar mensi nez 4 bajty (`uint8_t`, `uint16_t`) zabere 4 bajty,
//!   pole techto skalaru je ale husta a doplnena na nasobek 4,
//! - `size_t` = 8 bajtu, handle = id objektu `u64`,
//! - velikost pole = `u64`; ukazatel na jednu hodnotu = velikost pole 0/1,
//! - retezec = velikost `strlen + 1`, bajty s NUL, doplneno na 4.
//!
//! Enkoder chyby (pole kratsi nez jeho delka, neplatny clen retezce pNext)
//! jen zaznamena a `finish` je vrati: generovany kod tak nemusi vsude
//! propagovat `Result`. Dekoder je bezpecny na libovolna data: cteni za
//! konec nebo nesmyslna velikost pole = chyba, nikdy panika.

use alloc::vec::Vec;

/// Chyba kodovani nebo cteni proudu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CsError {
    /// Pole kratsi, nez rika jeho delka (clen s `len`).
    ShortArray,
    /// Struktura v retezci pNext, kterou rodic nepovoluje.
    BadChain,
    /// Retezec obsahuje NUL.
    BadString,
    /// Cteni za koncem dat.
    Truncated,
    /// Neocekavana hodnota (typ prikazu odpovedi, sType, velikost pole).
    Unexpected,
}

pub type Result<T> = core::result::Result<T, CsError>;

#[derive(Debug, Default)]
pub struct Encoder {
    buf: Vec<u8>,
    err: Option<CsError>,
}

impl Encoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Zaznamena chybu (prvni vyhrava).
    pub fn fail(&mut self, e: CsError) {
        self.err.get_or_insert(e);
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Hotovy proud, nebo prvni chyba.
    pub fn finish(self) -> Result<Vec<u8>> {
        match self.err {
            Some(e) => Err(e),
            None => Ok(self.buf),
        }
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }

    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i64(&mut self, v: i64) {
        self.u64(v as u64);
    }

    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    pub fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    /// Skalar mensi nez 4 bajty zabere 4 bajty.
    pub fn u16(&mut self, v: u16) {
        self.u32(v as u32);
    }

    pub fn u8(&mut self, v: u8) {
        self.u32(v as u32);
    }

    /// Velikost pole.
    pub fn array_size(&mut self, n: usize) {
        self.u64(n as u64);
    }

    /// Ukazatel na jednu hodnotu: 1 = nasleduje hodnota, 0 = NULL.
    pub fn pointer(&mut self, present: bool) {
        self.u64(present as u64);
    }

    /// Surove bajty doplnene nulami na nasobek 4.
    pub fn blob(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
        let pad = b.len().next_multiple_of(4) - b.len();
        self.buf.extend_from_slice(&[0u8; 3][..pad]);
    }

    /// Retezec: velikost `len + 1`, bajty s NUL, doplneni.
    pub fn string(&mut self, s: &str) {
        if s.as_bytes().contains(&0) {
            self.fail(CsError::BadString);
        }
        self.array_size(s.len() + 1);
        self.buf.extend_from_slice(s.as_bytes());
        let n = s.len() + 1;
        self.buf.push(0);
        let pad = n.next_multiple_of(4) - n;
        self.buf.extend_from_slice(&[0u8; 3][..pad]);
    }

    /// Volitelny retezec (`None` = velikost pole 0).
    pub fn opt_string(&mut self, s: Option<&str>) {
        match s {
            Some(s) => self.string(s),
            None => self.array_size(0),
        }
    }

    pub fn u8_array(&mut self, v: &[u8]) {
        self.blob(v);
    }

    pub fn u16_array(&mut self, v: &[u16]) {
        for x in v {
            self.buf.extend_from_slice(&x.to_le_bytes());
        }
        if v.len() % 2 == 1 {
            self.buf.extend_from_slice(&[0, 0]);
        }
    }

    pub fn u32_array(&mut self, v: &[u32]) {
        v.iter().for_each(|&x| self.u32(x));
    }

    pub fn i32_array(&mut self, v: &[i32]) {
        v.iter().for_each(|&x| self.i32(x));
    }

    pub fn u64_array(&mut self, v: &[u64]) {
        v.iter().for_each(|&x| self.u64(x));
    }

    pub fn i64_array(&mut self, v: &[i64]) {
        v.iter().for_each(|&x| self.i64(x));
    }

    pub fn f32_array(&mut self, v: &[f32]) {
        v.iter().for_each(|&x| self.f32(x));
    }

    pub fn f64_array(&mut self, v: &[f64]) {
        v.iter().for_each(|&x| self.f64(x));
    }

    /// Prvnich `n` prvku pole s delkou `n` (clen s `len`): prazdne pole =
    /// NULL (velikost 0), kratsi pole nez `n` = chyba. Vraci prvky ke
    /// kodovani (prazdne, kdyz je pole NULL).
    pub fn dyn_array<'a, T>(&mut self, v: &'a [T], n: usize) -> &'a [T] {
        if v.is_empty() {
            self.array_size(0);
            return &[];
        }
        if v.len() < n {
            self.fail(CsError::ShortArray);
            self.array_size(0);
            return &[];
        }
        self.array_size(n);
        &v[..n]
    }
}

/// Cteni proudu (odpovedi rendereru).
pub struct Decoder<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, pos: 0 }
    }

    /// Prectene bajty.
    pub fn position(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.b.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(CsError::Truncated)?;
        let s = self.b.get(self.pos..end).ok_or(CsError::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }

    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    pub fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }

    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }

    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }

    pub fn u16(&mut self) -> Result<u16> {
        Ok(self.u32()? as u16)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.u32()? as u8)
    }

    /// Velikost pole; vic prvku, nez se do zbytku dat vejde (prvek ma
    /// aspon `min_elem` bajtu), je chyba (ochrana pred obri alokaci).
    pub fn array_size(&mut self, min_elem: usize) -> Result<usize> {
        let n = self.u64()?;
        let n = usize::try_from(n).map_err(|_| CsError::Unexpected)?;
        if min_elem > 0 && n.saturating_mul(min_elem) > self.remaining() {
            return Err(CsError::Unexpected);
        }
        Ok(n)
    }

    /// Velikost pevneho pole: musi byt presne `n`.
    pub fn fixed_array(&mut self, n: usize) -> Result<()> {
        if self.u64()? != n as u64 {
            return Err(CsError::Unexpected);
        }
        Ok(())
    }

    /// Ukazatel na jednu hodnotu (nenulova velikost = hodnota nasleduje).
    pub fn pointer(&mut self) -> Result<bool> {
        Ok(self.u64()? != 0)
    }

    /// Retezec pNext odpovedi: tento klient retezce ve vystupu nepouziva,
    /// takze musi byt NULL.
    pub fn no_chain(&mut self) -> Result<()> {
        if self.pointer()? { Err(CsError::Unexpected) } else { Ok(()) }
    }

    /// `n` bajtu doplnenych na nasobek 4.
    pub fn blob(&mut self, out: &mut [u8]) -> Result<()> {
        let n = out.len();
        let s = self.take(n.next_multiple_of(4))?;
        out.copy_from_slice(&s[..n]);
        Ok(())
    }

    pub fn u8_array(&mut self, out: &mut [u8]) -> Result<()> {
        self.blob(out)
    }

    /// Pevne pole `char`: posledni bajt se vynuluje (jako `vn_decode_char_array`).
    pub fn char_array(&mut self, out: &mut [u8]) -> Result<()> {
        self.blob(out)?;
        if let Some(l) = out.last_mut() {
            *l = 0;
        }
        Ok(())
    }

    pub fn u16_array(&mut self, out: &mut [u16]) -> Result<()> {
        let s = self.take((out.len() * 2).next_multiple_of(4))?;
        for (i, o) in out.iter_mut().enumerate() {
            *o = u16::from_le_bytes([s[2 * i], s[2 * i + 1]]);
        }
        Ok(())
    }

    pub fn u32_array(&mut self, out: &mut [u32]) -> Result<()> {
        out.iter_mut().try_for_each(|o| self.u32().map(|v| *o = v))
    }

    pub fn i32_array(&mut self, out: &mut [i32]) -> Result<()> {
        out.iter_mut().try_for_each(|o| self.i32().map(|v| *o = v))
    }

    pub fn u64_array(&mut self, out: &mut [u64]) -> Result<()> {
        out.iter_mut().try_for_each(|o| self.u64().map(|v| *o = v))
    }

    pub fn i64_array(&mut self, out: &mut [i64]) -> Result<()> {
        out.iter_mut().try_for_each(|o| self.i64().map(|v| *o = v))
    }

    pub fn f32_array(&mut self, out: &mut [f32]) -> Result<()> {
        out.iter_mut().try_for_each(|o| self.f32().map(|v| *o = v))
    }

    pub fn f64_array(&mut self, out: &mut [f64]) -> Result<()> {
        out.iter_mut().try_for_each(|o| self.f64().map(|v| *o = v))
    }
}

/// Jmeno pevneho pole `char` (do prvniho NUL, neplatne UTF-8 = `?`).
pub fn c_str(b: &[u8]) -> &str {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    core::str::from_utf8(&b[..n]).unwrap_or("?")
}
