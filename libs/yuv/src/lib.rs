//! Prevod YUV 4:2:0 (NV12, I420) na BGRA pro kompozitor.
//!
//! Vzorce: BT.709, omezeny rozsah (Y 16..235, U a V 16..240), jak ho maji
//! dekodery videa HD. Celociselne s 10 bity zlomku ([`coef`]), aby CPU
//! (tady) a GPU (shader `spirv::yuv`) daly bit po bitu stejne pixely:
//!
//! ```text
//! Yc = Y - 16, Uc = U - 128, Vc = V - 128
//! R = (1192 Yc + 1836 Vc + 512) >> 10
//! G = (1192 Yc - 218 Uc - 546 Vc + 512) >> 10
//! B = (1192 Yc + 2163 Uc + 512) >> 10
//! ```
//!
//! (orezano na 0..255; koeficienty = BT.709 x 1024, zaokrouhlene). Vystup
//! je `0xFFRRGGBB` (B G R A v pameti, jako buffery oken).
//!
//! SIMD pres `core::simd` (8 pixelu naraz; SSE na x86-64-v2, NEON na
//! aarch64); okraje a liche sirky skalarne. Skalarni [`pixel`] je reference
//! pro testy. Funkce nikdy necte mimo predane rezy: rozmery a kroky se
//! overi predem ([`Error`]).

#![no_std]
#![feature(portable_simd)]

#[cfg(test)]
mod tests;

use core::simd::num::{SimdInt, SimdUint};
use core::simd::cmp::SimdOrd;
use core::simd::{Simd, simd_swizzle};

/// Koeficienty BT.709 (omezeny rozsah) x 1024.
pub mod coef {
    pub const Y: i32 = 1192;
    pub const RV: i32 = 1836;
    pub const GU: i32 = 218;
    pub const GV: i32 = 546;
    pub const BU: i32 = 2163;
    /// Zaokrouhleni pred posunem o 10 bitu.
    pub const ROUND: i32 = 512;
}

/// Rozmery nebo kroky neodpovidaji predanym rezum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Nulova sirka nebo vyska.
    Size,
    /// Rovina nebo vystup je kratsi, nez rozmery a krok vyzaduji.
    Short,
}

/// Jeden pixel (reference a okraje).
#[inline]
pub fn pixel(y: u8, u: u8, v: u8) -> u32 {
    let yc = (y as i32 - 16) * coef::Y + coef::ROUND;
    let (uc, vc) = (u as i32 - 128, v as i32 - 128);
    let r = clamp((yc + coef::RV * vc) >> 10);
    let g = clamp((yc - coef::GU * uc - coef::GV * vc) >> 10);
    let b = clamp((yc + coef::BU * uc) >> 10);
    0xFF00_0000 | r << 16 | g << 8 | b
}

#[inline]
fn clamp(v: i32) -> u32 {
    v.clamp(0, 255) as u32
}

type I32x8 = Simd<i32, 8>;

/// 8 pixelu radku: `y` 8 jasu, `u` a `v` 4 hodnoty barvy (kazda pro dva
/// sousedni pixely).
#[inline]
fn eight(y: [u8; 8], u: [u8; 4], v: [u8; 4], out: &mut [u32]) {
    let y = Simd::from_array(y).cast::<i32>();
    let u4 = Simd::from_array(u);
    let v4 = Simd::from_array(v);
    let u: Simd<u8, 8> = simd_swizzle!(u4, [0, 0, 1, 1, 2, 2, 3, 3]);
    let v: Simd<u8, 8> = simd_swizzle!(v4, [0, 0, 1, 1, 2, 2, 3, 3]);
    let uc = u.cast::<i32>() - I32x8::splat(128);
    let vc = v.cast::<i32>() - I32x8::splat(128);
    let yc = (y - I32x8::splat(16)) * I32x8::splat(coef::Y) + I32x8::splat(coef::ROUND);
    let lo = I32x8::splat(0);
    let hi = I32x8::splat(255);
    let r = ((yc + I32x8::splat(coef::RV) * vc) >> I32x8::splat(10)).simd_clamp(lo, hi);
    let g = ((yc - I32x8::splat(coef::GU) * uc - I32x8::splat(coef::GV) * vc) >> I32x8::splat(10)).simd_clamp(lo, hi);
    let b = ((yc + I32x8::splat(coef::BU) * uc) >> I32x8::splat(10)).simd_clamp(lo, hi);
    let px = I32x8::splat(0xFF00_0000u32 as i32) | (r << I32x8::splat(16)) | (g << I32x8::splat(8)) | b;
    out[..8].copy_from_slice(&px.cast::<u32>().to_array());
}

/// Overi, ze rovina `pitch` x `rows` s radkem `row` bajtu je v `len` bajtech.
fn fits(len: usize, pitch: usize, rows: usize, row: usize) -> bool {
    pitch >= row && pitch.checked_mul(rows - 1).and_then(|n| n.checked_add(row)).is_some_and(|n| n <= len)
}

/// NV12 (rovina Y, prokladana rovina UV) -> BGRA `w` x `h` do `out`
/// s krokem `out_stride` pixelu.
#[allow(clippy::too_many_arguments)]
pub fn nv12_to_bgra(
    y: &[u8],
    y_pitch: usize,
    uv: &[u8],
    uv_pitch: usize,
    w: usize,
    h: usize,
    out: &mut [u32],
    out_stride: usize,
) -> Result<(), Error> {
    if w == 0 || h == 0 {
        return Err(Error::Size);
    }
    let cw = w.div_ceil(2);
    if !fits(y.len(), y_pitch, h, w) || !fits(uv.len(), uv_pitch, h.div_ceil(2), cw * 2) || !fits(out.len(), out_stride, h, w) {
        return Err(Error::Short);
    }
    for row in 0..h {
        let yr = &y[row * y_pitch..row * y_pitch + w];
        let cr = &uv[(row / 2) * uv_pitch..(row / 2) * uv_pitch + cw * 2];
        let o = &mut out[row * out_stride..row * out_stride + w];
        let mut x = 0;
        while x + 8 <= w {
            let c = &cr[x..x + 8];
            eight(yr[x..x + 8].try_into().unwrap(), [c[0], c[2], c[4], c[6]], [c[1], c[3], c[5], c[7]], &mut o[x..]);
            x += 8;
        }
        while x < w {
            o[x] = pixel(yr[x], cr[(x / 2) * 2], cr[(x / 2) * 2 + 1]);
            x += 1;
        }
    }
    Ok(())
}

/// I420 (roviny Y, U, V) -> BGRA `w` x `h` do `out` s krokem `out_stride`
/// pixelu.
#[allow(clippy::too_many_arguments)]
pub fn i420_to_bgra(
    y: &[u8],
    y_pitch: usize,
    u: &[u8],
    u_pitch: usize,
    v: &[u8],
    v_pitch: usize,
    w: usize,
    h: usize,
    out: &mut [u32],
    out_stride: usize,
) -> Result<(), Error> {
    if w == 0 || h == 0 {
        return Err(Error::Size);
    }
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    if !fits(y.len(), y_pitch, h, w)
        || !fits(u.len(), u_pitch, ch, cw)
        || !fits(v.len(), v_pitch, ch, cw)
        || !fits(out.len(), out_stride, h, w)
    {
        return Err(Error::Short);
    }
    for row in 0..h {
        let yr = &y[row * y_pitch..row * y_pitch + w];
        let ur = &u[(row / 2) * u_pitch..(row / 2) * u_pitch + cw];
        let vr = &v[(row / 2) * v_pitch..(row / 2) * v_pitch + cw];
        let o = &mut out[row * out_stride..row * out_stride + w];
        let mut x = 0;
        while x + 8 <= w {
            let c = x / 2;
            eight(yr[x..x + 8].try_into().unwrap(), ur[c..c + 4].try_into().unwrap(), vr[c..c + 4].try_into().unwrap(), &mut o[x..]);
            x += 8;
        }
        while x < w {
            o[x] = pixel(yr[x], ur[x / 2], vr[x / 2]);
            x += 1;
        }
    }
    Ok(())
}

/// Vzorek NV12 nebo I420 (`y`, `u`, `v`) pro pixel z barvy RGB: obracene
/// vzorce BT.709 omezeneho rozsahu (testy a testovaci producent videa).
pub fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    // BT.709 x 1024 (omezeny rozsah: Y 219/255, U a V 224/255).
    let y = (186 * r + 627 * g + 63 * b + 512) / 1024 + 16;
    let u = (-103 * r - 346 * g + 449 * b + 512).div_euclid(1024) + 128;
    let v = (449 * r - 408 * g - 41 * b + 512).div_euclid(1024) + 128;
    (y.clamp(16, 235) as u8, u.clamp(16, 240) as u8, v.clamp(16, 240) as u8)
}
