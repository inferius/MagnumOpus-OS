//! Obrysy glyfu z tabulky `glyf`: jednoduche glyfy (kontury z bodu na
//! krivce a mimo ni) a slozene (komponenty s posunem a skalovanim).

use alloc::vec::Vec;

use crate::tables::{Data, Tables};
use crate::{Point, Segment};

/// Nejvic vnoreni slozenych glyfu a bodu jednoho glyfu.
const MAX_DEPTH: u32 = 8;
const MAX_POINTS: usize = 8192;
const MAX_SEGMENTS: usize = 32768;

// Priznaky bodu.
const ON_CURVE: u8 = 0x01;
const X_SHORT: u8 = 0x02;
const Y_SHORT: u8 = 0x04;
const REPEAT: u8 = 0x08;
const X_SAME: u8 = 0x10;
const Y_SAME: u8 = 0x20;

// Priznaky komponenty.
const ARG_WORDS: u16 = 0x0001;
const ARGS_XY: u16 = 0x0002;
const HAVE_SCALE: u16 = 0x0008;
const MORE: u16 = 0x0020;
const HAVE_XY_SCALE: u16 = 0x0040;
const HAVE_2X2: u16 = 0x0080;

/// Afinni transformace komponenty: (a, b, c, d, dx, dy).
#[derive(Clone, Copy)]
struct Xform([f32; 6]);

impl Xform {
    const ID: Xform = Xform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    fn apply(&self, p: Point) -> Point {
        let m = &self.0;
        Point {
            x: m[0] * p.x + m[2] * p.y + m[4],
            y: m[1] * p.x + m[3] * p.y + m[5],
        }
    }

    /// Nejdriv `inner`, pak `self`.
    fn then(&self, inner: &Xform) -> Xform {
        let (a, b) = (&self.0, &inner.0);
        Xform([
            a[0] * b[0] + a[2] * b[1],
            a[1] * b[0] + a[3] * b[1],
            a[0] * b[2] + a[2] * b[3],
            a[1] * b[2] + a[3] * b[3],
            a[0] * b[4] + a[2] * b[5] + a[4],
            a[1] * b[4] + a[3] * b[5] + a[5],
        ])
    }
}

/// Obrys glyfu `gid` jako useky (prazdny pri jakekoli chybe dat).
pub fn glyph(t: &Tables<'_>, gid: u16) -> Vec<Segment> {
    let mut out = Vec::new();
    if add(t, gid, &Xform::ID, 0, &mut out).is_none() {
        out.clear();
    }
    out
}

fn add(t: &Tables<'_>, gid: u16, x: &Xform, depth: u32, out: &mut Vec<Segment>) -> Option<()> {
    if depth > MAX_DEPTH {
        return None;
    }
    let g = t.glyph_data(gid)?;
    if g.len() == 0 {
        return Some(());
    }
    let contours = g.i16(0)?;
    if contours >= 0 {
        simple(g, contours as usize, x, out)
    } else {
        composite(t, g, x, depth, out)
    }
}

fn simple(g: Data<'_>, contours: usize, x: &Xform, out: &mut Vec<Segment>) -> Option<()> {
    let mut ends = Vec::with_capacity(contours);
    let mut prev: i32 = -1;
    for i in 0..contours {
        let e = g.u16(10 + i * 2)? as i32;
        if e <= prev && i > 0 {
            return None;
        }
        prev = e;
        ends.push(e as usize);
    }
    let n = ends.last().map_or(0, |&e| e + 1);
    if n > MAX_POINTS {
        return None;
    }
    let ins_len = g.u16(10 + contours * 2)? as usize;
    let mut pos = 12 + contours * 2 + ins_len;
    // Priznaky (s opakovanim).
    let mut flags = Vec::with_capacity(n);
    while flags.len() < n {
        let f = g.u8(pos)?;
        pos += 1;
        flags.push(f);
        if f & REPEAT != 0 {
            let r = g.u8(pos)?;
            pos += 1;
            for _ in 0..r {
                if flags.len() < n {
                    flags.push(f);
                }
            }
        }
    }
    // Souradnice X, pak Y (delta).
    let mut xs = Vec::with_capacity(n);
    let mut v: i32 = 0;
    for &f in &flags {
        if f & X_SHORT != 0 {
            let d = g.u8(pos)? as i32;
            pos += 1;
            v += if f & X_SAME != 0 { d } else { -d };
        } else if f & X_SAME == 0 {
            v += g.i16(pos)? as i32;
            pos += 2;
        }
        xs.push(v);
    }
    let mut ys = Vec::with_capacity(n);
    v = 0;
    for &f in &flags {
        if f & Y_SHORT != 0 {
            let d = g.u8(pos)? as i32;
            pos += 1;
            v += if f & Y_SAME != 0 { d } else { -d };
        } else if f & Y_SAME == 0 {
            v += g.i16(pos)? as i32;
            pos += 2;
        }
        ys.push(v);
    }
    let mut start = 0;
    for &end in &ends {
        if end >= n || end < start {
            return None;
        }
        let pts: Vec<(Point, bool)> = (start..=end)
            .map(|i| {
                (
                    x.apply(Point {
                        x: xs[i] as f32,
                        y: ys[i] as f32,
                    }),
                    flags[i] & ON_CURVE != 0,
                )
            })
            .collect();
        contour(&pts, out)?;
        start = end + 1;
    }
    Some(())
}

fn mid(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}

/// Kontura z bodu na krivce a mimo ni -> useky (mezi dvema body mimo
/// krivku lezi implicitni bod na krivce v pulce).
fn contour(pts: &[(Point, bool)], out: &mut Vec<Segment>) -> Option<()> {
    let n = pts.len();
    if n < 2 {
        return Some(());
    }
    // Zacatek: prvni bod na krivce, jinak pulka prvnich dvou.
    let first_on = pts.iter().position(|p| p.1);
    let (start, begin) = match first_on {
        Some(i) => (pts[i].0, i),
        None => (mid(pts[0].0, pts[1].0), 0),
    };
    let mut cur = start;
    let mut ctrl: Option<Point> = None;
    for k in 1..=n {
        let (p, on) = pts[(begin + k) % n];
        if first_on.is_none() && k == n {
            // Kontura jen z bodu mimo krivku: uzavrit k zacatku.
            break;
        }
        match (on, ctrl) {
            (true, None) => {
                out.push(Segment::Line(cur, p));
                cur = p;
            }
            (true, Some(c)) => {
                out.push(Segment::Quad(cur, c, p));
                cur = p;
                ctrl = None;
            }
            (false, None) => ctrl = Some(p),
            (false, Some(c)) => {
                let m = mid(c, p);
                out.push(Segment::Quad(cur, c, m));
                cur = m;
                ctrl = Some(p);
            }
        }
        if out.len() > MAX_SEGMENTS {
            return None;
        }
    }
    match ctrl {
        Some(c) => out.push(Segment::Quad(cur, c, start)),
        None if cur != start => out.push(Segment::Line(cur, start)),
        None => {}
    }
    Some(())
}

fn f2dot14(v: i16) -> f32 {
    v as f32 / 16384.0
}

fn composite(t: &Tables<'_>, g: Data<'_>, parent: &Xform, depth: u32, out: &mut Vec<Segment>) -> Option<()> {
    let mut pos = 10;
    loop {
        let flags = g.u16(pos)?;
        let gid = g.u16(pos + 2)?;
        pos += 4;
        let (dx, dy) = if flags & ARG_WORDS != 0 {
            let v = (g.i16(pos)?, g.i16(pos + 2)?);
            pos += 4;
            (v.0 as f32, v.1 as f32)
        } else {
            let v = (g.u8(pos)? as i8, g.u8(pos + 1)? as i8);
            pos += 2;
            (v.0 as f32, v.1 as f32)
        };
        // Body k sobe (ARGS_XY = 0) zatim neumime: bez posunu.
        let (dx, dy) = if flags & ARGS_XY != 0 { (dx, dy) } else { (0.0, 0.0) };
        let mut m = [1.0, 0.0, 0.0, 1.0, dx, dy];
        if flags & HAVE_SCALE != 0 {
            let s = f2dot14(g.i16(pos)?);
            pos += 2;
            m[0] = s;
            m[3] = s;
        } else if flags & HAVE_XY_SCALE != 0 {
            m[0] = f2dot14(g.i16(pos)?);
            m[3] = f2dot14(g.i16(pos + 2)?);
            pos += 4;
        } else if flags & HAVE_2X2 != 0 {
            m[0] = f2dot14(g.i16(pos)?);
            m[1] = f2dot14(g.i16(pos + 2)?);
            m[2] = f2dot14(g.i16(pos + 4)?);
            m[3] = f2dot14(g.i16(pos + 6)?);
            pos += 8;
        }
        let x = parent.then(&Xform(m));
        add(t, gid, &x, depth + 1, out)?;
        if out.len() > MAX_SEGMENTS {
            return None;
        }
        if flags & MORE == 0 {
            return Some(());
        }
    }
}
