//! Logo a nazev: vektorova data primo z prototypu (`design/assets`),
//! vykreslena do masek pokryti.
//!
//! Cesty pouzivaji jen absolutni prikazy `M`, `L`, `C` a `Z` a pravidlo
//! `fill-rule="evenodd"`; krivky se deli na usecky a plni rasterizerem
//! pisma (`font::Rasterizer::finish_even_odd`).

use alloc::vec::Vec;

use font::{Point, Rasterizer};

/// Symbol (`design/assets/logo/symbol-fialovy.svg`, viewBox -8 -12 400 400).
static SYMBOL_SVG: &str = include_str!("../../../design/assets/logo/symbol-fialovy.svg");
/// Nazev (`design/assets/logo-wordmark.js`, cesta v retezci JS).
static WORDMARK_JS: &str = include_str!("../../../design/assets/logo-wordmark.js");

/// ViewBox symbolu: pocatek a strana ctverce.
pub const SYMBOL_VIEW: (f32, f32, f32) = (-8.0, -12.0, 400.0);

/// Uzavreny obrys (lomena cara v jednotkach zdroje).
pub type Contour = Vec<(f32, f32)>;

/// Maska pokryti `w x h` (0-255).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mask {
    pub w: u32,
    pub h: u32,
    pub cov: Vec<u8>,
}

impl Mask {
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x as u32 >= self.w || y as u32 >= self.h {
            return 0;
        }
        self.cov[(y as u32 * self.w + x as u32) as usize]
    }
}

/// Obsah atributu `d` vsech cest v textu (JS i SVG; v JS jsou uvozovky
/// escapovane `\"`).
fn path_data(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find(" d=") {
        rest = &rest[i + 3..];
        let body = rest.trim_start_matches('\\').trim_start_matches('"');
        let end = body.find('"').unwrap_or(body.len());
        let d = body[..end].trim_end_matches('\\');
        out.push(d);
        rest = &body[end..];
    }
    out
}

/// Prevede data cesty (`M L C Z`, absolutni) na obrysy. Escape sekvence
/// z JS (`\r\n`) a jine neznama pismena se preskoci.
pub fn parse_path(d: &str) -> Vec<Contour> {
    let b = d.as_bytes();
    let mut i = 0;
    let mut out: Vec<Contour> = Vec::new();
    let mut cur: Contour = Vec::new();
    let mut cmd = b'M';
    let mut pen = (0.0f32, 0.0f32);
    let num = |i: &mut usize| -> Option<f32> {
        while *i < b.len() && matches!(b[*i], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            *i += 1;
        }
        let start = *i;
        while *i < b.len() && (b[*i].is_ascii_digit() || matches!(b[*i], b'.' | b'-' | b'+' | b'e' | b'E')) {
            *i += 1;
        }
        d.get(start..*i)?.parse::<f32>().ok()
    };
    loop {
        while i < b.len() && matches!(b[i], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        let c = b[i];
        if c == b'\\' {
            // `\r`, `\n` z retezce JS.
            i += 2;
            continue;
        }
        if c.is_ascii_alphabetic() {
            i += 1;
            match c {
                b'Z' | b'z' => {
                    if cur.len() > 2 {
                        out.push(core::mem::take(&mut cur));
                    }
                    cur.clear();
                }
                b'M' | b'L' | b'C' => cmd = c,
                _ => {}
            }
            continue;
        }
        match cmd {
            b'M' => {
                let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else { break };
                if cur.len() > 2 {
                    out.push(core::mem::take(&mut cur));
                }
                cur.clear();
                cur.push((x, y));
                pen = (x, y);
                // Dalsi dvojice po M jsou implicitni L.
                cmd = b'L';
            }
            b'L' => {
                let (Some(x), Some(y)) = (num(&mut i), num(&mut i)) else { break };
                cur.push((x, y));
                pen = (x, y);
            }
            b'C' => {
                let mut v = [0f32; 6];
                for k in &mut v {
                    match num(&mut i) {
                        Some(n) => *k = n,
                        None => return out,
                    }
                }
                let (p0, p1, p2, p3) = (pen, (v[0], v[1]), (v[2], v[3]), (v[4], v[5]));
                // Pevny pocet useku: krivky loga jsou kratke (jednotky az
                // desitky jednotek pri 400 na symbol).
                const N: u32 = 8;
                for k in 1..=N {
                    let t = k as f32 / N as f32;
                    let mt = 1.0 - t;
                    let a = mt * mt * mt;
                    let bb = 3.0 * mt * mt * t;
                    let cc = 3.0 * mt * t * t;
                    let dd = t * t * t;
                    cur.push((
                        a * p0.0 + bb * p1.0 + cc * p2.0 + dd * p3.0,
                        a * p0.1 + bb * p1.1 + cc * p2.1 + dd * p3.1,
                    ));
                }
                pen = p3;
            }
            _ => {
                i += 1;
            }
        }
    }
    if cur.len() > 2 {
        out.push(cur);
    }
    out
}

/// Obrysy symbolu loga.
pub fn symbol() -> Vec<Contour> {
    path_data(SYMBOL_SVG).into_iter().flat_map(parse_path).collect()
}

/// Obrysy nazvu.
pub fn wordmark() -> Vec<Contour> {
    path_data(WORDMARK_JS).into_iter().flat_map(parse_path).collect()
}

/// Ohraniceni obrysu (min x, min y, max x, max y).
pub fn bounds(contours: &[Contour]) -> (f32, f32, f32, f32) {
    let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in contours.iter().flatten() {
        b.0 = b.0.min(p.0);
        b.1 = b.1.min(p.1);
        b.2 = b.2.max(p.0);
        b.3 = b.3.max(p.1);
    }
    b
}

/// Vykresli obrysy do masky `w x h`: bod zdroje `(x, y)` je v masce
/// `((x - ox) * s, (y - oy) * s)`.
pub fn render(contours: &[Contour], w: u32, h: u32, ox: f32, oy: f32, s: f32) -> Mask {
    let (w, h) = (w.clamp(1, 2048), h.clamp(1, 2048));
    let mut r = Rasterizer::new(w, h);
    let tr = |p: (f32, f32)| Point { x: (p.0 - ox) * s, y: (p.1 - oy) * s };
    for c in contours {
        for seg in c.windows(2) {
            r.line(tr(seg[0]), tr(seg[1]));
        }
        if let (Some(&a), Some(&b)) = (c.last(), c.first()) {
            r.line(tr(a), tr(b));
        }
    }
    Mask { w, h, cov: r.finish_even_odd() }
}
