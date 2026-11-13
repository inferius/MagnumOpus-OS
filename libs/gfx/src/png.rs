//! Zapis a cteni PNG (RGB 8 bitu, komprese deflate z [`crate::deflate`]).
//! Slouzi testum (referencni snimky vzhledu v `libs/ui`) a snimkum
//! obrazovky pro kontrolu okem.

use alloc::vec;
use alloc::vec::Vec;

fn crc32(d: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

/// PNG (RGB, 8 bitu) z pixelu 0xAARRGGBB (`stride` v pixelech; alfa se
/// zahodi).
pub fn encode(px: &[u32], w: u32, h: u32, stride: u32) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let p = px.get((y * stride + x) as usize).copied().unwrap_or(0);
            raw.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
    }
    let mut z = vec![0x78, 0x01];
    z.extend(crate::deflate::compress(&raw));
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&(b << 16 | a).to_be_bytes());
    let mut png = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let mut chunk = |ty: &[u8], data: &[u8]| {
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(data);
        png.extend_from_slice(&c);
        png.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    png
}

/// Precte PNG (barevne typy RGB a RGBA, 8 bitu, bez prokladani) do pixelu
/// 0xFFRRGGBB: (sirka, vyska, pixely). `None` = jiny nebo vadny soubor.
pub fn decode(png: &[u8]) -> Option<(u32, u32, Vec<u32>)> {
    if png.get(..8)? != [0x89, b'P', b'N', b'G', 13, 10, 26, 10] {
        return None;
    }
    let (mut w, mut h, mut bpp) = (0u32, 0u32, 0usize);
    let mut idat = Vec::new();
    let mut p = 8;
    while p + 8 <= png.len() {
        let len = u32::from_be_bytes(png[p..p + 4].try_into().ok()?) as usize;
        let ty = &png[p + 4..p + 8];
        let data = png.get(p + 8..p + 8 + len)?;
        match ty {
            b"IHDR" => {
                w = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?);
                h = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?);
                bpp = match (data.get(8)?, data.get(9)?, data.get(12)?) {
                    (8, 2, 0) => 3,
                    (8, 6, 0) => 4,
                    _ => return None,
                };
            }
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        p += 12 + len;
    }
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return None;
    }
    let row = w as usize * bpp;
    let raw = crate::deflate::decompress(idat.get(2..)?, (row + 1) * h as usize)?;
    if raw.len() < (row + 1) * h as usize {
        return None;
    }
    let mut cur = vec![0u8; row];
    let mut prev = vec![0u8; row];
    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        let f = raw[y * (row + 1)];
        let src = &raw[y * (row + 1) + 1..(y + 1) * (row + 1)];
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
            let pred = match f {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                4 => {
                    let pp = a + b - c;
                    let (pa, pb, pc) = ((pp - a).abs(), (pp - b).abs(), (pp - c).abs());
                    if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
                }
                _ => return None,
            };
            cur[i] = src[i].wrapping_add(pred as u8);
        }
        for x in 0..w as usize {
            let q = &cur[x * bpp..x * bpp + 3];
            out.push(0xFF00_0000 | (q[0] as u32) << 16 | (q[1] as u32) << 8 | q[2] as u32);
        }
        core::mem::swap(&mut cur, &mut prev);
    }
    Some((w, h, out))
}
