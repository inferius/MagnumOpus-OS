//! Matematika f32 bez `std` (programy se prekladaji bez FPU knihoven).

pub fn floor(x: f32) -> f32 {
    if !x.is_finite() || x.abs() >= 8_388_608.0 {
        return x;
    }
    let i = x as i32 as f32;
    if i > x { i - 1.0 } else { i }
}

pub fn ceil(x: f32) -> f32 {
    let f = floor(x);
    if f < x { f + 1.0 } else { f }
}

/// Odmocnina (Newtonova metoda z odhadu podle exponentu).
pub fn sqrt(x: f32) -> f32 {
    if !(x > 0.0) || !x.is_finite() {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC0_0000);
    for _ in 0..4 {
        y = 0.5 * (y + x / y);
    }
    y
}
