//! Formatovani ve stylu `printf` (jadro `vsnprintf` a `vprintf`).
//!
//! Podporovane: priznaky `-+ #0`, sirka a presnost (i `*`), delky `hh h l
//! ll z j t`, prevody `d i u o x X c s p %`, siroke `%lc %ls` (prevod pres
//! kodovani locale, [`crate::mb`]). Stejny stroj formatuje do bajtu
//! (`printf`, [`format`]) i do sirokych znaku (`wprintf`, [`wformat`]);
//! sirka a presnost se pocitaji v jednotkach vystupu. Znak, ktery nejde
//! prevest, ukonci formatovani chybou [`Eilseq`]. Cela cisla se ctou podle delky
//! (bez delky jako `int`, C je tak predava). Zamerne:
//!
//! - `%n` nic nezapise (klasicka bezpecnostni dira printf), jen spotrebuje
//!   argument,
//! - neznamy prevod se vypise doslova (`%q` -> `%q`) a nic nespotrebuje,
//! - plovouci carka `%f %e %g %a` (modul `float`); `long double` (`%Lf`)
//!   neumime precist z `VaList` - vypise se `?` (viz docs/cs/cstd.md).
//!
//! Zdroj argumentu je rozhrani [`Args`]: v systemu `VaList` z C, v testech
//! seznam hodnot - samotne formatovani je tak testovatelne na hostiteli.

use alloc::vec::Vec;

use crate::mb::{Enc, MbState, Step};

/// Delka celeho cisla v argumentu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Len {
    /// `hh`: `char` (predany jako `int`).
    Char,
    /// `h`: `short` (predany jako `int`).
    Short,
    /// Bez delky: `int`.
    Int,
    /// `l`, `ll`, `z`, `j`, `t`: 64 bitu na nasich cilech.
    Long,
}

/// Odkud formatovani bere argumenty.
pub trait Args {
    /// Dalsi argument jako cele cislo se znamenkem dane delky.
    fn signed(&mut self, len: Len) -> i64;
    /// Dalsi argument jako cele cislo bez znamenka dane delky.
    fn unsigned(&mut self, len: Len) -> u64;
    /// Dalsi argument jako ukazatel (`%p`, `%n`).
    fn pointer(&mut self) -> usize;
    /// Dalsi argument jako retezec bajtu: preda bajty `out` po jednom, dokud
    /// `out` vraci `true` a retezec nekonci nulou. `false` = nulovy ukazatel.
    fn string(&mut self, out: &mut dyn FnMut(u8) -> bool) -> bool;
    /// Dalsi argument jako siroky retezec (`wchar_t *`), jinak jako `string`.
    fn wstring(&mut self, out: &mut dyn FnMut(u32) -> bool) -> bool;
    /// Dalsi argument typu `double` (i `float`, C ho povysi na `double`).
    fn double(&mut self) -> f64;
}

/// Kam formatovani pise (`U` = bajt nebo siroky znak).
pub trait Out<U = u8> {
    fn put(&mut self, b: U);
}

/// Znak, ktery nejde prevest mezi vicebajtovym a sirokym zapisem (EILSEQ).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eilseq;

/// Jednotka vystupu: bajt (`printf`) nebo siroky znak (`wprintf`).
pub trait Unit: Copy + 'static {
    const WIDE: bool;
    fn from_code(c: u32) -> Self;
    fn code(self) -> u32;
    fn from_ascii(b: u8) -> Self {
        Self::from_code(b as u32)
    }
}

impl Unit for u8 {
    const WIDE: bool = false;
    fn from_code(c: u32) -> Self {
        c as u8
    }
    fn code(self) -> u32 {
        self as u32
    }
}

impl Unit for u32 {
    const WIDE: bool = true;
    fn from_code(c: u32) -> Self {
        c
    }
    fn code(self) -> u32 {
        self
    }
}

/// Zapis do pole pevne delky s pocitanim celkove delky (`snprintf` vraci
/// delku, kterou by vystup mel bez oriznuti).
pub struct Buf<'a, U = u8> {
    pub buf: &'a mut [U],
    pub pos: usize,
    pub total: usize,
}

impl<U: Copy> Out<U> for Buf<'_, U> {
    fn put(&mut self, b: U) {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        }
        self.total += 1;
    }
}

/// Bajtovy vystup (`float`) do vystupu libovolne jednotky (cislice jsou ASCII).
struct Widen<'a, U: Unit>(&'a mut dyn Out<U>);

impl<U: Unit> Out<u8> for Widen<'_, U> {
    fn put(&mut self, b: u8) {
        self.0.put(U::from_ascii(b));
    }
}

#[derive(Default, Clone, Copy)]
pub(crate) struct Spec {
    pub(crate) left: bool,
    pub(crate) plus: bool,
    pub(crate) space: bool,
    pub(crate) alt: bool,
    pub(crate) zero: bool,
    pub(crate) width: usize,
    pub(crate) prec: Option<usize>,
}

/// Rozumny strop sirky a presnosti: `%2000000000d` nesmi zablokovat program
/// zapisem gigabajtu mezer (vystup by stejne oriznul buffer).
pub(crate) const MAX_WIDTH: usize = 4096;

pub(crate) fn pad<U: Unit>(out: &mut dyn Out<U>, n: usize, c: u8) {
    for _ in 0..n {
        out.put(U::from_ascii(c));
    }
}

fn number<U: Unit>(out: &mut dyn Out<U>, s: Spec, neg: bool, mag: u64, base: u64, upper: bool, prefix_alt: bool) {
    let digits_set: &[u8; 16] = if upper { b"0123456789ABCDEF" } else { b"0123456789abcdef" };
    let mut tmp = [0u8; 24];
    let mut n = 0;
    let mut v = mag;
    while v != 0 {
        tmp[n] = digits_set[(v % base) as usize];
        v /= base;
        n += 1;
    }
    // Presnost = minimalni pocet cislic; `%.0d` s nulou nevypise nic.
    let min_digits = s.prec.unwrap_or(1).min(MAX_WIDTH);
    let mut zeros = min_digits.saturating_sub(n);
    let mut prefix: &[u8] = b"";
    if neg {
        prefix = b"-";
    } else if s.plus {
        prefix = b"+";
    } else if s.space {
        prefix = b" ";
    }
    if prefix_alt && s.alt {
        match base {
            16 if mag != 0 => prefix = if upper { b"0X" } else { b"0x" },
            8 if zeros == 0 && (n == 0 || tmp[n - 1] != b'0') => zeros = 1,
            _ => {}
        }
    }
    let body = prefix.len() + zeros + n;
    let fill = s.width.saturating_sub(body);
    if !s.left && !(s.zero && s.prec.is_none()) {
        pad(out, fill, b' ');
    }
    for &b in prefix {
        out.put(U::from_ascii(b));
    }
    if !s.left && s.zero && s.prec.is_none() {
        pad(out, fill, b'0');
    }
    pad(out, zeros, b'0');
    for i in (0..n).rev() {
        out.put(U::from_ascii(tmp[i]));
    }
    if s.left {
        pad(out, fill, b' ');
    }
}

/// Siroky znak do vystupu `U` (u bajtu zakodovany), nejvys `max`
/// jednotek celkem: `Ok(false)` = uz se nevejde (konec textu).
fn emit_wide<U: Unit>(wc: u32, enc: Enc, max: usize, n: &mut usize, sink: &mut dyn FnMut(U)) -> Result<bool, Eilseq> {
    if U::WIDE {
        if *n >= max {
            return Ok(false);
        }
        sink(U::from_code(wc));
        *n += 1;
        return Ok(true);
    }
    let mut b = [0u8; 4];
    let k = enc.encode(wc, &mut b).ok_or(Eilseq)?;
    // Nikdy pul vicebajtoveho znaku.
    if *n + k > max {
        return Ok(false);
    }
    for &x in &b[..k] {
        sink(U::from_code(x as u32));
    }
    *n += k;
    Ok(true)
}

/// Text prevodu `%c %lc %s %ls` v jednotkach vystupu `U`, nejvys `max`
/// jednotek. `Ok(false)` = nulovy ukazatel.
fn text<U: Unit>(args: &mut dyn Args, wide_arg: bool, single: bool, max: usize, enc: Enc, sink: &mut dyn FnMut(U)) -> Result<bool, Eilseq> {
    let mut n = 0usize;
    let mut err: Option<Eilseq> = None;
    if single {
        // `%c` (int -> unsigned char) nebo `%lc` (wint_t).
        let v = args.unsigned(Len::Int) as u32;
        if wide_arg {
            emit_wide(v, enc, usize::MAX, &mut n, sink)?;
        } else if U::WIDE {
            // Bajt -> siroky znak (`btowc`).
            let (wc, _) = enc.decode(&[v as u8]).ok_or(Eilseq)?;
            sink(U::from_code(wc));
        } else {
            sink(U::from_code(v & 0xFF));
        }
        return Ok(true);
    }
    let present = if wide_arg {
        args.wstring(&mut |wc| match emit_wide(wc, enc, max, &mut n, sink) {
            Ok(more) => more,
            Err(e) => {
                err = Some(e);
                false
            }
        })
    } else if U::WIDE {
        // Vicebajtovy retezec -> siroke znaky (`mbrtowc` po bajtech).
        let mut st = MbState::INITIAL;
        let present = args.string(&mut |b| {
            if n >= max {
                return false;
            }
            match enc.feed(&mut st, b) {
                Step::Char(c) => {
                    sink(U::from_code(c));
                    n += 1;
                    true
                }
                Step::More => true,
                Step::Bad => {
                    err = Some(Eilseq);
                    false
                }
            }
        });
        // Retezec skoncil uprostred znaku.
        if present && err.is_none() && !st.is_initial() && n < max {
            err = Some(Eilseq);
        }
        present
    } else {
        args.string(&mut |b| {
            if n >= max {
                return false;
            }
            sink(U::from_code(b as u32));
            n += 1;
            true
        })
    };
    match err {
        Some(e) => Err(e),
        None => Ok(present),
    }
}

/// Zformatuje `fmt` (bez koncove nuly) s argumenty z `args` do `out`;
/// prevody sirokych znaku podle kodovani aktualniho locale. Chyba prevodu
/// formatovani ukonci (vystup do te doby zustava).
pub fn format(out: &mut dyn Out, fmt: &[u8], args: &mut dyn Args) {
    let _ = run(out, fmt, args, crate::mb::current());
}

/// Jako [`format`], s kodovanim `enc` a chybou prevodu (EILSEQ).
pub fn format_enc(out: &mut dyn Out, fmt: &[u8], args: &mut dyn Args, enc: Enc) -> Result<(), Eilseq> {
    run(out, fmt, args, enc)
}

/// Formatovani do sirokych znaku (`wprintf`): format i vystup jsou siroke.
pub fn wformat(out: &mut dyn Out<u32>, fmt: &[u32], args: &mut dyn Args, enc: Enc) -> Result<(), Eilseq> {
    run(out, fmt, args, enc)
}

fn run<U: Unit>(out: &mut dyn Out<U>, fmt: &[U], args: &mut dyn Args, enc: Enc) -> Result<(), Eilseq> {
    let mut i = 0;
    // Ridici znaky formatu jsou ASCII; jiny znak je "neznamy" (0xFF).
    let next = |i: &mut usize| -> Option<u8> {
        let c = fmt.get(*i).map(|u| u.code());
        *i += 1;
        c.map(|c| if c < 0x80 { c as u8 } else { 0xFF })
    };
    while let Some(c) = fmt.get(i).copied() {
        i += 1;
        if c.code() != b'%' as u32 {
            out.put(c);
            continue;
        }
        let start = i - 1;
        let mut s = Spec::default();
        let mut c = next(&mut i);
        while let Some(f) = c {
            match f {
                b'-' => s.left = true,
                b'+' => s.plus = true,
                b' ' => s.space = true,
                b'#' => s.alt = true,
                b'0' => s.zero = true,
                _ => break,
            }
            c = next(&mut i);
        }
        if c == Some(b'*') {
            let w = args.signed(Len::Int);
            if w < 0 {
                s.left = true;
            }
            s.width = (w.unsigned_abs() as usize).min(MAX_WIDTH);
            c = next(&mut i);
        } else {
            while let Some(d @ b'0'..=b'9') = c {
                s.width = (s.width * 10 + (d - b'0') as usize).min(MAX_WIDTH);
                c = next(&mut i);
            }
        }
        if c == Some(b'.') {
            c = next(&mut i);
            if c == Some(b'*') {
                let p = args.signed(Len::Int);
                s.prec = (p >= 0).then_some((p as usize).min(MAX_WIDTH));
                c = next(&mut i);
            } else {
                let mut p = 0usize;
                while let Some(d @ b'0'..=b'9') = c {
                    p = (p * 10 + (d - b'0') as usize).min(MAX_WIDTH);
                    c = next(&mut i);
                }
                s.prec = Some(p);
            }
        }
        let mut len = Len::Int;
        let mut long_double = false;
        let mut wide_arg = false;
        match c {
            Some(b'h') => {
                c = next(&mut i);
                len = if c == Some(b'h') {
                    c = next(&mut i);
                    Len::Char
                } else {
                    Len::Short
                };
            }
            Some(b'l') => {
                c = next(&mut i);
                wide_arg = true;
                if c == Some(b'l') {
                    c = next(&mut i);
                    wide_arg = false;
                }
                len = Len::Long;
            }
            Some(b'z' | b'j' | b't') => {
                c = next(&mut i);
                len = Len::Long;
            }
            Some(b'L') => {
                c = next(&mut i);
                long_double = true;
            }
            _ => {}
        }
        match c {
            Some(b'd' | b'i') => {
                let v = args.signed(len);
                number(out, s, v < 0, v.unsigned_abs(), 10, false, false);
            }
            Some(b'u') => number(out, Spec { plus: false, space: false, ..s }, false, args.unsigned(len), 10, false, false),
            Some(b'o') => number(out, Spec { plus: false, space: false, ..s }, false, args.unsigned(len), 8, false, true),
            Some(b'x') => number(out, Spec { plus: false, space: false, ..s }, false, args.unsigned(len), 16, false, true),
            Some(b'X') => number(out, Spec { plus: false, space: false, ..s }, false, args.unsigned(len), 16, true, true),
            Some(b'p') => {
                let p = args.pointer() as u64;
                number(out, Spec { alt: true, plus: false, space: false, ..s }, false, p, 16, false, true);
            }
            Some(conv @ (b'c' | b'C' | b's' | b'S')) => {
                // `%C` a `%S` = `%lc` a `%ls` (XSI).
                let wide = wide_arg || conv.is_ascii_uppercase();
                let single = conv.eq_ignore_ascii_case(&b'c');
                let max = if single { usize::MAX } else { s.prec.unwrap_or(usize::MAX) };
                if s.width == 0 {
                    if !text::<U>(args, wide, single, max, enc, &mut |u| out.put(u))? {
                        pad_text(out, b"(null)");
                    }
                } else {
                    // Delku zname az po precteni: text do docasneho pole.
                    let mut tmp: Vec<U> = Vec::new();
                    if !text::<U>(args, wide, single, max, enc, &mut |u| tmp.push(u))? {
                        tmp = b"(null)".iter().map(|&b| U::from_ascii(b)).collect();
                    }
                    let fill = s.width.saturating_sub(tmp.len());
                    if !s.left {
                        pad(out, fill, b' ');
                    }
                    for &u in &tmp {
                        out.put(u);
                    }
                    if s.left {
                        pad(out, fill, b' ');
                    }
                }
            }
            Some(b'%') => out.put(U::from_ascii(b'%')),
            Some(b'n') => {
                let _ = args.pointer();
            }
            Some(conv @ (b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A')) => {
                if long_double {
                    // `long double` neni v VaList Rustu citelny (x86_64 ho
                    // predava v pameti po 16 B): radsi nic nespotrebovat.
                    out.put(U::from_ascii(b'?'));
                } else {
                    crate::float::format_float(&mut Widen(out), s, conv, args.double());
                }
            }
            Some(_) | None => {
                // Neznamy prevod: vypsat doslova (az po konec formatu).
                let end = i.min(fmt.len());
                for &u in &fmt[start..end] {
                    out.put(u);
                }
            }
        }
    }
    Ok(())
}

fn pad_text<U: Unit>(out: &mut dyn Out<U>, t: &[u8]) {
    for &b in t {
        out.put(U::from_ascii(b));
    }
}

/// Argumenty z `VaList` jazyka C (`vprintf`, `vsnprintf`).
pub struct VaArgs<'a, 'b>(pub &'a mut core::ffi::VaList<'b>);

impl Args for VaArgs<'_, '_> {
    fn signed(&mut self, len: Len) -> i64 {
        // SAFETY: format odpovida argumentum (kontrakt printf v C).
        unsafe {
            match len {
                Len::Char => self.0.next_arg::<i32>() as i8 as i64,
                Len::Short => self.0.next_arg::<i32>() as i16 as i64,
                Len::Int => self.0.next_arg::<i32>() as i64,
                Len::Long => self.0.next_arg::<i64>(),
            }
        }
    }

    fn unsigned(&mut self, len: Len) -> u64 {
        // SAFETY: dtto.
        unsafe {
            match len {
                Len::Char => self.0.next_arg::<u32>() as u8 as u64,
                Len::Short => self.0.next_arg::<u32>() as u16 as u64,
                Len::Int => self.0.next_arg::<u32>() as u64,
                Len::Long => self.0.next_arg::<u64>(),
            }
        }
    }

    fn pointer(&mut self) -> usize {
        // SAFETY: dtto.
        unsafe { self.0.next_arg::<*const u8>() as usize }
    }

    fn string(&mut self, out: &mut dyn FnMut(u8) -> bool) -> bool {
        // SAFETY: dtto; retezec je zakonceny nulou nebo `out` skonci driv.
        let p = unsafe { self.0.next_arg::<*const u8>() };
        if p.is_null() {
            return false;
        }
        let mut i = 0;
        loop {
            // SAFETY: viz vyse.
            let b = unsafe { *p.add(i) };
            if b == 0 || !out(b) {
                break;
            }
            i += 1;
        }
        true
    }

    fn wstring(&mut self, out: &mut dyn FnMut(u32) -> bool) -> bool {
        // SAFETY: dtto (`wchar_t` ma 32 bitu).
        let p = unsafe { self.0.next_arg::<*const u32>() };
        if p.is_null() {
            return false;
        }
        let mut i = 0;
        loop {
            // SAFETY: viz vyse.
            let c = unsafe { *p.add(i) };
            if c == 0 || !out(c) {
                break;
            }
            i += 1;
        }
        true
    }

    fn double(&mut self) -> f64 {
        // Vsechny programy jsou hard-float: `double` jde ve vektorovych
        // registrech (SSE / V), VaList ho cte podle ABI.
        // SAFETY: dtto.
        unsafe { self.0.next_arg::<f64>() }
    }
}
