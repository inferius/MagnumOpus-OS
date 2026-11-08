//! Formatovani ve stylu `printf` (jadro `vsnprintf` a `vprintf`).
//!
//! Podporovane: priznaky `-+ #0`, sirka a presnost (i `*`), delky `hh h l
//! ll z j t`, prevody `d i u o x X c s p %`. Cela cisla se ctou podle delky
//! (bez delky jako `int`, C je tak predava). Zamerne:
//!
//! - `%n` nic nezapise (klasicka bezpecnostni dira printf), jen spotrebuje
//!   argument,
//! - neznamy prevod se vypise doslova (`%q` -> `%q`) a nic nespotrebuje,
//! - plovouci carka (`%f`, `%e`, `%g`) zatim neni: programy bezi bez FPU;
//!   argument se spotrebuje a vypise se `?`.
//!
//! Zdroj argumentu je rozhrani [`Args`]: v systemu `VaList` z C, v testech
//! seznam hodnot - samotne formatovani je tak testovatelne na hostiteli.

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
    /// Dalsi argument jako retezec; cte nejvys `max` bajtu (presnost).
    /// `None` = nulovy ukazatel. Volajici bajty preda `out` po jednom.
    fn string(&mut self, max: usize, out: &mut dyn FnMut(u8)) -> bool;
    /// Dalsi argument typu `double` (jen spotrebovat, viz modul).
    fn skip_double(&mut self);
}

/// Kam formatovani pise.
pub trait Out {
    fn put(&mut self, b: u8);
}

/// Zapis do pole pevne delky s pocitanim celkove delky (`snprintf` vraci
/// delku, kterou by vystup mel bez oriznuti).
pub struct Buf<'a> {
    pub buf: &'a mut [u8],
    pub pos: usize,
    pub total: usize,
}

impl Out for Buf<'_> {
    fn put(&mut self, b: u8) {
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b;
            self.pos += 1;
        }
        self.total += 1;
    }
}

#[derive(Default, Clone, Copy)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alt: bool,
    zero: bool,
    width: usize,
    prec: Option<usize>,
}

/// Rozumny strop sirky a presnosti: `%2000000000d` nesmi zablokovat program
/// zapisem gigabajtu mezer (vystup by stejne oriznul buffer).
const MAX_WIDTH: usize = 4096;

fn pad(out: &mut dyn Out, n: usize, c: u8) {
    for _ in 0..n {
        out.put(c);
    }
}

fn number(out: &mut dyn Out, s: Spec, neg: bool, mag: u64, base: u64, upper: bool, prefix_alt: bool) {
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
        out.put(b);
    }
    if !s.left && s.zero && s.prec.is_none() {
        pad(out, fill, b'0');
    }
    pad(out, zeros, b'0');
    for i in (0..n).rev() {
        out.put(tmp[i]);
    }
    if s.left {
        pad(out, fill, b' ');
    }
}

/// Zformatuje `fmt` (bez koncove nuly) s argumenty z `args` do `out`.
pub fn format(out: &mut dyn Out, fmt: &[u8], args: &mut dyn Args) {
    let mut i = 0;
    let next = |i: &mut usize| -> Option<u8> {
        let c = fmt.get(*i).copied();
        *i += 1;
        c
    };
    while let Some(c) = next(&mut i) {
        if c != b'%' {
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
                if c == Some(b'l') {
                    c = next(&mut i);
                }
                len = Len::Long;
            }
            Some(b'z' | b'j' | b't') => {
                c = next(&mut i);
                len = Len::Long;
            }
            Some(b'L') => c = next(&mut i),
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
            Some(b'c') => {
                let ch = args.signed(Len::Int) as u8;
                let fill = s.width.saturating_sub(1);
                if !s.left {
                    pad(out, fill, b' ');
                }
                out.put(ch);
                if s.left {
                    pad(out, fill, b' ');
                }
            }
            Some(b's') => {
                // Delku zname az po precteni: retezec nejdriv do docasneho
                // pole (sirka zarovnani), dlouhe retezce bez zarovnani rovnou.
                let max = s.prec.unwrap_or(usize::MAX);
                if s.width == 0 {
                    if !args.string(max, &mut |b| out.put(b)) {
                        for &b in b"(null)" {
                            out.put(b);
                        }
                    }
                } else {
                    let mut tmp = [0u8; 256];
                    let mut n = 0usize;
                    let mut count = 0usize;
                    let ok = args.string(max, &mut |b| {
                        if n < tmp.len() {
                            tmp[n] = b;
                            n += 1;
                        }
                        count += 1;
                    });
                    let text: &[u8] = if ok { &tmp[..n] } else { b"(null)" };
                    let shown = if ok { count.min(tmp.len()) } else { text.len() };
                    let fill = s.width.saturating_sub(shown);
                    if !s.left {
                        pad(out, fill, b' ');
                    }
                    for &b in text {
                        out.put(b);
                    }
                    if s.left {
                        pad(out, fill, b' ');
                    }
                }
            }
            Some(b'%') => out.put(b'%'),
            Some(b'n') => {
                let _ = args.pointer();
            }
            Some(b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A') => {
                args.skip_double();
                out.put(b'?');
            }
            Some(_) | None => {
                // Neznamy prevod: vypsat doslova (az po konec formatu).
                let end = i.min(fmt.len());
                for &b in &fmt[start..end] {
                    out.put(b);
                }
            }
        }
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

    fn string(&mut self, max: usize, out: &mut dyn FnMut(u8)) -> bool {
        // SAFETY: dtto; retezec je zakonceny nulou nebo ma aspon `max` bajtu.
        let p = unsafe { self.0.next_arg::<*const u8>() };
        if p.is_null() {
            return false;
        }
        let mut i = 0;
        while i < max {
            // SAFETY: viz vyse.
            let b = unsafe { *p.add(i) };
            if b == 0 {
                break;
            }
            out(b);
            i += 1;
        }
        true
    }

    fn skip_double(&mut self) {
        // Bez FPU (soft-float ABI) se `double` predava v obecnych registrech
        // a na zasobniku jako 64bitove slovo.
        // SAFETY: dtto.
        let _ = unsafe { self.0.next_arg::<u64>() };
    }
}
