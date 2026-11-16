//! Tabulka velkych pismen exFAT (porovnani jmen bez ohledu na velikost
//! a hash jmena). Na mediu je komprimovana: `0xFFFF, n` = dalsich `n`
//! znaku se mapuje samo na sebe.

/// Nejvic bajtu tabulky na mediu (nekomprimovana ma 128 KiB).
pub const MAX_BYTES: u64 = 256 * 1024;

pub struct Upcase(Vec<u16>);

impl Upcase {
    /// Tabulka podle Unicode (svazek bez platne tabulky, testy).
    pub fn generated() -> Self {
        let mut t: Vec<u16> = (0..=0xFFFFu32).map(|c| c as u16).collect();
        for (i, v) in t.iter_mut().enumerate() {
            if let Some(ch) = char::from_u32(i as u32) {
                let mut up = ch.to_uppercase();
                if let (Some(u), None) = (up.next(), up.next())
                    && (u as u32) <= 0xFFFF
                {
                    *v = u as u16;
                }
            }
        }
        Upcase(t)
    }

    /// Tabulka z media; `None` = spatny kontrolni soucet.
    pub fn parse(raw: &[u8], checksum: u32) -> Option<Self> {
        let mut sum: u32 = 0;
        for &b in raw {
            sum = sum.rotate_right(1).wrapping_add(b as u32);
        }
        if sum != checksum {
            return None;
        }
        let mut t: Vec<u16> = (0..=0xFFFFu32).map(|c| c as u16).collect();
        let words: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let mut idx = 0usize;
        let mut i = 0;
        while i < words.len() && idx < t.len() {
            if words[i] == 0xFFFF && i + 1 < words.len() {
                idx += words[i + 1] as usize;
                i += 2;
                continue;
            }
            t[idx] = words[i];
            idx += 1;
            i += 1;
        }
        Some(Upcase(t))
    }

    pub fn up(&self, c: u16) -> u16 {
        self.0[c as usize]
    }

    /// Jmena se rovnaji bez ohledu na velikost pismen.
    pub fn eq(&self, a: &[u16], b: &[u16]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| self.up(x) == self.up(y))
    }
}
