//! Barvy z tokenu: `#rrggbb`, `#rrggbbaa`, `rgba(r, g, b, a)`; prevod na
//! premultiplied `0xAARRGGBB` a kontrast podle WCAG 2.2.

/// Precte barvu a vrati ji premultiplied.
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return match hex.len() {
            6 => Some(0xFF00_0000 | v),
            8 => Some(to_premul(v >> 24 & 0xFF, v >> 16 & 0xFF, v >> 8 & 0xFF, v & 0xFF)),
            _ => None,
        };
    }
    let inner = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb("))?.strip_suffix(')')?;
    let mut it = inner.split(',').map(str::trim);
    let mut ch = [0u32; 3];
    for c in ch.iter_mut() {
        let v: u32 = it.next()?.parse().ok()?;
        if v > 255 {
            return None;
        }
        *c = v;
    }
    let a = match it.next() {
        Some(a) => {
            let a: f32 = a.parse().ok()?;
            if !(0.0..=1.0).contains(&a) {
                return None;
            }
            (a * 255.0 + 0.5) as u32
        }
        None => 255,
    };
    if it.next().is_some() {
        return None;
    }
    Some(to_premul(ch[0], ch[1], ch[2], a))
}

/// Premultiplied `0xAARRGGBB` ze slozek 0..=255.
pub fn to_premul(r: u32, g: u32, b: u32, a: u32) -> u32 {
    let m = |c: u32| (c * a + 127) / 255;
    a << 24 | m(r) << 16 | m(g) << 8 | m(b)
}

/// Slozky bez premultiplikace (r, g, b, a).
pub fn unpremul(c: u32) -> (u32, u32, u32, u32) {
    let a = c >> 24;
    if a == 0 {
        return (0, 0, 0, 0);
    }
    let d = |v: u32| ((v * 255 + a / 2) / a).min(255);
    (d(c >> 16 & 0xFF), d(c >> 8 & 0xFF), d(c & 0xFF), a)
}

fn channel(v: u32) -> f32 {
    let c = v as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { pow((c + 0.055) / 1.055, 2.4) }
}

fn luminance(c: u32) -> f32 {
    let (r, g, b, _) = unpremul(c);
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// Pomer kontrastu dvou nepruhlednych barev (1 az 21).
pub fn contrast_ratio(a: u32, b: u32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `x^y` pro x v (0, 1] bez `std` (exp a ln radami; staci na kontrast).
fn pow(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    exp(y * ln(x))
}

fn ln(x: f32) -> f32 {
    // x = m * 2^e, m v [1, 2); ln m radou atanh.
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xFF) as i32 - 127;
    let m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    let t = (m - 1.0) / (m + 1.0);
    let t2 = t * t;
    let mut sum = 0.0;
    let mut term = t;
    let mut k = 1.0;
    for _ in 0..12 {
        sum += term / k;
        term *= t2;
        k += 2.0;
    }
    2.0 * sum + e as f32 * core::f32::consts::LN_2
}

fn exp(x: f32) -> f32 {
    // e^x = 2^k * e^r, |r| <= ln2/2.
    let k = (x / core::f32::consts::LN_2 + if x < 0.0 { -0.5 } else { 0.5 }) as i32;
    let r = x - k as f32 * core::f32::consts::LN_2;
    let mut sum = 1.0;
    let mut term = 1.0;
    for i in 1..12 {
        term *= r / i as f32;
        sum += term;
    }
    if !(-126..=127).contains(&k) {
        return if k < 0 { 0.0 } else { f32::MAX };
    }
    sum * f32::from_bits(((k + 127) as u32) << 23)
}

#[cfg(test)]
mod t {
    use super::*;

    #[test]
    fn parse() {
        assert_eq!(parse_color("#6b3fd6"), Some(0xFF6B_3FD6));
        assert_eq!(parse_color("rgba(0, 0, 0, 0.5)"), Some(0x8000_0000));
        assert_eq!(parse_color("rgba(255,255,255,0.5)"), Some(0x8080_8080));
        assert_eq!(parse_color("rgba(1,2,3,0.5,1)"), None);
        assert_eq!(parse_color("rgb(1,2,3)"), Some(0xFF01_0203));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("rgba(300,0,0,1)"), None);
        assert_eq!(parse_color("rgba(0,0,0,2)"), None);
    }

    #[test]
    fn ratio() {
        assert!((contrast_ratio(0xFF00_0000, 0xFFFF_FFFF) - 21.0).abs() < 0.05);
        assert!((contrast_ratio(0xFF77_7777, 0xFFFF_FFFF) - 4.48).abs() < 0.05);
        for x in [0.01f32, 0.2, 0.5, 0.99] {
            let p = pow(x, 2.4);
            assert!((p - libm_pow(x)).abs() < 1e-4, "{x}");
        }
    }

    fn libm_pow(x: f32) -> f32 {
        (x as f64).powf(2.4) as f32
    }
}
