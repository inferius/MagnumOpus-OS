//! Pomocna geometrie a barvy bez `std`.

use core::f32::consts::{PI, TAU};

/// Obdelnik v pixelech obrazovky.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// Ctverec se stredem `(cx, cy)` a "polomerem" `r` (zaokrouhleno ven).
    pub fn around(cx: f32, cy: f32, r: f32) -> Rect {
        let x0 = floor(cx - r) as i32;
        let y0 = floor(cy - r) as i32;
        let x1 = floor(cx + r) as i32 + 1;
        let y1 = floor(cy + r) as i32 + 1;
        Rect { x: x0, y: y0, w: (x1 - x0).max(0) as u32, h: (y1 - y0).max(0) as u32 }
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    pub fn right(&self) -> i32 {
        self.x.saturating_add(self.w as i32)
    }

    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h as i32)
    }

    pub fn intersect(&self, o: Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 {
            return Rect::default();
        }
        Rect { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 }
    }

    /// Orez na obrazovku `w x h`.
    pub fn clip(&self, w: u32, h: u32) -> Rect {
        self.intersect(Rect { x: 0, y: 0, w, h })
    }

    /// Nejmensi obdelnik s obema (prazdny se ignoruje).
    pub fn union(&self, o: Rect) -> Rect {
        if self.is_empty() {
            return o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        Rect { x: x0, y: y0, w: (self.right().max(o.right()) - x0) as u32, h: (self.bottom().max(o.bottom()) - y0) as u32 }
    }

    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }
}

pub fn floor(x: f32) -> f32 {
    gfx::floorf(x)
}

pub fn round(x: f32) -> f32 {
    floor(x + 0.5)
}

/// Odmocnina: odhad z exponentu a dva kroky Newtonovy metody (relativni
/// chyba pod 1e-6 - na pixely staci, `gfx::sqrtf` dela ctyri kroky).
pub fn sqrt(x: f32) -> f32 {
    if !(x > 0.0) || !x.is_finite() {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FBB_4F2E);
    y = 0.5 * (y + x / y);
    0.5 * (y + x / y)
}

pub fn sin(x: f32) -> f32 {
    let mut x = x - TAU * floor(x / TAU);
    if x > PI {
        x -= TAU;
    }
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    for k in 1..8 {
        term *= -x2 / ((2 * k) as f32 * (2 * k + 1) as f32);
        sum += term;
    }
    sum
}

pub fn cos(x: f32) -> f32 {
    sin(x + core::f32::consts::FRAC_PI_2)
}

/// atan pro |x| <= 1 (polynom, chyba pod 1e-5 rad).
fn atan_unit(x: f32) -> f32 {
    let y = x / (1.0 + sqrt(1.0 + x * x));
    let y2 = y * y;
    let mut term = y;
    let mut sum = y;
    for k in 1..10 {
        term *= -y2;
        sum += term / (2 * k + 1) as f32;
    }
    2.0 * sum
}

fn atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    if x.abs() >= y.abs() {
        let a = atan_unit(y / x);
        if x > 0.0 {
            a
        } else if y >= 0.0 {
            a + PI
        } else {
            a - PI
        }
    } else {
        let a = atan_unit(x / y);
        if y > 0.0 { PI / 2.0 - a } else { -PI / 2.0 - a }
    }
}

/// Uhel bodu `(dx, dy)` (y dolu) od vrcholu po smeru hodin, 0..2pi.
pub fn clockwise_angle(dx: f32, dy: f32) -> f32 {
    let a = atan2(dx, -dy);
    if a < 0.0 { a + TAU } else { a }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` v case `t` (puleni intervalu).
pub fn bezier_ease(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let bez = |u: f32, a: f32, b: f32| {
        let m = 1.0 - u;
        3.0 * m * m * u * a + 3.0 * m * u * u * b + u * u * u
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = 0.5 * (lo + hi);
        if bez(mid, x1, x2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(0.5 * (lo + hi), y1, y2)
}

/// `0x..RRGGBB` -> [r, g, b] jako f32 0..255.
pub fn rgbf(c: u32) -> [f32; 3] {
    [(c >> 16 & 0xFF) as f32, (c >> 8 & 0xFF) as f32, (c & 0xFF) as f32]
}

/// `a` prelnute k `b` s krytim `t`.
pub fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

pub fn pack(c: [f32; 3]) -> u32 {
    let ch = |v: f32| (v + 0.5).clamp(0.0, 255.0) as u32;
    0xFF00_0000 | ch(c[0]) << 16 | ch(c[1]) << 8 | ch(c[2])
}

/// Ztlumeni barvy k cerne: `k` 0..=256 (256 = beze zmeny).
pub fn dim(c: u32, k: u32) -> u32 {
    let rb = ((c & 0x00FF_00FF) * k >> 8) & 0x00FF_00FF;
    let g = ((c & 0x0000_FF00) * k >> 8) & 0x0000_FF00;
    0xFF00_0000 | rb | g
}

/// Prolnuti `a` a `b`: `kb` 0..=256 je podil `b`.
pub fn lerp(a: u32, b: u32, kb: u32) -> u32 {
    let ka = 256 - kb;
    let rb = (((a & 0x00FF_00FF) * ka + (b & 0x00FF_00FF) * kb) >> 8) & 0x00FF_00FF;
    let g = (((a & 0x0000_FF00) * ka + (b & 0x0000_FF00) * kb) >> 8) & 0x0000_FF00;
    0xFF00_0000 | rb | g
}

/// Prohozeni R a B pro framebuffer RGB.
pub fn swap(c: u32, rgb: bool) -> u32 {
    if rgb { (c & 0xFF00_FF00) | (c >> 16 & 0xFF) | (c & 0xFF) << 16 } else { c }
}

pub fn unswap(c: u32, rgb: bool) -> u32 {
    swap(c, rgb)
}
