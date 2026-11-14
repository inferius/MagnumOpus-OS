//! Rodina `scanf`: `sscanf`, `vsscanf`, `fscanf`, `vfscanf`, `scanf`.
//!
//! Prevody `d i u o x X c s [ n p f e g a %` s sirkou, `*` (bez ulozeni)
//! a delkami `hh h l ll z j t L`; cisla jako `strtol`/`strtod`. Vraci
//! pocet ulozenych prevodu, `EOF` pri konci vstupu pred prvnim prevodem.

use core::ffi::VaList;

use crate::stdio::{FILE, fgetc, ungetc};
use crate::{c_bytes, c_char, c_int};

/// Zdroj znaku s nahledem jednoho znaku.
pub(crate) trait Input {
    fn peek(&mut self) -> Option<u8>;
    fn next(&mut self) -> Option<u8>;
    /// Prectenych znaku (`%n`).
    fn count(&self) -> usize;
}

struct Bytes<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Input for Bytes<'_> {
    fn peek(&mut self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }
    fn next(&mut self) -> Option<u8> {
        let c = self.peek()?;
        self.pos += 1;
        Some(c)
    }
    fn count(&self) -> usize {
        self.pos
    }
}

struct Stream {
    f: *mut FILE,
    look: Option<Option<u8>>,
    n: usize,
}

impl Input for Stream {
    fn peek(&mut self) -> Option<u8> {
        if self.look.is_none() {
            // SAFETY: platny FILE (kontrakt C).
            let c = unsafe { fgetc(self.f) };
            self.look = Some((c >= 0).then_some(c as u8));
        }
        self.look.flatten()
    }
    fn next(&mut self) -> Option<u8> {
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
            unsafe { ungetc(c_int::from(c), self.f) };
        }
    }
}

/// Cil ulozeni (z `VaList`); testy na hostiteli maji vlastni.
pub trait Sink {
    fn int(&mut self, len: u8, v: i64);
    fn float(&mut self, len: u8, v: f64);
    /// Retezec `%s`/`%[`; `nul` = pridat nulu (ne u `%c`).
    fn bytes(&mut self, b: &[u8], nul: bool);
    fn ptr(&mut self, v: usize);
}

/// Delky: 0 int, 1 char (hh), 2 short (h), 3 long/ll/z/j/t, 4 long double (L).
fn parse_len(fmt: &[u8], i: &mut usize) -> u8 {
    match fmt.get(*i) {
        Some(b'h') => {
            *i += 1;
            if fmt.get(*i) == Some(&b'h') {
                *i += 1;
                1
            } else {
                2
            }
        }
        Some(b'l') => {
            *i += 1;
            if fmt.get(*i) == Some(&b'l') {
                *i += 1;
            }
            3
        }
        Some(b'z' | b'j' | b't') => {
            *i += 1;
            3
        }
        Some(b'L') => {
            *i += 1;
            4
        }
        _ => 0,
    }
}

fn skip_ws(inp: &mut dyn Input) {
    while inp.peek().is_some_and(|c| c.is_ascii_whitespace()) {
        inp.next();
    }
}

/// Precte cislo do bufferu (nejvys `width` znaku) podle povolenych znaku.
fn take(inp: &mut dyn Input, width: usize, ok: impl Fn(&[u8], u8) -> bool) -> Vec<u8> {
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

fn int_chars(base: u32) -> impl Fn(&[u8], u8) -> bool {
    move |v: &[u8], c: u8| {
        if v.is_empty() && (c == b'+' || c == b'-') {
            return true;
        }
        let digits = v.iter().filter(|&&b| b != b'+' && b != b'-').count();
        if base == 16 || base == 0 {
            // 0x prefix
            if digits == 1 && (c == b'x' || c == b'X') && v.last() == Some(&b'0') {
                return true;
            }
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

fn float_chars(v: &[u8], c: u8) -> bool {
    let last = v.last().copied();
    let hex = v.iter().any(|&b| b == b'x' || b == b'X');
    match c {
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

/// Jadro `scanf`; vraci pocet ulozenych prevodu nebo `EOF` (-1).
pub(crate) fn scan(fmt: &[u8], inp: &mut dyn Input, out: &mut dyn Sink) -> c_int {
    let mut stored: c_int = 0;
    let mut any_conv = false;
    let mut i = 0;
    while i < fmt.len() {
        let f = fmt[i];
        if f.is_ascii_whitespace() {
            skip_ws(inp);
            i += 1;
            continue;
        }
        if f != b'%' {
            if inp.peek() != Some(f) {
                break;
            }
            inp.next();
            i += 1;
            continue;
        }
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            skip_ws(inp);
            if inp.next() != Some(b'%') {
                break;
            }
            i += 1;
            continue;
        }
        let suppress = fmt.get(i) == Some(&b'*');
        if suppress {
            i += 1;
        }
        let mut width = 0usize;
        while let Some(d @ b'0'..=b'9') = fmt.get(i).copied() {
            width = width * 10 + usize::from(d - b'0');
            i += 1;
        }
        let len = parse_len(fmt, &mut i);
        let Some(&conv) = fmt.get(i) else { break };
        i += 1;
        if conv != b'c' && conv != b'[' && conv != b'n' {
            skip_ws(inp);
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
                let t = take(inp, w, int_chars(base));
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
                let t = take(inp, w, float_chars);
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
                let t = take(inp, w, |_, c| !c.is_ascii_whitespace());
                any_conv = true;
                if !suppress {
                    out.bytes(&t, true);
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
                    out.bytes(&t, false);
                    stored += 1;
                }
            }
            b'[' => {
                let mut set = [false; 256];
                let neg = fmt.get(i) == Some(&b'^');
                if neg {
                    i += 1;
                }
                let mut first = true;
                while let Some(&c) = fmt.get(i) {
                    if c == b']' && !first {
                        break;
                    }
                    if fmt.get(i + 1) == Some(&b'-') && fmt.get(i + 2).is_some_and(|&e| e != b']') {
                        for x in c..=fmt[i + 2] {
                            set[x as usize] = true;
                        }
                        i += 3;
                    } else {
                        set[c as usize] = true;
                        i += 1;
                    }
                    first = false;
                }
                i += 1;
                let t = take(inp, w, |_, c| set[c as usize] != neg);
                if t.is_empty() {
                    break;
                }
                any_conv = true;
                if !suppress {
                    out.bytes(&t, true);
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

/// Cil z `VaList` (ukazatele z C).
struct VaSink<'a, 'b>(&'a mut VaList<'b>);

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
    fn ptr(&mut self, v: usize) {
        // SAFETY: dtto.
        unsafe { *self.0.next_arg::<*mut usize>() = v };
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn vsscanf(s: *const c_char, fmt: *const c_char, mut ap: VaList) -> c_int {
    // SAFETY: retezce C.
    let (src, f) = unsafe { (c_bytes(s), c_bytes(fmt)) };
    scan(f, &mut Bytes { s: src, pos: 0 }, &mut VaSink(&mut ap))
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
    scan(fmt, &mut Bytes { s, pos: 0 }, out)
}
