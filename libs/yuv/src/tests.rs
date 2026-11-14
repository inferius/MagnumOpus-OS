//! Testy prevodu YUV na hostiteli: `cargo test -p yuv`.

extern crate std;

use std::vec;
use std::vec::Vec;

use crate::{Error, i420_to_bgra, nv12_to_bgra, pixel, rgb_to_yuv};

#[test]
fn reference_colors() {
    // Cerna, bila a sede: Y 16 / 235, barva 128.
    assert_eq!(pixel(16, 128, 128), 0xFF00_0000);
    assert_eq!(pixel(235, 128, 128), 0xFFFF_FFFF);
    // Mimo rozsah se orizne, nepretece.
    assert_eq!(pixel(0, 128, 128), 0xFF00_0000);
    assert_eq!(pixel(255, 128, 128), 0xFFFF_FFFF);
    // Ciste barvy BT.709 tam a zpet (tolerance zaokrouhleni 2).
    for (r, g, b) in [(255u8, 0u8, 0u8), (0, 255, 0), (0, 0, 255), (200, 120, 40), (10, 200, 230)] {
        let (y, u, v) = rgb_to_yuv(r, g, b);
        let p = pixel(y, u, v);
        let got = [(p >> 16) as u8, (p >> 8) as u8, p as u8];
        for (a, e) in got.iter().zip([r, g, b]) {
            assert!(a.abs_diff(e) <= 2, "{r},{g},{b} -> {y},{u},{v} -> {got:?}");
        }
    }
}

/// Pseudonahodne bajty (xorshift).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[test]
fn simd_matches_scalar_nv12() {
    // Sirky s okrajem pro skalarni dobeh (17, 33) i bez (16).
    for (w, h) in [(16usize, 4usize), (17, 3), (33, 5), (1, 1), (64, 2)] {
        let (yp, cp) = (w + 3, w.div_ceil(2) * 2 + 1);
        let y = noise(yp * h, 1 + w as u64);
        let uv = noise(cp * h.div_ceil(2), 7 + h as u64);
        let mut out = vec![0u32; (w + 2) * h];
        nv12_to_bgra(&y, yp, &uv, cp, w, h, &mut out, w + 2).unwrap();
        for r in 0..h {
            for x in 0..w {
                let c = (r / 2) * cp + (x / 2) * 2;
                assert_eq!(out[r * (w + 2) + x], pixel(y[r * yp + x], uv[c], uv[c + 1]), "{w}x{h} [{x},{r}]");
            }
            // Vypln za sirkou vystupu se nezmeni.
            assert_eq!(out[r * (w + 2) + w], 0);
        }
    }
}

#[test]
fn simd_matches_scalar_i420() {
    for (w, h) in [(16usize, 4usize), (19, 3), (40, 7)] {
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let y = noise(w * h, 3);
        let u = noise(cw * ch, 5);
        let v = noise(cw * ch, 9);
        let mut out = vec![0u32; w * h];
        i420_to_bgra(&y, w, &u, cw, &v, cw, w, h, &mut out, w).unwrap();
        for r in 0..h {
            for x in 0..w {
                let c = (r / 2) * cw + x / 2;
                assert_eq!(out[r * w + x], pixel(y[r * w + x], u[c], v[c]), "{w}x{h} [{x},{r}]");
            }
        }
    }
}

#[test]
fn short_planes_are_rejected_not_read() {
    let y = vec![0u8; 16 * 4];
    let uv = vec![128u8; 16 * 2];
    let mut out = vec![0u32; 16 * 4];
    assert_eq!(nv12_to_bgra(&y, 16, &uv, 16, 16, 4, &mut out, 16), Ok(()));
    assert_eq!(nv12_to_bgra(&y[..63], 16, &uv, 16, 16, 4, &mut out, 16), Err(Error::Short));
    assert_eq!(nv12_to_bgra(&y, 16, &uv[..31], 16, 16, 4, &mut out, 16), Err(Error::Short));
    assert_eq!(nv12_to_bgra(&y, 16, &uv, 16, 16, 4, &mut out[..63], 16), Err(Error::Short));
    assert_eq!(nv12_to_bgra(&y, 15, &uv, 16, 16, 4, &mut out, 16), Err(Error::Short), "krok < sirka");
    assert_eq!(nv12_to_bgra(&y, 16, &uv, 16, 0, 4, &mut out, 16), Err(Error::Size));
    let u = vec![128u8; 8 * 2];
    assert_eq!(i420_to_bgra(&y, 16, &u, 8, &u[..15], 8, 16, 4, &mut out, 16), Err(Error::Short));
}
