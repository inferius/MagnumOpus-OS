//! Rodina `scanf`: `sscanf`, `vsscanf`, `fscanf`, `vfscanf`, `scanf`;
//! stejny motor slouzi i siroke rodine `wscanf` (`wchar::wscanf`).
//!
//! Prevody `d i u o x X c s [ n p f e g a %` s sirkou, `*` (bez ulozeni)
//! a delkami `hh h l ll z j t L`; cisla jako `strtol`/`strtod`. `%lc`,
//! `%ls`, `%l[` ukladaji siroke znaky (v bajtovem `scanf` prevedene
//! z kodovani locale), `%c %s %[` v `wscanf` naopak vicebajtove. Vraci
//! pocet ulozenych prevodu, `EOF` pri konci vstupu pred prvnim prevodem.
//!
//! Motor pracuje se znaky `u32`: bajtovy vstup je bajt na znak, siroky
//! vstup siroky znak; format stejne.

use core::ffi::VaList;

use cstd_core::mb::Enc;

use crate::stdio::{FILE, fgetc, ungetc};
use crate::{c_bytes, c_char, c_int, errno};

/// Zdroj znaku s nahledem jednoho znaku.
pub(crate) trait Input {
    fn peek(&mut self) -> Option<u32>;
    fn next(&mut self) -> Option<u32>;
    /// Prectenych znaku (`%n`).
    fn count(&self) -> usize;
}

pub(crate) struct Units<'a, T> {
    pub(crate) s: &'a [T],
    pub(crate) pos: usize,
}

impl<T: Copy + Into<u32>> Input for Units<'_, T> {
    fn peek(&mut self) -> Option<u32> {
        self.s.get(self.pos).map(|&c| c.into())
    }
    fn next(&mut self) -> Option<u32> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }
    fn count(&self) -> usize {
        self.pos
    }
}

/// Bajtovy proud `FILE`.
struct Stream {
    f: *mut FILE,
    look: Option<Option<u32>>,
    n: usize,
}

impl Input for Stream {
    fn peek(&mut self) -> Option<u32> {
        if self.look.is_none() {
            // SAFETY: platny FILE (kontrakt C).
            let c = unsafe { fgetc(self.f) };
            self.look = Some((c >= 0).then_some(c as u32));
        }
        self.look.flatten()
    }
    fn next(&mut self) -> Option<u32> {
        let c = self.peek();
        self.look = None;
        if c.is_some() {
            self.n += 1;
        }
        c
    }
    fn count(&self) -> usize {
        self.n
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // Nahlednuty, nespotrebovany znak vratit do proudu.
        if let Some(Some(c)) = self.look {
            // SAFETY: platny FILE.
            unsafe { ungetc(c as c_int, self.f) };
        }
    }
}

/// Cil ulozeni (z `VaList`); testy na hostiteli maji vlastni.
pub trait Sink {
    fn int(&mut self, len: u8, v: i64);
    fn float(&mut self, len: u8, v: f64);
    /// Retezec `%s`/`%[`; `nul` = pridat nulu (ne u `%c`).
    fn bytes(&mut self, b: &[u8], nul: bool);
    /// Siroky retezec (`%ls`, `%lc`, `%l[`, `wchar_t *`).
    fn wide(&mut self, w: &[u32], nul: bool);
    fn ptr(&mut self, v: usize);
}

/// Rezim motoru: bajtovy (`scanf`) nebo siroky (`wscanf`) vstup
/// a kodovani pro prevody mezi nimi.
#[derive(Clone, Copy)]
pub(crate) struct Mode {
    pub(crate) wide: bool,
    pub(crate) enc: Enc,
}

/// Delky: 0 int, 1 char (hh), 2 short (h), 3 long/ll/z/j/t, 4 long double
/// (L), 5 = `l` u `c s [` (siroke).
fn parse_len(fmt: &[u32], i: &mut usize) -> u8 {
    let at = |i: usize| fmt.get(i).copied();
    match at(*i) {
        Some(0x68) => {
            *i += 1;
            if at(*i) == Some(0x68) {
                *i += 1;
                1
            } else {
                2
            }
        }
        Some(0x6C) => {
            *i += 1;
            if at(*i) == Some(0x6C) {
                *i += 1;
            }
            3
        }
        Some(0x7A | 0x6A | 0x74) => {
            *i += 1;
            3
        }
        Some(0x4C) => {
            *i += 1;
            4
        }
        _ => 0,
    }
}

fn is_ws(m: Mode, c: u32) -> bool {
    if m.wide { uniprop::is_space(c) } else { c < 0x80 && (c as u8).is_ascii_whitespace() }
}

fn skip_ws(m: Mode, inp: &mut dyn Input) {
    while inp.peek().is_some_and(|c| is_ws(m, c)) {
        inp.next();
    }
}

/// Precte znaky (nejvys `width`) podle povolenych znaku.
fn take(inp: &mut dyn Input, width: usize, ok: impl Fn(&[u32], u32) -> bool) -> Vec<u32> {
    let mut v = Vec::new();
    while v.len() < width {
        match inp.peek() {
            Some(c) if ok(&v, c) => {
                v.push(c);
                inp.next();
            }
            _ => break,
        }
    }
    v
}

/// Znaky cisla jako ASCII (jine predikaty cisel nepusti).
fn ascii(v: &[u32]) -> Vec<u8> {
    v.iter().map(|&c| c as u8).collect()
}

fn as_byte(c: u32) -> u8 {
    if c < 0x80 { c as u8 } else { 0 }
}

fn int_chars(base: u32) -> impl Fn(&[u32], u32) -> bool {
    move |v: &[u32], c: u32| {
        let c = as_byte(c);
        let v = ascii(v);
        if v.is_empty() && (c == b'+' || c == b'-') {
            return true;
        }
        let digits = v.iter().filter(|&&b| b != b'+' && b != b'-').count();
        if (base == 16 || base == 0) && digits == 1 && (c == b'x' || c == b'X') && v.last() == Some(&b'0') {
            return true;
        }
        let hex = v.iter().any(|&b| b == b'x' || b == b'X');
        let b = if base == 0 {
            if hex {
                16
            } else if v.iter().find(|&&b| b != b'+' && b != b'-') == Some(&b'0') {
                8
            } else {
                10
            }
        } else {
            base
        };
        (c as char).to_digit(b).is_some() || (base == 0 && digits == 0 && c == b'0')
    }
}

fn float_chars(v: &[u32], c: u32) -> bool {
    let c = as_byte(c);
    let v = ascii(v);
    let last = v.last().copied();
    let hex = v.iter().any(|&b| b == b'x' || b == b'X');
    match c {
        0 => false,
        b'+' | b'-' => v.is_empty() || matches!(last, Some(b'e' | b'E' | b'p' | b'P')),
        b'0'..=b'9' | b'.' => true,
        b'x' | b'X' => last == Some(b'0') && v.len() <= 2,
        b'p' | b'P' => hex,
        b'e' | b'E' if !hex => !v.iter().any(|&b| b == b'e' || b == b'E') && v.iter().any(u8::is_ascii_digit),
        c if hex => c.is_ascii_hexdigit(),
        // Slova inf, infinity, nan.
        c => b"infinityaINFINITYA".contains(&c) && !v.iter().any(u8::is_ascii_digit),
    }
}

/// Ulozi text prevodu `%c %s %[` v pozadovane podobe; `false` = znak
/// nejde prevest (EILSEQ).
fn store_text(m: Mode, out: &mut dyn Sink, t: &[u32], wide_dst: bool, nul: bool) -> bool {
    match (m.wide, wide_dst) {
        (false, false) => out.bytes(&ascii_bytes(t), nul),
        (true, true) => out.wide(t, nul),
        (false, true) => {
            let b = ascii_bytes(t);
            let Some(w) = crate::wchar::conv::decode_all(m.enc, &b) else { return false };
            out.wide(&w, nul);
        }
        (true, false) => {
            let Some(b) = crate::wchar::conv::encode_all(m.enc, t) else { return false };
            out.bytes(&b, nul);
        }
    }
    true
}

/// Bajty bajtoveho vstupu (znaky jsou 0-255).
fn ascii_bytes(t: &[u32]) -> Vec<u8> {
    t.iter().map(|&c| c as u8).collect()
}

/// Mnozina `%[...]` jako rozsahy znaku; vraci (rozsahy, negace).
fn parse_set(fmt: &[u32], i: &mut usize) -> (Vec<(u32, u32)>, bool) {
    let mut set = Vec::new();
    let neg = fmt.get(*i) == Some(&(b'^' as u32));
    if neg {
        *i += 1;
    }
    let mut first = true;
    while let Some(&c) = fmt.get(*i) {
        if c == b']' as u32 && !first {
            break;
        }
        if fmt.get(*i + 1) == Some(&(b'-' as u32)) && fmt.get(*i + 2).is_some_and(|&e| e != b']' as u32) {
            set.push((c, fmt[*i + 2]));
            *i += 3;
        } else {
            set.push((c, c));
            *i += 1;
        }
        first = false;
    }
    *i += 1;
    (set, neg)
}

/// Jadro `scanf`; vraci pocet ulozenych prevodu nebo `EOF` (-1).
pub(crate) fn scan_units(m: Mode, fmt: &[u32], inp: &mut dyn Input, out: &mut dyn Sink) -> c_int {
    let mut stored: c_int = 0;
    let mut any_conv = false;
    let mut i = 0;
    let at = |i: usize| fmt.get(i).copied();
    while i < fmt.len() {
        let f = fmt[i];
        if is_ws(m, f) {
            skip_ws(m, inp);
            i += 1;
            continue;
        }
        if f != b'%' as u32 {
            if inp.peek() != Some(f) {
                break;
            }
            inp.next();
            i += 1;
            continue;
        }
        i += 1;
        if at(i) == Some(b'%' as u32) {
            skip_ws(m, inp);
            if inp.next() != Some(b'%' as u32) {
                break;
            }
            i += 1;
            continue;
        }
        let suppress = at(i) == Some(b'*' as u32);
        if suppress {
            i += 1;
        }
        let mut width = 0usize;
        while let Some(d @ 0x30..=0x39) = at(i) {
            width = width.saturating_mul(10).saturating_add((d - 0x30) as usize);
            i += 1;
        }
        let len = parse_len(fmt, &mut i);
        let Some(conv) = at(i) else { break };
        let conv = as_byte(conv);
        i += 1;
        // `%C`, `%S` = `%lc`, `%ls`; `l` u textu = siroky cil.
        let wide_dst = len == 3 || conv == b'C' || conv == b'S';
        let conv = match conv {
            b'C' => b'c',
            b'S' => b's',
            c => c,
        };
        if conv != b'c' && conv != b'[' && conv != b'n' {
            skip_ws(m, inp);
        }
        if inp.peek().is_none() && conv != b'n' {
            return if any_conv || stored > 0 { stored } else { -1 };
        }
        let w = if width == 0 { usize::MAX } else { width };
        match conv {
            b'd' | b'i' | b'u' | b'o' | b'x' | b'X' | b'p' => {
                let base = match conv {
                    b'd' | b'u' => 10,
                    b'o' => 8,
                    b'x' | b'X' | b'p' => 16,
                    _ => 0,
                };
                let t = ascii(&take(inp, w, int_chars(base)));
                if !t.iter().any(u8::is_ascii_hexdigit) {
                    break;
                }
                let (v, neg, _, _) = cstd_core::stdlib::parse_integer(&t, base);
                let v = if neg { (v as i64).wrapping_neg() } else { v as i64 };
                any_conv = true;
                if !suppress {
                    if conv == b'p' {
                        out.ptr(v as usize);
                    } else {
                        out.int(len, v);
                    }
                    stored += 1;
                }
            }
            b'f' | b'e' | b'g' | b'a' | b'E' | b'G' | b'F' | b'A' => {
                let t = ascii(&take(inp, w, float_chars));
                let (v, used, _) = cstd_core::stdlib::parse_float(&t);
                if used == 0 {
                    break;
                }
                any_conv = true;
                if !suppress {
                    out.float(len, v);
                    stored += 1;
                }
            }
            b's' => {
                let t = take(inp, w, |_, c| !is_ws(m, c));
                any_conv = true;
                if !suppress {
                    if !store_text(m, out, &t, wide_dst, true) {
                        errno::set(errno::EILSEQ);
                        break;
                    }
                    stored += 1;
                }
            }
            b'c' => {
                let n = if width == 0 { 1 } else { width };
                let t = take(inp, n, |_, _| true);
                if t.len() < n {
                    break;
                }
                any_conv = true;
                if !suppress {
                    if !store_text(m, out, &t, wide_dst, false) {
                        errno::set(errno::EILSEQ);
                        break;
                    }
                    stored += 1;
                }
            }
            b'[' => {
                let (set, neg) = parse_set(fmt, &mut i);
                let t = take(inp, w, |_, c| set.iter().any(|&(a, b)| (a..=b).contains(&c)) != neg);
                if t.is_empty() {
                    break;
                }
                any_conv = true;
                if !suppress {
                    if !store_text(m, out, &t, wide_dst, true) {
                        errno::set(errno::EILSEQ);
                        break;
                    }
                    stored += 1;
                }
            }
            b'n' => {
                if !suppress {
                    out.int(len, inp.count() as i64);
                }
            }
            _ => break,
        }
    }
    stored
}

/// Jadro bajtoveho `scanf` (format v bajtech).
pub(crate) fn scan(fmt: &[u8], inp: &mut dyn Input, out: &mut dyn Sink) -> c_int {
    let f: Vec<u32> = fmt.iter().map(|&b| b as u32).collect();
    scan_units(Mode { wide: false, enc: crate::locale::ctype() }, &f, inp, out)
}

/// Cil z `VaList` (ukazatele z C).
pub(crate) struct VaSink<'a, 'b>(pub(crate) &'a mut VaList<'b>);

impl Sink for VaSink<'_, '_> {
    fn int(&mut self, len: u8, v: i64) {
        // SAFETY: format odpovida argumentum (kontrakt scanf).
        unsafe {
            match len {
                1 => *self.0.next_arg::<*mut i8>() = v as i8,
                2 => *self.0.next_arg::<*mut i16>() = v as i16,
                3 | 4 => *self.0.next_arg::<*mut i64>() = v,
                _ => *self.0.next_arg::<*mut i32>() = v as i32,
            }
        }
    }
    fn float(&mut self, len: u8, v: f64) {
        // SAFETY: dtto; `long double` (L) se neumi ulozit, bere se jako double.
        unsafe {
            match len {
                0 => *self.0.next_arg::<*mut f32>() = v as f32,
                _ => *self.0.next_arg::<*mut f64>() = v,
            }
        }
    }
    fn bytes(&mut self, b: &[u8], nul: bool) {
        // SAFETY: cil ma misto (kontrakt scanf: sirka nebo dost velky buffer).
        unsafe {
            let p = self.0.next_arg::<*mut u8>();
            core::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
            if nul {
                *p.add(b.len()) = 0;
            }
        }
    }
    fn wide(&mut self, w: &[u32], nul: bool) {
        // SAFETY: dtto (`wchar_t` ma 32 bitu).
        unsafe {
            let p = self.0.next_arg::<*mut u32>();
            core::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            if nul {
                *p.add(w.len()) = 0;
            }
        }
    }
    fn ptr(&mut self, v: usize) {
        // SAFETY: dtto.
        unsafe { *self.0.next_arg::<*mut usize>() = v };
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vsscanf(s: *const c_char, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: retezce C.
    let (src, f) = unsafe { (c_bytes(s), c_bytes(fmt)) };
    scan(f, &mut Units { s: src, pos: 0 }, &mut VaSink(&mut ap))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn sscanf(s: *const c_char, fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vsscanf(s, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vfscanf(f: *mut FILE, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: retezec C.
    let fm = unsafe { c_bytes(fmt) };
    scan(fm, &mut Stream { f, look: None, n: 0 }, &mut VaSink(&mut ap))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn fscanf(f: *mut FILE, fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: kontrakt C.
    unsafe { vfscanf(f, fmt, args) }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn scanf(fmt: *const c_char, args: ...) -> c_int {
    // SAFETY: stdin je platny FILE.
    unsafe { vfscanf(crate::stdio::stdin_ptr(), fmt, args) }
}

/// Testovaci vstup z bajtu (host testy).
pub fn scan_bytes(s: &[u8], fmt: &[u8], out: &mut dyn Sink) -> c_int {
    scan(fmt, &mut Units { s, pos: 0 }, out)
}
