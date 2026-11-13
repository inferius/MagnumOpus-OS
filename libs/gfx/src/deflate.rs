//! Deflate (RFC 1951) pro PNG: kompresor s pevnym Huffmanovym kodem
//! a LZ77 (okno 32 KiB, hash trojic) a dekompresor pro bloky bez komprese,
//! s pevnym i dynamickym kodem. Snimky rozhrani (velke jednobarevne
//! plochy) se zmensi desitkykrat; zadna zavislost navic.

use alloc::vec;
use alloc::vec::Vec;

/// Zakladni delky a extra bity kodu delek 257..285.
const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
/// Zakladni vzdalenosti a extra bity kodu 0..29.
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

const WINDOW: usize = 32 * 1024;
const MAX_MATCH: usize = 258;
/// Kolik kandidatu v retezci hashe zkusit (rychlost proti pomeru).
const CHAIN: usize = 48;

struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Huffmanuv kod (zapisuje se od nejvyssiho bitu).
    fn code(&mut self, code: u32, len: u32) {
        let mut r = 0;
        for i in 0..len {
            r |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(r, len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

/// Pevny kod literalu / delky `sym`.
fn lit(b: &mut Bits, sym: u32) {
    match sym {
        0..=143 => b.code(0x30 + sym, 8),
        144..=255 => b.code(0x190 + sym - 144, 9),
        256..=279 => b.code(sym - 256, 7),
        _ => b.code(0xC0 + sym - 280, 8),
    }
}

fn hash(d: &[u8], i: usize) -> usize {
    ((d[i] as usize) << 10 ^ (d[i + 1] as usize) << 5 ^ d[i + 2] as usize) & 0x7FFF
}

/// Komprimuje `data` jednim blokem s pevnym kodem (bez hlavicky zlib).
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut b = Bits { out: Vec::with_capacity(data.len() / 8 + 16), acc: 0, n: 0 };
    b.put(1, 1); // posledni blok
    b.put(1, 2); // pevny kod
    let mut head = vec![usize::MAX; 1 << 15];
    let mut prev = vec![usize::MAX; WINDOW];
    let mut i = 0;
    let insert = |head: &mut [usize], prev: &mut [usize], i: usize| {
        if i + 2 < data.len() {
            let h = hash(data, i);
            prev[i % WINDOW] = head[h];
            head[h] = i;
        }
    };
    while i < data.len() {
        let (mut best, mut dist) = (0, 0);
        if i + 2 < data.len() {
            let mut c = head[hash(data, i)];
            let max = MAX_MATCH.min(data.len() - i);
            for _ in 0..CHAIN {
                if c == usize::MAX || i - c > WINDOW || c >= i {
                    break;
                }
                let mut l = 0;
                while l < max && data[c + l] == data[i + l] {
                    l += 1;
                }
                if l > best {
                    best = l;
                    dist = i - c;
                    if l == max {
                        break;
                    }
                }
                c = prev[c % WINDOW];
            }
        }
        if best >= 3 {
            let li = LEN_BASE.iter().rposition(|&x| x as usize <= best).unwrap();
            lit(&mut b, 257 + li as u32);
            b.put((best - LEN_BASE[li] as usize) as u32, LEN_EXTRA[li] as u32);
            let di = DIST_BASE.iter().rposition(|&x| x as usize <= dist).unwrap();
            b.code(di as u32, 5);
            b.put((dist - DIST_BASE[di] as usize) as u32, DIST_EXTRA[di] as u32);
            for k in i..i + best {
                insert(&mut head, &mut prev, k);
            }
            i += best;
        } else {
            lit(&mut b, data[i] as u32);
            insert(&mut head, &mut prev, i);
            i += 1;
        }
    }
    lit(&mut b, 256);
    b.finish()
}

struct In<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
}

impl In<'_> {
    fn bits(&mut self, n: u32) -> Option<u32> {
        while self.n < n {
            let byte = *self.d.get(self.pos)?;
            self.pos += 1;
            self.acc |= (byte as u64) << self.n;
            self.n += 8;
        }
        let v = (self.acc & ((1u64 << n) - 1)) as u32;
        self.acc >>= n;
        self.n -= n;
        Some(v)
    }
}

/// Kanonicky Huffmanuv kod z delek: (pocty delek, symboly podle kodu).
struct Huff {
    count: [u16; 16],
    sym: Vec<u16>,
}

impl Huff {
    fn new(lens: &[u8]) -> Self {
        let mut count = [0u16; 16];
        for &l in lens {
            count[l as usize] += 1;
        }
        count[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + count[i - 1];
        }
        let mut sym = vec![0u16; lens.len()];
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                sym[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Self { count, sym }
    }

    fn decode(&self, r: &mut In<'_>) -> Option<u16> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= r.bits(1)? as i32;
            let c = self.count[len] as i32;
            if code - c < first {
                return self.sym.get((index + code - first) as usize).copied();
            }
            index += c;
            first += c;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

/// Rozbali data deflate (bez hlavicky zlib); `None` = vadna data.
/// `limit` = nejvic bajtu vystupu (ochrana proti bombe).
pub fn decompress(d: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut r = In { d, pos: 0, acc: 0, n: 0 };
    let mut out = Vec::new();
    loop {
        let last = r.bits(1)?;
        match r.bits(2)? {
            0 => {
                r.acc = 0;
                r.n = 0;
                let len = u16::from_le_bytes([*d.get(r.pos)?, *d.get(r.pos + 1)?]) as usize;
                r.pos += 4;
                out.extend_from_slice(d.get(r.pos..r.pos + len)?);
                r.pos += len;
            }
            t @ (1 | 2) => {
                let (lh, dh) = if t == 1 {
                    let mut l = [8u8; 288];
                    l[144..256].fill(9);
                    l[256..280].fill(7);
                    (Huff::new(&l), Huff::new(&[5u8; 30]))
                } else {
                    let hlit = r.bits(5)? as usize + 257;
                    let hdist = r.bits(5)? as usize + 1;
                    let hclen = r.bits(4)? as usize + 4;
                    const ORD: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &o in &ORD[..hclen] {
                        cl[o] = r.bits(3)? as u8;
                    }
                    let ch = Huff::new(&cl);
                    let mut lens = Vec::with_capacity(hlit + hdist);
                    while lens.len() < hlit + hdist {
                        match ch.decode(&mut r)? {
                            s @ 0..=15 => lens.push(s as u8),
                            16 => {
                                let p = *lens.last()?;
                                for _ in 0..3 + r.bits(2)? {
                                    lens.push(p);
                                }
                            }
                            17 => lens.extend(core::iter::repeat_n(0, 3 + r.bits(3)? as usize)),
                            _ => lens.extend(core::iter::repeat_n(0, 11 + r.bits(7)? as usize)),
                        }
                    }
                    (Huff::new(&lens[..hlit]), Huff::new(&lens[hlit..hlit + hdist]))
                };
                loop {
                    let s = lh.decode(&mut r)? as usize;
                    if s < 256 {
                        out.push(s as u8);
                    } else if s == 256 {
                        break;
                    } else {
                        let li = s - 257;
                        let len = *LEN_BASE.get(li)? as usize + r.bits(*LEN_EXTRA.get(li)? as u32)? as usize;
                        let di = dh.decode(&mut r)? as usize;
                        let dist = *DIST_BASE.get(di)? as usize + r.bits(*DIST_EXTRA.get(di)? as u32)? as usize;
                        if dist > out.len() {
                            return None;
                        }
                        let from = out.len() - dist;
                        for k in 0..len {
                            out.push(out[from + k]);
                        }
                    }
                    if out.len() > limit {
                        return None;
                    }
                }
            }
            _ => return None,
        }
        if out.len() > limit {
            return None;
        }
        if last == 1 {
            return Some(out);
        }
    }
}
