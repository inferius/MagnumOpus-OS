//! Zaznam padu jadra (pstore): format oblasti RAM, ktera prezije teply
//! restart.
//!
//! Jadro pri panice nebo fatalni vyjimce zapise do vyhrazene oblasti
//! ([`write`]) hlavicku a konec logu jadra (v nem uz je hlaseni pady,
//! registry, zasobnik a posledni zpravy jadra i sluzeb). Pri dalsim startu
//! ho jadro najde ([`parse`]), sluzba logu ho precte (syscall
//! `CRASH_RECORD`), ulozi na disk a smaze.
//!
//! Rozlozeni (little-endian, oblast typicky 64 KiB):
//!
//! | offset | velikost | pole |
//! |---|---|---|
//! | 0 | 8 | podpis `MOCRASH1` |
//! | 8 | 4 | verze formatu ([`VERSION`]) |
//! | 12 | 4 | priznaky ([`flags`]) |
//! | 16 | 4 | delka textu |
//! | 20 | 4 | CRC-32 bajtu 24..konec textu |
//! | 24 | 8 | cas od startu v ns |
//! | 32 | 8 | realny cas (s od 1970, 0 = neznamy) |
//! | 40 | 4 | CPU |
//! | 44 | 4 | rezerva |
//! | 48 | 32 | kod duvodu (ASCII, doplneno nulami), napr. `KERNEL_PANIC` |
//! | 80 | 32 | verze systemu (ASCII) |
//! | 112 | ... | text (UTF-8, konec logu jadra) |
//!
//! Knihovna je bez alokace (pouziva ji jadro v obsluze paniky); textovy
//! vystup pro soubor `.crash` je za feature `alloc`.

#![cfg_attr(not(test), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

pub const MAGIC: &[u8; 8] = b"MOCRASH1";
pub const VERSION: u32 = 1;
/// Velikost hlavicky; text zacina za ni.
pub const HEADER: usize = 112;

/// Priznaky zaznamu.
pub mod flags {
    /// Pad vyvolany testem (`CRASH_RECORD` op `TEST_PANIC`): jadro po
    /// zapisu hned restartuje.
    pub const TEST: u32 = 1 << 0;
}

/// Hlavicka zaznamu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub flags: u32,
    pub uptime_ns: u64,
    pub realtime_s: i64,
    pub cpu: u32,
    pub reason: [u8; 32],
    pub version: [u8; 32],
}

/// ASCII retezec do pevneho pole (oriznuty, doplneny nulami).
pub fn fixed<const N: usize>(s: &str) -> [u8; N] {
    let mut out = [0u8; N];
    for (o, b) in out.iter_mut().zip(s.bytes()) {
        *o = if b.is_ascii_graphic() || b == b' ' { b } else { b'?' };
    }
    out
}

/// Text pevneho pole bez koncovych nul.
pub fn field_str(f: &[u8]) -> &str {
    let end = f.iter().position(|&b| b == 0).unwrap_or(f.len());
    core::str::from_utf8(&f[..end]).unwrap_or("?")
}

/// CRC-32 (IEEE, jako zip), bez tabulky (zapis jen pri padu).
pub fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    crc = !crc;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Zapise zaznam do `area` (hlavicka a text z `parts` za sebou, oriznuty
/// na kapacitu tak, ze se zachova KONEC textu). Vraci delku textu.
pub fn write(area: &mut [u8], h: &Header, parts: &[&[u8]]) -> usize {
    if area.len() < HEADER {
        return 0;
    }
    let cap = area.len() - HEADER;
    let total: usize = parts.iter().map(|p| p.len()).sum();
    let mut skip = total.saturating_sub(cap);
    let mut at = HEADER;
    for p in parts {
        let p = if skip >= p.len() {
            skip -= p.len();
            continue;
        } else {
            let s = &p[skip..];
            skip = 0;
            s
        };
        area[at..at + p.len()].copy_from_slice(p);
        at += p.len();
    }
    let text_len = at - HEADER;
    area[8..12].copy_from_slice(&VERSION.to_le_bytes());
    area[12..16].copy_from_slice(&h.flags.to_le_bytes());
    area[16..20].copy_from_slice(&(text_len as u32).to_le_bytes());
    area[24..32].copy_from_slice(&h.uptime_ns.to_le_bytes());
    area[32..40].copy_from_slice(&h.realtime_s.to_le_bytes());
    area[40..44].copy_from_slice(&h.cpu.to_le_bytes());
    area[44..48].fill(0);
    area[48..80].copy_from_slice(&h.reason);
    area[80..112].copy_from_slice(&h.version);
    let crc = crc32_update(0, &area[24..HEADER + text_len]);
    area[20..24].copy_from_slice(&crc.to_le_bytes());
    // Podpis posledni: zaznam preruseny uprostred zapisu neplati.
    area[0..8].copy_from_slice(MAGIC);
    text_len
}

/// Platny zaznam v `area`: hlavicka, text a CRC (pro id). `None` = zadny
/// nebo poskozeny (RAM po vypnuti obsahuje nahodna data).
pub fn parse(area: &[u8]) -> Option<(Header, &[u8], u32)> {
    if area.len() < HEADER || &area[0..8] != MAGIC {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(area[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(area[o..o + 8].try_into().unwrap());
    if u32_at(8) != VERSION {
        return None;
    }
    let len = u32_at(16) as usize;
    if len > area.len() - HEADER {
        return None;
    }
    let crc = u32_at(20);
    if crc32_update(0, &area[24..HEADER + len]) != crc {
        return None;
    }
    let h = Header {
        flags: u32_at(12),
        uptime_ns: u64_at(24),
        realtime_s: u64_at(32) as i64,
        cpu: u32_at(40),
        reason: area[48..80].try_into().unwrap(),
        version: area[80..112].try_into().unwrap(),
    };
    Some((h, &area[HEADER..HEADER + len], crc))
}

/// Smaze zaznam (jen podpis; zbytek se prepise pri dalsim padu).
pub fn clear(area: &mut [u8]) {
    if area.len() >= 8 {
        area[0..8].fill(0);
    }
}

/// Kratke id zaznamu z CRC: `MO-7F3A-21C9` (12 znaku). Uzivatel ho cte
/// z obrazovky pady a hleda podle nej soubor.
pub fn crash_id(crc: u32) -> [u8; 12] {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = *b"MO-0000-0000";
    for i in 0..8 {
        let nib = (crc >> (28 - 4 * i)) & 0xF;
        let pos = if i < 4 { 3 + i } else { 4 + i };
        out[pos] = HEX[nib as usize];
    }
    out
}

/// Text souboru `.crash`: souhrn a pak puvodni text zaznamu.
#[cfg(feature = "alloc")]
pub fn render(h: &Header, text: &[u8], crc: u32) -> alloc::string::String {
    use core::fmt::Write;
    let mut s = alloc::string::String::new();
    let id = crash_id(crc);
    let _ = writeln!(s, "MagnumOpus: zaznam padu jadra");
    let _ = writeln!(s, "id:      #{}", core::str::from_utf8(&id).unwrap_or("?"));
    let _ = writeln!(s, "duvod:   {}", field_str(&h.reason));
    let _ = writeln!(s, "verze:   {}", field_str(&h.version));
    let _ = writeln!(
        s,
        "cas:     {}.{:03} s od startu, cpu {}",
        h.uptime_ns / 1_000_000_000,
        (h.uptime_ns / 1_000_000) % 1000,
        h.cpu
    );
    if h.realtime_s != 0 {
        let _ = writeln!(s, "realny cas: {} s od 1970 (UTC)", h.realtime_s);
    }
    if h.flags & flags::TEST != 0 {
        let _ = writeln!(s, "pozn.:   pad vyvolany testem");
    }
    let _ = writeln!(s, "adresy v zasobniku prelozi: cargo xtask symbolize <tento soubor>");
    let _ = writeln!(s, "--- konec logu jadra pred padem ---");
    s.push_str(&alloc::string::String::from_utf8_lossy(text));
    s
}

#[cfg(test)]
mod tests;
