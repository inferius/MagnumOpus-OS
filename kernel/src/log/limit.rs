//! Omezeni opakovanych hlasek: z jednoho mista (soubor a radek jadra,
//! nebo proces a text bez cisel u `DEBUG_LOG`) nejvys [`PER_SECOND`]
//! zaznamu za sekundu. Dalsi se jen spocitaji; s prvni zpravou z toho mista
//! v nove sekunde prijde souhrn "potlaceno N x". Tak zaplava jedne hlasky
//! (stovky za sekundu) nevytlaci z kruhoveho bufferu dulezite zpravy.
//!
//! Tabulka ma pevnou velikost (bez alokace); misto, ktere se do ni
//! nevejde (kolize), se neomezuje.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::sync::SpinLock;

/// Nejvic zaznamu z jednoho mista za sekundu.
pub const PER_SECOND: u32 = 10;
const SLOTS: usize = 128;

#[derive(Clone, Copy)]
struct Slot {
    key: u64,
    sec: u64,
    count: u32,
    suppressed: u32,
}

static TABLE: SpinLock<[Slot; SLOTS]> = SpinLock::new(
    [Slot {
        key: 0,
        sec: 0,
        count: 0,
        suppressed: 0,
    }; SLOTS],
);

/// Celkem potlacenych zaznamu (statistika).
pub static SUPPRESSED: AtomicU32 = AtomicU32::new(0);

/// Vysledek kontroly.
pub enum Verdict {
    /// Zapsat; `u32` = kolik zaznamu z tohoto mista se v minulych sekundach
    /// potlacilo (pred zaznam souhrn, kdyz > 0).
    Log(u32),
    /// Potlacit.
    Drop,
}

/// Zkontroluje zaznam z mista `key` v case `now_ns`.
pub fn check(key: u64, now_ns: u64) -> Verdict {
    let key = key | 1; // 0 = volny slot
    let sec = now_ns / 1_000_000_000;
    // Nikdy necekat (logovani z preruseni): obsazena tabulka = zapsat.
    let Some(mut t) = TABLE.try_lock() else {
        return Verdict::Log(0);
    };
    let i = (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 57) as usize % SLOTS;
    let s = &mut t[i];
    if s.key != key {
        // Volny nebo cizi slot: prevzit (stara statistika se zahodi).
        *s = Slot {
            key,
            sec,
            count: 1,
            suppressed: 0,
        };
        return Verdict::Log(0);
    }
    if s.sec != sec {
        let was = s.suppressed;
        s.sec = sec;
        s.count = 1;
        s.suppressed = 0;
        return Verdict::Log(was);
    }
    s.count += 1;
    if s.count > PER_SECOND {
        s.suppressed += 1;
        SUPPRESSED.fetch_add(1, Ordering::Relaxed);
        Verdict::Drop
    } else {
        Verdict::Log(0)
    }
}

/// Klic textu z userspace: proces a text bez cislic (stejna hlaska
/// s jinym cislem = stejne misto).
pub fn text_key(pid: u64, text: &str) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64 ^ pid.wrapping_mul(0x100_0000_01B3);
    for b in text.bytes().take(96).filter(|b| !b.is_ascii_digit()) {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01B3);
    }
    h
}
