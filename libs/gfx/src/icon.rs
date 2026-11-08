//! Ikony design systemu: linkove, mrizka 24 px, tah 2 px se zaoblenymi
//! konci a spoji.
//!
//! Zdrojem je primo sada z prototypu (`design/assets/mo.js`, objekt
//! `ICONS`), vlozena pri prekladu: ikona v prototypu a v systemu je tataz
//! data. Umi se podmnozina SVG, kterou sada pouziva: `path` (prikazy
//! `M L H V C S Q T A Z`, i relativni), `rect` (i `rx`), `circle`; prvek
//! s `fill="currentColor"` se vyplni.
//!
//! Tah se kresli vzdalenostnim polem: pokryti pixelu je
//! `clamp(sirka / 2 + 0.5 - vzdalenost od cary)`, takze konce a spoje jsou
//! sami od sebe kulate. Vypln jde pres rasterizer pisma.

use alloc::string::String;
use alloc::vec::Vec;

use font::{Point, Rasterizer};

use crate::{Canvas, Rect, over, scale};

/// Zdroj ikon (`design/assets/mo.js`).
pub static SOURCE: &str = include_str!("../../../design/assets/mo.js");

/// Cast ikony: lomena cara (tah) nebo uzavreny tvar (vypln).
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub points: Vec<(f32, f32)>,
    pub closed: bool,
    pub fill: bool,
}

/// Ikona v souradnicich 0..24.
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    pub name: String,
    pub parts: Vec<Part>,
}

/// Vykreslena ikona: maska pokryti `size x size`.
#[derive(Debug, Clone)]
pub struct IconMask {
    pub size: u32,
    pub coverage: Vec<u8>,
}

/// Precte sadu ikon ze zdroje prototypu: JSON objekt za `var ICONS = `
/// ve tvaru `{"jmeno":{"c":"kategorie","f":"<vypln>","l":"<linka>"},...}`
/// (generuje `design/tools/build-icons.mjs`). Kresli se linka `l`; mekka
/// vypln `f` s prechodem barev kategorie prijde s vektorovym rasterizerem.
pub fn parse_set(src: &str) -> Vec<Icon> {
    let mut out = Vec::new();
    let Some(start) = src.find("var ICONS = {") else { return out };
    let mut s = &src[start + "var ICONS = {".len()..];
    loop {
        s = s.trim_start_matches([' ', '\n', '\r', '\t', ',']);
        if !s.starts_with('"') {
            break;
        }
        let Some((name, rest)) = json_str(s) else { break };
        let Some(rest) = rest.trim_start().strip_prefix(':').map(str::trim_start) else { break };
        let Some(mut rest) = rest.strip_prefix('{') else { break };
        let mut line = String::new();
        loop {
            rest = rest.trim_start_matches([' ', ',']);
            if let Some(r) = rest.strip_prefix('}') {
                rest = r;
                break;
            }
            let Some((key, r)) = json_str(rest) else { return out };
            let Some(r) = r.trim_start().strip_prefix(':').map(str::trim_start) else { return out };
            let Some((val, r)) = json_str(r) else { return out };
            if key == "l" {
                line = val;
            }
            rest = r;
        }
        out.push(Icon { name, parts: parse_svg(&line) });
        s = rest;
    }
    out
}

/// Retezec JSON na zacatku `s` (vcetne uvozovek) -> (hodnota, zbytek).
fn json_str(s: &str) -> Option<(String, &str)> {
    let body = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = body.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &body[i + 1..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'u' => {
                    let mut v = 0u32;
                    for _ in 0..4 {
                        v = v * 16 + chars.next()?.1.to_digit(16)?;
                    }
                    out.push(char::from_u32(v).unwrap_or('?'));
                }
                e => out.push(e),
            },
            c => out.push(c),
        }
    }
    None
}

/// Prvky SVG -> casti (lomene cary).
pub fn parse_svg(s: &str) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let Some(end) = rest.find('>') else { break };
        let el = &rest[..end];
        rest = &rest[end + 1..];
        let (tag, attrs) = el.split_once(' ').unwrap_or((el, ""));
        let fill = attr(attrs, "fill") == Some("currentColor");
        let num = |n: &str| attr(attrs, n).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
        let before = parts.len();
        match tag {
            "path" => {
                if let Some(d) = attr(attrs, "d") {
                    path(d, &mut parts);
                }
            }
            "rect" => {
                let (x, y, w, h) = (num("x"), num("y"), num("width"), num("height"));
                let rx = num("rx").min(w / 2.0).min(h / 2.0);
                parts.push(Part { points: rounded_rect(x, y, w, h, rx), closed: true, fill: false });
            }
            "circle" => {
                let (cx, cy, r) = (num("cx"), num("cy"), num("r"));
                let mut pts = Vec::new();
                let steps = 32;
                for k in 0..steps {
                    let a = k as f32 / steps as f32 * core::f32::consts::TAU;
                    pts.push((cx + r * cos(a), cy + r * sin(a)));
                }
                parts.push(Part { points: pts, closed: true, fill: false });
            }
            _ => {}
        }
        for p in &mut parts[before..] {
            p.fill = fill;
        }
    }
    parts
}

fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut s = attrs;
    loop {
        let i = s.find(name)?;
        let after = &s[i + name.len()..];
        let boundary = i == 0 || s.as_bytes()[i - 1] == b' ';
        if boundary && let Some(v) = after.strip_prefix("=\"") {
            return v.find('"').map(|e| &v[..e]);
        }
        s = after;
    }
}

fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<(f32, f32)> {
    if r <= 0.0 {
        return alloc::vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)];
    }
    let mut pts = Vec::new();
    let corners = [(x + w - r, y + r, -0.25f32), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 0.25), (x + r, y + r, 0.5)];
    for (cx, cy, start) in corners {
        for k in 0..=6 {
            let a = (start + k as f32 / 24.0) * core::f32::consts::TAU;
            pts.push((cx + r * cos(a), cy + r * sin(a)));
        }
    }
    pts
}

/// Cteni cisel a prikazu z atributu `d`.
struct Tok<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tok<'_> {
    fn skip(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\n' | b'\t') {
            self.i += 1;
        }
    }

    fn cmd(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn has_num(&mut self) -> bool {
        self.skip();
        self.s.get(self.i).is_some_and(|&c| c.is_ascii_digit() || matches!(c, b'-' | b'.' | b'+'))
    }

    fn num(&mut self) -> Option<f32> {
        self.skip();
        let start = self.i;
        let mut seen_dot = false;
        let mut seen_e = false;
        while let Some(&c) = self.s.get(self.i) {
            let ok = c.is_ascii_digit()
                || ((c == b'-' || c == b'+') && (self.i == start || matches!(self.s[self.i - 1], b'e' | b'E')))
                || (c == b'.' && !seen_dot && !seen_e)
                || ((c == b'e' || c == b'E') && !seen_e && self.i > start);
            if !ok {
                break;
            }
            seen_dot |= c == b'.';
            seen_e |= c == b'e' || c == b'E';
            self.i += 1;
        }
        core::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok()
    }

    /// Priznak oblouku (`0` nebo `1`, muze byt bez oddelovace).
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

/// Jemnost deleni krivek (usecek na krivku).
const CURVE_STEPS: usize = 12;

fn path(d: &str, out: &mut Vec<Part>) {
    let mut t = Tok { s: d.as_bytes(), i: 0 };
    let mut cur = (0f32, 0f32);
    let mut start = cur;
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut last_ctrl: Option<(f32, f32)> = None;
    let mut cmd = b'M';
    let flush = |pts: &mut Vec<(f32, f32)>, closed: bool, out: &mut Vec<Part>| {
        if pts.len() > 1 || (pts.len() == 1 && !closed) {
            out.push(Part { points: core::mem::take(pts), closed, fill: false });
        }
        pts.clear();
    };
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        if let Some(c) = t.cmd() {
            cmd = c;
        } else if !t.has_num() {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { cur } else { (0.0, 0.0) };
        let pt = |t: &mut Tok<'_>| -> Option<(f32, f32)> { Some((t.num()? + base.0, t.num()? + base.1)) };
        let ok = (|| -> Option<()> {
            match cmd.to_ascii_uppercase() {
                b'M' => {
                    flush(&mut pts, false, out);
                    cur = pt(&mut t)?;
                    start = cur;
                    pts.push(cur);
                    // Dalsi dvojice za M jsou L.
                    cmd = if rel { b'l' } else { b'L' };
                    last_ctrl = None;
                }
                b'L' => {
                    cur = pt(&mut t)?;
                    pts.push(cur);
                    last_ctrl = None;
                }
                b'H' => {
                    let x = t.num()?;
                    cur.0 = if rel { cur.0 + x } else { x };
                    pts.push(cur);
                    last_ctrl = None;
                }
                b'V' => {
                    let y = t.num()?;
                    cur.1 = if rel { cur.1 + y } else { y };
                    pts.push(cur);
                    last_ctrl = None;
                }
                b'C' | b'S' => {
                    let c1 = if cmd.to_ascii_uppercase() == b'C' {
                        pt(&mut t)?
                    } else {
                        last_ctrl.map(|c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1)).unwrap_or(cur)
                    };
                    let c2 = pt(&mut t)?;
                    let p = pt(&mut t)?;
                    for k in 1..=CURVE_STEPS {
                        let s = k as f32 / CURVE_STEPS as f32;
                        let u = 1.0 - s;
                        let x = u * u * u * cur.0 + 3.0 * u * u * s * c1.0 + 3.0 * u * s * s * c2.0 + s * s * s * p.0;
                        let y = u * u * u * cur.1 + 3.0 * u * u * s * c1.1 + 3.0 * u * s * s * c2.1 + s * s * s * p.1;
                        pts.push((x, y));
                    }
                    last_ctrl = Some(c2);
                    cur = p;
                }
                b'Q' | b'T' => {
                    let c = if cmd.to_ascii_uppercase() == b'Q' {
                        pt(&mut t)?
                    } else {
                        last_ctrl.map(|c| (2.0 * cur.0 - c.0, 2.0 * cur.1 - c.1)).unwrap_or(cur)
                    };
                    let p = pt(&mut t)?;
                    for k in 1..=CURVE_STEPS {
                        let s = k as f32 / CURVE_STEPS as f32;
                        let u = 1.0 - s;
                        pts.push((u * u * cur.0 + 2.0 * u * s * c.0 + s * s * p.0, u * u * cur.1 + 2.0 * u * s * c.1 + s * s * p.1));
                    }
                    last_ctrl = Some(c);
                    cur = p;
                }
                b'A' => {
                    let (rx, ry, rot) = (t.num()?, t.num()?, t.num()?);
                    let (large, sweep) = (t.flag()?, t.flag()?);
                    let p = pt(&mut t)?;
                    arc(cur, p, rx, ry, rot, large, sweep, &mut pts);
                    cur = p;
                    last_ctrl = None;
                }
                b'Z' => {
                    pts.push(start);
                    flush(&mut pts, true, out);
                    cur = start;
                    pts.push(cur);
                    last_ctrl = None;
                    // Za Z muze nasledovat jen prikaz.
                    if !t.cmd_ahead() {
                        return None;
                    }
                }
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            break;
        }
    }
    flush(&mut pts, false, out);
}

impl Tok<'_> {
    fn cmd_ahead(&mut self) -> bool {
        self.skip();
        self.s.get(self.i).is_some_and(u8::is_ascii_alphabetic)
    }
}

/// Eliptický oblouk SVG (prevod z koncovych bodu na stred, SVG 1.1 F.6.5).
#[allow(clippy::too_many_arguments)]
fn arc(p0: (f32, f32), p1: (f32, f32), rx: f32, ry: f32, rot_deg: f32, large: bool, sweep: bool, pts: &mut Vec<(f32, f32)>) {
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx == 0.0 || ry == 0.0 || p0 == p1 {
        pts.push(p1);
        return;
    }
    let phi = rot_deg.to_radians();
    let (cp, sp) = (cos(phi), sin(phi));
    let dx = (p0.0 - p1.0) / 2.0;
    let dy = (p0.1 - p1.1) / 2.0;
    let x1 = cp * dx + sp * dy;
    let y1 = -sp * dx + cp * dy;
    let lam = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lam > 1.0 {
        let s = crate::sqrtf(lam);
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut co = crate::sqrtf((num / den).max(0.0));
    if large == sweep {
        co = -co;
    }
    let cxp = co * rx * y1 / ry;
    let cyp = -co * ry * x1 / rx;
    let cx = cp * cxp - sp * cyp + (p0.0 + p1.0) / 2.0;
    let cy = sp * cxp + cp * cyp + (p0.1 + p1.1) / 2.0;
    let ang = |ux: f32, uy: f32, vx: f32, vy: f32| atan2(ux * vy - uy * vx, ux * vx + uy * vy);
    let t1 = ang(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut dt = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && dt > 0.0 {
        dt -= core::f32::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += core::f32::consts::TAU;
    }
    let steps = ((dt.abs() / core::f32::consts::TAU * 32.0) as usize).clamp(2, 32);
    for k in 1..=steps {
        let a = t1 + dt * k as f32 / steps as f32;
        let (ca, sa) = (cos(a), sin(a));
        pts.push((cx + rx * ca * cp - ry * sa * sp, cy + rx * ca * sp + ry * sa * cp));
    }
}

impl Icon {
    /// Vykresli ikonu do masky `size x size`; tah je `stroke` v jednotkach
    /// mrizky 24 (prototyp: 1,75, tlacitka okna 2).
    pub fn render(&self, size: u32, stroke: f32) -> IconMask {
        let size = size.clamp(1, 256);
        let s = size as f32 / 24.0;
        let hw = stroke * s / 2.0;
        let mut cov = alloc::vec![0f32; (size * size) as usize];
        // Vyplne pres rasterizer pisma.
        let fills: Vec<&Part> = self.parts.iter().filter(|p| p.fill).collect();
        if !fills.is_empty() {
            let mut r = Rasterizer::new(size, size);
            for p in &fills {
                for w in p.points.windows(2) {
                    r.line(Point { x: w[0].0 * s, y: w[0].1 * s }, Point { x: w[1].0 * s, y: w[1].1 * s });
                }
                if let (Some(a), Some(b)) = (p.points.last(), p.points.first()) {
                    r.line(Point { x: a.0 * s, y: a.1 * s }, Point { x: b.0 * s, y: b.1 * s });
                }
            }
            for (c, v) in cov.iter_mut().zip(r.finish()) {
                *c = v as f32 / 255.0;
            }
        }
        // Tahy (i obrys vyplnenych tvaru, jako v SVG se `stroke`).
        let mut segs: Vec<((f32, f32), (f32, f32))> = Vec::new();
        for p in &self.parts {
            let pts: Vec<(f32, f32)> = p.points.iter().map(|&(x, y)| (x * s, y * s)).collect();
            if pts.len() == 1 {
                segs.push((pts[0], pts[0]));
            }
            for w in pts.windows(2) {
                segs.push((w[0], w[1]));
            }
            if p.closed && pts.len() > 2 {
                segs.push((pts[pts.len() - 1], pts[0]));
            }
        }
        for y in 0..size {
            for x in 0..size {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let mut d2 = f32::MAX;
                for &(a, b) in &segs {
                    d2 = d2.min(seg_dist2(px, py, a, b));
                    if d2 == 0.0 {
                        break;
                    }
                }
                let d = crate::sqrtf(d2);
                let c = (hw + 0.5 - d).clamp(0.0, 1.0);
                let i = (y * size + x) as usize;
                cov[i] = cov[i].max(c);
            }
        }
        IconMask { size, coverage: cov.iter().map(|&c| (c * 255.0 + 0.5) as u8).collect() }
    }
}

fn seg_dist2(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (px - a.0, py - a.1);
    let l2 = vx * vx + vy * vy;
    let t = if l2 > 0.0 { ((wx * vx + wy * vy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (dx, dy) = (wx - t * vx, wy - t * vy);
    dx * dx + dy * dy
}

/// Sada ikon s mezipameti vykreslenych masek (klic: jmeno, velikost, tah).
pub struct Icons {
    set: Vec<Icon>,
    cache: Vec<((usize, u32, u32), IconMask)>,
}

impl Default for Icons {
    fn default() -> Self {
        Self::new()
    }
}

impl Icons {
    /// Sada z prototypu ([`SOURCE`]).
    pub fn new() -> Self {
        Self { set: parse_set(SOURCE), cache: Vec::new() }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.set.iter().map(|i| i.name.as_str())
    }

    pub fn get(&self, name: &str) -> Option<&Icon> {
        self.set.iter().find(|i| i.name == name)
    }

    /// Nakresli ikonu `name` velikosti `size` s levym hornim rohem (`x`,
    /// `y`) barvou `color`. Neznama ikona = nic.
    pub fn draw(&mut self, c: &mut Canvas<'_>, name: &str, x: i32, y: i32, size: u32, stroke: f32, color: u32) {
        let Some(i) = self.set.iter().position(|i| i.name == name) else { return };
        let key = (i, size, (stroke * 100.0) as u32);
        let idx = match self.cache.iter().position(|(k, _)| *k == key) {
            Some(p) => p,
            None => {
                if self.cache.len() >= 256 {
                    self.cache.clear();
                }
                let m = self.set[i].render(size, stroke);
                self.cache.push((key, m));
                self.cache.len() - 1
            }
        };
        let m = &self.cache[idx].1;
        let r = Rect::new(x, y, m.size, m.size);
        let (size, cov) = (m.size, &m.coverage);
        c.rows(r, |row, rx, ry| {
            let my = (ry - y) as u32;
            for (k, p) in row.iter_mut().enumerate() {
                let mx = (rx - x) as u32 + k as u32;
                let a = cov[(my * size + mx) as usize] as u32;
                if a > 0 {
                    *p = over(*p, scale(color, a));
                }
            }
        });
    }
}

// Goniometrie bez `std` (rady; staci na ikony).
fn sin(x: f32) -> f32 {
    let tau = core::f32::consts::TAU;
    let mut x = x - tau * crate::floorf(x / tau);
    if x > core::f32::consts::PI {
        x -= tau;
    }
    // Taylor do x^11 na [-pi, pi].
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    for k in 1..8 {
        term *= -x2 / ((2 * k) as f32 * (2 * k + 1) as f32);
        sum += term;
    }
    sum
}

fn cos(x: f32) -> f32 {
    sin(x + core::f32::consts::FRAC_PI_2)
}

fn atan(x: f32) -> f32 {
    // atan pro |x| <= 1 radou s redukci argumentu.
    if x.abs() > 1.0 {
        let s = if x > 0.0 { 1.0 } else { -1.0 };
        return s * core::f32::consts::FRAC_PI_2 - atan(1.0 / x);
    }
    // Redukce: atan(x) = 2 atan(x / (1 + sqrt(1 + x^2))).
    let y = x / (1.0 + crate::sqrtf(1.0 + x * x));
    let y2 = y * y;
    let mut term = y;
    let mut sum = y;
    for k in 1..12 {
        term *= -y2;
        sum += term / (2 * k + 1) as f32;
    }
    2.0 * sum
}

fn atan2(y: f32, x: f32) -> f32 {
    use core::f32::consts::PI;
    if x > 0.0 {
        atan(y / x)
    } else if x < 0.0 {
        if y >= 0.0 { atan(y / x) + PI } else { atan(y / x) - PI }
    } else if y > 0.0 {
        PI / 2.0
    } else if y < 0.0 {
        -PI / 2.0
    } else {
        0.0
    }
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn trig() {
        for i in -20..20 {
            let x = i as f32 * 0.37;
            assert!((sin(x) - (x as f64).sin() as f32).abs() < 1e-4, "{x}");
            assert!((cos(x) - (x as f64).cos() as f32).abs() < 1e-4, "{x}");
            let y = i as f32 * 0.21 - 1.0;
            assert!((atan2(y, x) - (y as f64).atan2(x as f64) as f32).abs() < 1e-4, "{y} {x}");
        }
    }
}
