//! Plovouci carka pro `printf`: `%f %F %e %E %g %G %a %A`.
//!
//! Cislice dava `core::fmt` (presne zaokrouhleni binarni hodnoty, bez
//! alokace) do bufferu na zasobniku; tady se prevedou na tvar C (exponent
//! `e+05`, `%g` bez koncovych nul, sestnactkovy `%a`) a zarovnaji.

use core::fmt::{self, Write};

use crate::format::{MAX_WIDTH, Out, Spec, pad};

/// Nejdelsi telo: 309 cislic celeho cisla, tecka, presnost, exponent.
const CAP: usize = MAX_WIDTH + 340;

struct Text {
    buf: [u8; CAP],
    len: usize,
}

impl Text {
    fn new() -> Self {
        Text { buf: [0; CAP], len: 0 }
    }

    fn push(&mut self, b: u8) {
        if self.len < CAP {
            self.buf[self.len] = b;
            self.len += 1;
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

impl Write for Text {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for b in s.bytes() {
            self.push(b);
        }
        Ok(())
    }
}

/// `%e` z Rustu (`1.5e3`) na tvar C (`1.5e+03`); vraci exponent.
fn exp_form(t: &mut Text, v: f64, prec: usize, alt: bool) -> i32 {
    let mut raw = Text::new();
    let _ = write!(raw, "{:.*e}", prec, v);
    let s = raw.as_slice();
    let epos = s.iter().position(|&b| b == b'e').unwrap_or(s.len());
    let exp: i32 = core::str::from_utf8(&s[epos + 1..]).ok().and_then(|x| x.parse().ok()).unwrap_or(0);
    for &b in &s[..epos] {
        t.push(b);
    }
    if alt && prec == 0 {
        t.push(b'.');
    }
    t.push(b'e');
    t.push(if exp < 0 { b'-' } else { b'+' });
    let a = exp.unsigned_abs();
    if a < 10 {
        t.push(b'0');
    }
    let _ = write!(t, "{a}");
    exp
}

fn fixed_form(t: &mut Text, v: f64, prec: usize, alt: bool) {
    let _ = write!(t, "{:.*}", prec, v);
    if alt && prec == 0 {
        t.push(b'.');
    }
}

/// `%g`: odstrani koncove nuly za teckou (a samotnou tecku) pred
/// exponentem nebo koncem.
fn trim_zeros(t: &mut Text) {
    let s = t.as_slice();
    let Some(point) = s.iter().position(|&b| b == b'.') else { return };
    let end = s.iter().position(|&b| b == b'e').unwrap_or(s.len());
    let mut cut = end;
    while cut > point + 1 && s[cut - 1] == b'0' {
        cut -= 1;
    }
    if cut == point + 1 {
        cut = point;
    }
    let tail: [u8; 8] = {
        let mut x = [0u8; 8];
        let n = (t.len - end).min(8);
        x[..n].copy_from_slice(&t.buf[end..end + n]);
        x
    };
    let tail_len = (t.len - end).min(8);
    t.len = cut;
    for &b in &tail[..tail_len] {
        t.push(b);
    }
}

/// Sestnactkovy tvar `0x1.8p+3` (`%a`).
fn hex_form(t: &mut Text, v: f64, prec: Option<usize>, alt: bool) {
    let bits = v.to_bits();
    let mut exp = ((bits >> 52) & 0x7ff) as i32;
    let mut mant = bits & ((1u64 << 52) - 1);
    let mut lead = 1u64;
    if exp == 0 {
        if mant == 0 {
            lead = 0;
        } else {
            // Subnormalni cislo: normalizovat na 1.xxx.
            let shift = mant.leading_zeros() - 11;
            mant = (mant << shift) & ((1u64 << 52) - 1);
            exp = 1 - shift as i32;
        }
    }
    let e = if lead == 0 { 0 } else { exp - 1023 };
    let mut digits = 13usize;
    let mut m = mant;
    match prec {
        Some(p) if p < 13 => {
            // Zaokrouhleni na `p` cislic (polovina na sudou).
            let drop = (13 - p) * 4;
            let half = 1u64 << (drop - 1);
            let rem = m & ((1u64 << drop) - 1);
            m >>= drop;
            if rem > half || (rem == half && m & 1 == 1) {
                m += 1;
                if p == 0 || m >> (p * 4) != 0 {
                    m &= if p == 0 { 0 } else { (1u64 << (p * 4)) - 1 };
                    lead += 1;
                }
            }
            digits = p;
        }
        Some(_) => {}
        None => {
            while digits > 0 && m & 0xf == 0 {
                m >>= 4;
                digits -= 1;
            }
        }
    }
    let set = b"0123456789abcdef";
    t.push(b'0');
    t.push(b'x');
    t.push(set[(lead & 0xf) as usize]);
    let extra = prec.map_or(0, |p| p.saturating_sub(13));
    if digits > 0 || alt || extra > 0 {
        t.push(b'.');
    }
    for i in (0..digits).rev() {
        t.push(set[((m >> (i * 4)) & 0xf) as usize]);
    }
    for _ in 0..extra {
        t.push(b'0');
    }
    t.push(b'p');
    t.push(if e < 0 { b'-' } else { b'+' });
    let _ = write!(t, "{}", e.unsigned_abs());
}

/// Zformatuje `v` prevodem `conv` (`f F e E g G a A`) se specifikaci `s`.
pub(crate) fn format_float(out: &mut dyn Out, s: Spec, conv: u8, v: f64) {
    let upper = conv.is_ascii_uppercase();
    let neg = v.is_sign_negative();
    let a = v.abs();
    let mut t = Text::new();
    let finite = a.is_finite();
    if !finite {
        let w: &[u8] = if a.is_nan() { b"nan" } else { b"inf" };
        for &b in w {
            t.push(b);
        }
    } else {
        match conv.to_ascii_lowercase() {
            b'f' => fixed_form(&mut t, a, s.prec.unwrap_or(6), s.alt),
            b'e' => {
                exp_form(&mut t, a, s.prec.unwrap_or(6), s.alt);
            }
            b'g' => {
                let p = match s.prec {
                    Some(0) => 1,
                    Some(p) => p,
                    None => 6,
                };
                // Exponent po zaokrouhleni na `p` platnych cislic.
                let mut probe = Text::new();
                let x = exp_form(&mut probe, a, p - 1, false);
                if x < -4 || x >= p as i32 {
                    exp_form(&mut t, a, p - 1, s.alt);
                } else {
                    fixed_form(&mut t, a, (p as i32 - 1 - x) as usize, s.alt);
                }
                if !s.alt {
                    trim_zeros(&mut t);
                }
            }
            _ => hex_form(&mut t, a, s.prec, s.alt),
        }
    }
    if upper {
        for b in &mut t.buf[..t.len] {
            *b = b.to_ascii_uppercase();
        }
    }
    let sign: &[u8] = if neg {
        b"-"
    } else if s.plus {
        b"+"
    } else if s.space {
        b" "
    } else {
        b""
    };
    let body = sign.len() + t.len;
    let fill = s.width.saturating_sub(body);
    // Nulami se doplnuje jen konecne cislo (za znamenko a 0x).
    let zero = s.zero && !s.left && finite;
    if !s.left && !zero {
        pad(out, fill, b' ');
    }
    for &b in sign {
        out.put(b);
    }
    let digits = t.as_slice();
    let mut rest = digits;
    if zero {
        if conv.eq_ignore_ascii_case(&b'a') && digits.len() >= 2 {
            out.put(digits[0]);
            out.put(digits[1]);
            rest = &digits[2..];
        }
        pad(out, fill, b'0');
    }
    for &b in rest {
        out.put(b);
    }
    if s.left {
        pad(out, fill, b' ');
    }
}
