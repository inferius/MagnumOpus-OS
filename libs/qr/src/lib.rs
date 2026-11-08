//! Kodovani QR (ISO/IEC 18004) bez alokace: bajtovy rezim, uroven opravy
//! chyb M (15 %), verze 1-9 (az 180 bajtu; od verze 10 ma delka 16 bitu). Pro obrazovku pady jadra
//! (kresli se v obsluze paniky, heap muze byt zamceny).
//!
//! Postup podle referencni implementace Project Nayuki (MIT): funkcni
//! vzory (vyhledavaci, casovaci, zarovnavaci, format a verze), data se
//! zakoncenim a vyplni, Reed-Solomon po blocich a prokladani, cik-cak
//! umisteni a maska s nejnizsi penalizaci.

#![cfg_attr(not(test), no_std)]

/// Nejvyssi podporovana verze (57 x 57 modulu).
pub const MAX_VERSION: usize = 9;
const MAX_SIZE: usize = 17 + 4 * MAX_VERSION;
/// Nejvic kodovych slov (verze 9).
const MAX_CODEWORDS: usize = 292;

/// Kodova slova pro opravu chyb na blok a pocet bloku pro uroven M,
/// verze 1-9.
const ECC_PER_BLOCK: [u8; MAX_VERSION] = [10, 16, 26, 18, 24, 16, 18, 22, 22];
const BLOCKS: [u8; MAX_VERSION] = [1, 1, 1, 2, 2, 4, 4, 4, 5];

/// Hotovy kod: ctverec `size` x `size` modulu (tmavy = `true`).
#[derive(Clone)]
pub struct Qr {
    pub version: usize,
    pub size: usize,
    rows: [u64; MAX_SIZE],
    func: [u64; MAX_SIZE],
}

impl Qr {
    /// Tmavy modul na (`x`, `y`)?
    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.size && y < self.size && self.rows[y] >> x & 1 != 0
    }

    fn set(&mut self, x: usize, y: usize, dark: bool) {
        if dark {
            self.rows[y] |= 1 << x;
        } else {
            self.rows[y] &= !(1 << x);
        }
    }

    fn set_func(&mut self, x: usize, y: usize, dark: bool) {
        self.set(x, y, dark);
        self.func[y] |= 1 << x;
    }

    fn is_func(&self, x: usize, y: usize) -> bool {
        self.func[y] >> x & 1 != 0
    }
}

/// Pocet modulu pro data a opravu chyb (bez funkcnich vzoru).
fn raw_modules(ver: usize) -> usize {
    let mut r = (16 * ver + 128) * ver + 64;
    if ver >= 2 {
        let n = ver / 7 + 2;
        r -= (25 * n - 10) * n - 55;
        if ver >= 7 {
            r -= 36;
        }
    }
    r
}

/// Kolik datovych kodovych slov (bez opravy) ma verze `ver`.
pub fn data_codewords(ver: usize) -> usize {
    raw_modules(ver) / 8 - ECC_PER_BLOCK[ver - 1] as usize * BLOCKS[ver - 1] as usize
}

/// Nejvic bajtu dat ve verzi `ver` (hlavicka: rezim 4 b, delka 8 b).
pub fn capacity(ver: usize) -> usize {
    data_codewords(ver) - 2
}

/// Zakoduje `data`; `None` = nevejde se do verze 9.
pub fn encode(data: &[u8]) -> Option<Qr> {
    let ver = (1..=MAX_VERSION).find(|&v| data.len() <= capacity(v))?;
    let size = 17 + 4 * ver;
    let mut q = Qr {
        version: ver,
        size,
        rows: [0; MAX_SIZE],
        func: [0; MAX_SIZE],
    };
    function_patterns(&mut q);

    // Data: rezim 0100 (bajty), delka 8 b, data, zakonceni, vypln.
    let cap = data_codewords(ver);
    let mut cw = [0u8; MAX_CODEWORDS];
    let mut bits = 0usize;
    fn put(cw: &mut [u8; MAX_CODEWORDS], bits: &mut usize, val: u32, n: usize) {
        for i in (0..n).rev() {
            if val >> i & 1 != 0 {
                cw[*bits / 8] |= 0x80 >> (*bits % 8);
            }
            *bits += 1;
        }
    }
    put(&mut cw, &mut bits, 0b0100, 4);
    put(&mut cw, &mut bits, data.len() as u32, 8);
    for &b in data {
        put(&mut cw, &mut bits, b as u32, 8);
    }
    let end = (cap * 8 - bits).min(4);
    put(&mut cw, &mut bits, 0, end);
    let mut n = bits.div_ceil(8);
    let mut pad = 0xEC;
    while n < cap {
        cw[n] = pad;
        pad ^= 0xEC ^ 0x11;
        n += 1;
    }

    let mut all = [0u8; MAX_CODEWORDS];
    let total = interleave(ver, &cw[..cap], &mut all);
    draw_codewords(&mut q, &all[..total]);

    // Maska s nejnizsi penalizaci.
    let mut best = (u32::MAX, 0);
    for mask in 0..8 {
        apply_mask(&mut q, mask);
        draw_format(&mut q, mask);
        let p = penalty(&q);
        if p < best.0 {
            best = (p, mask);
        }
        apply_mask(&mut q, mask);
    }
    apply_mask(&mut q, best.1);
    draw_format(&mut q, best.1);
    Some(q)
}

fn function_patterns(q: &mut Qr) {
    let size = q.size;
    for i in 0..size {
        q.set_func(6, i, i % 2 == 0);
        q.set_func(i, 6, i % 2 == 0);
    }
    for (cx, cy) in [(3, 3), (size - 4, 3), (3, size - 4)] {
        for dy in -4i32..=4 {
            for dx in -4i32..=4 {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if (0..size as i32).contains(&x) && (0..size as i32).contains(&y) {
                    let d = dx.abs().max(dy.abs());
                    q.set_func(x as usize, y as usize, d != 2 && d != 4);
                }
            }
        }
    }
    let (pos, n) = alignment_positions(q.version);
    for i in 0..n {
        for j in 0..n {
            if (i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0) {
                continue;
            }
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let d = dx.abs().max(dy.abs());
                    q.set_func((pos[i] as i32 + dx) as usize, (pos[j] as i32 + dy) as usize, d != 1);
                }
            }
        }
    }
    // Format zatim jako funkcni moduly (hodnota prijde s maskou).
    draw_format(q, 0);
    if q.version >= 7 {
        let ver = q.version as u32;
        let mut rem = ver;
        for _ in 0..12 {
            rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
        }
        let bits = ver << 12 | rem;
        for i in 0..18 {
            let dark = bits >> i & 1 != 0;
            let (a, b) = (size - 11 + i % 3, i / 3);
            q.set_func(a, b, dark);
            q.set_func(b, a, dark);
        }
    }
}

/// Stredy zarovnavacich vzoru (a jejich pocet).
fn alignment_positions(ver: usize) -> ([usize; 7], usize) {
    let mut out = [0usize; 7];
    if ver == 1 {
        return (out, 0);
    }
    let n = ver / 7 + 2;
    let step = (ver * 8 + n * 3 + 5) / (n * 4 - 4) * 2;
    out[0] = 6;
    let mut pos = 17 + 4 * ver - 7;
    for i in (1..n).rev() {
        out[i] = pos;
        pos -= step;
    }
    (out, n)
}

/// Format: uroven M (00) a maska, BCH (15,5), XOR 0x5412.
fn format_bits(mask: u32) -> u32 {
    let data = mask; // M = 00
    let mut rem = data;
    for _ in 0..10 {
        rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    (data << 10 | rem) ^ 0x5412
}

fn draw_format(q: &mut Qr, mask: u32) {
    let bits = format_bits(mask);
    let bit = |i: usize| bits >> i & 1 != 0;
    let size = q.size;
    for i in 0..=5 {
        q.set_func(8, i, bit(i));
    }
    q.set_func(8, 7, bit(6));
    q.set_func(8, 8, bit(7));
    q.set_func(7, 8, bit(8));
    for i in 9..15 {
        q.set_func(14 - i, 8, bit(i));
    }
    for i in 0..8 {
        q.set_func(size - 1 - i, 8, bit(i));
    }
    for i in 8..15 {
        q.set_func(8, size - 15 + i, bit(i));
    }
    q.set_func(8, size - 8, true);
}

/// Nasobeni v GF(2^8) s polynomem 0x11D.
fn gf_mul(x: u8, y: u8) -> u8 {
    let mut z: u32 = 0;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11D);
        z ^= ((y as u32 >> i) & 1) * x as u32;
    }
    z as u8
}

/// Delitel Reed-Solomon stupne `deg` (nejvys 30).
fn rs_divisor(deg: usize, out: &mut [u8; 30]) {
    out[..deg].fill(0);
    out[deg - 1] = 1;
    let mut root = 1u8;
    for _ in 0..deg {
        for j in 0..deg {
            out[j] = gf_mul(out[j], root);
            if j + 1 < deg {
                out[j] ^= out[j + 1];
            }
        }
        root = gf_mul(root, 2);
    }
}

fn rs_remainder(data: &[u8], div: &[u8], out: &mut [u8]) {
    let deg = div.len();
    out[..deg].fill(0);
    for &b in data {
        let factor = b ^ out[0];
        out.copy_within(1..deg, 0);
        out[deg - 1] = 0;
        for i in 0..deg {
            out[i] ^= gf_mul(div[i], factor);
        }
    }
}

/// Bloky s opravou chyb a jejich prokladani; vraci pocet kodovych slov.
fn interleave(ver: usize, data: &[u8], out: &mut [u8; MAX_CODEWORDS]) -> usize {
    let blocks = BLOCKS[ver - 1] as usize;
    let ecc_len = ECC_PER_BLOCK[ver - 1] as usize;
    let raw = raw_modules(ver) / 8;
    let short_blocks = blocks - raw % blocks;
    let short_len = raw / blocks;
    let mut div = [0u8; 30];
    rs_divisor(ecc_len, &mut div);
    // Bloky do pole [blok][slovo] (nejvys 5 bloku po 69 slovech + rezerva).
    let mut b = [[0u8; 80]; 5];
    let mut k = 0;
    for (i, blk) in b.iter_mut().enumerate().take(blocks) {
        let dat_len = short_len - ecc_len + usize::from(i >= short_blocks);
        blk[..dat_len].copy_from_slice(&data[k..k + dat_len]);
        k += dat_len;
        let mut ecc = [0u8; 30];
        rs_remainder(&blk[..dat_len], &div[..ecc_len], &mut ecc);
        // Kratky blok ma za daty jedno misto navic (pri prokladani se
        // preskoci), aby ECC lezelo stejne jako u dlouhych bloku.
        let at = short_len - ecc_len + 1;
        blk[at..at + ecc_len].copy_from_slice(&ecc[..ecc_len]);
    }
    let mut n = 0;
    for i in 0..=short_len {
        for (j, blk) in b.iter().enumerate().take(blocks) {
            if i != short_len - ecc_len || j >= short_blocks {
                out[n] = blk[i];
                n += 1;
            }
        }
    }
    n
}

fn draw_codewords(q: &mut Qr, data: &[u8]) {
    let size = q.size;
    let mut i = 0usize;
    let mut right = size as i32 - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..size {
            for j in 0..2 {
                let x = (right - j) as usize;
                let upward = (right + 1) & 2 == 0;
                let y = if upward { size - 1 - vert } else { vert };
                if !q.is_func(x, y) && i < data.len() * 8 {
                    let dark = data[i >> 3] >> (7 - (i & 7)) & 1 != 0;
                    q.set(x, y, dark);
                    i += 1;
                }
            }
        }
        right -= 2;
    }
}

fn mask_bit(mask: u32, x: usize, y: usize) -> bool {
    match mask {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => (x / 3 + y / 2) % 2 == 0,
        5 => x * y % 2 + x * y % 3 == 0,
        6 => (x * y % 2 + x * y % 3) % 2 == 0,
        _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
    }
}

/// XOR masky na datove moduly (podruhe ji vrati zpet).
fn apply_mask(q: &mut Qr, mask: u32) {
    for y in 0..q.size {
        for x in 0..q.size {
            if !q.is_func(x, y) && mask_bit(mask, x, y) {
                let v = q.get(x, y);
                q.set(x, y, !v);
            }
        }
    }
}

/// Penalizace podle normy (N1 behy, N2 bloky 2x2, N3 vzory podobne
/// vyhledavacim, N4 pomer tmavych).
fn penalty(q: &Qr) -> u32 {
    let n = q.size;
    let mut p = 0u32;
    let line = |i: usize, k: usize, horiz: bool| if horiz { q.get(k, i) } else { q.get(i, k) };
    for horiz in [true, false] {
        for i in 0..n {
            let mut run = 1;
            for k in 1..n {
                if line(i, k, horiz) == line(i, k - 1, horiz) {
                    run += 1;
                    if run == 5 {
                        p += 3;
                    } else if run > 5 {
                        p += 1;
                    }
                } else {
                    run = 1;
                }
            }
            // N3: 1011101 se ctyrmi svetlymi pred nebo za.
            for k in 0..n.saturating_sub(6) {
                let pat = [true, false, true, true, true, false, true];
                if (0..7).all(|d| line(i, k + d, horiz) == pat[d]) {
                    let before = k >= 4 && (k - 4..k).all(|d| !line(i, d, horiz));
                    let after = k + 11 <= n && (k + 7..k + 11).all(|d| !line(i, d, horiz));
                    if before || after {
                        p += 40;
                    }
                }
            }
        }
    }
    let mut dark = 0;
    for y in 0..n {
        for x in 0..n {
            dark += u32::from(q.get(x, y));
            if x + 1 < n && y + 1 < n {
                let c = q.get(x, y);
                if q.get(x + 1, y) == c && q.get(x, y + 1) == c && q.get(x + 1, y + 1) == c {
                    p += 3;
                }
            }
        }
    }
    let total = (n * n) as u32;
    let k = (dark * 20).abs_diff(total * 10).div_ceil(total).saturating_sub(1);
    p + k * 10
}

#[cfg(test)]
mod tests;
