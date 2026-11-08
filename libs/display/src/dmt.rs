//! Vyber z tabulky VESA Display Monitor Timing (DMT 1.0 rev. 13): presne
//! casovani beznych rezimu. Standardni a ustalena casovani EDID se nejdriv
//! hledaji tady, teprve potom se pocitaji podle CVT (tak to dela i Linux,
//! `drm_edid.c`).

use crate::timing::{Timing, flags};

/// Zaznam tabulky: id DMT, casovani.
pub struct Entry {
    pub id: u8,
    pub timing: Timing,
}

const P: u32 = flags::HSYNC_POS | flags::VSYNC_POS;
const N: u32 = 0;
const RB: u32 = flags::HSYNC_POS | flags::REDUCED;
const PRB: u32 = P | flags::REDUCED;
const VP: u32 = flags::VSYNC_POS;

#[allow(clippy::too_many_arguments)]
const fn e(id: u8, clock_khz: u32, h: [u16; 4], v: [u16; 4], f: u32) -> Entry {
    Entry {
        id,
        timing: Timing {
            clock_khz,
            hactive: h[0],
            hfront: h[1],
            hsync: h[2],
            hback: h[3],
            vactive: v[0],
            vfront: v[1],
            vsync: v[2],
            vback: v[3],
            flags: f,
        },
    }
}

/// Tabulka (aktivni, predni porch, pulz, zadni porch).
pub const TABLE: &[Entry] = &[
    e(0x04, 25_175, [640, 16, 96, 48], [480, 10, 2, 33], N),
    e(0x05, 31_500, [640, 24, 40, 128], [480, 9, 3, 28], N),
    e(0x06, 31_500, [640, 16, 64, 120], [480, 1, 3, 16], N),
    e(0x08, 36_000, [800, 24, 72, 128], [600, 1, 2, 22], P),
    e(0x09, 40_000, [800, 40, 128, 88], [600, 1, 4, 23], P),
    e(0x0A, 50_000, [800, 56, 120, 64], [600, 37, 6, 23], P),
    e(0x0B, 49_500, [800, 16, 80, 160], [600, 1, 3, 21], P),
    e(0x10, 65_000, [1024, 24, 136, 160], [768, 3, 6, 29], N),
    e(0x11, 75_000, [1024, 24, 136, 144], [768, 3, 6, 29], N),
    e(0x12, 78_750, [1024, 16, 96, 176], [768, 1, 3, 28], P),
    e(0x15, 108_000, [1152, 64, 128, 256], [864, 1, 3, 32], P),
    e(0x55, 74_250, [1280, 110, 40, 220], [720, 5, 5, 20], P),
    e(0x1C, 83_500, [1280, 72, 128, 200], [800, 3, 6, 22], VP),
    e(0x1B, 71_000, [1280, 48, 32, 80], [800, 3, 6, 14], RB),
    e(0x20, 108_000, [1280, 96, 112, 312], [960, 1, 3, 36], P),
    e(0x23, 108_000, [1280, 48, 112, 248], [1024, 1, 3, 38], P),
    e(0x24, 135_000, [1280, 16, 144, 248], [1024, 1, 3, 38], P),
    e(0x27, 85_500, [1360, 64, 112, 256], [768, 3, 6, 18], P),
    e(0x51, 85_500, [1366, 70, 143, 213], [768, 3, 3, 24], P),
    e(0x56, 72_000, [1366, 14, 56, 64], [768, 1, 3, 28], PRB),
    e(0x2E, 88_750, [1440, 48, 32, 80], [900, 3, 6, 17], RB),
    e(0x2F, 106_500, [1440, 80, 152, 232], [900, 3, 6, 25], VP),
    e(0x53, 108_000, [1600, 24, 80, 96], [900, 1, 3, 96], PRB),
    e(0x33, 162_000, [1600, 64, 192, 304], [1200, 1, 3, 46], P),
    e(0x39, 119_000, [1680, 48, 32, 80], [1050, 3, 6, 21], RB),
    e(0x3A, 146_250, [1680, 104, 176, 280], [1050, 3, 6, 30], VP),
    e(0x52, 148_500, [1920, 88, 44, 148], [1080, 4, 5, 36], P),
    e(0x44, 154_000, [1920, 48, 32, 80], [1200, 3, 6, 26], RB),
    e(0x45, 193_250, [1920, 136, 200, 336], [1200, 3, 6, 36], VP),
    e(0x4C, 268_500, [2560, 48, 32, 80], [1600, 3, 6, 37], RB),
];

/// Zaznam DMT pro rozmery a obnovu (cele Hz). Preferuje beznou variantu
/// pred redukovanym zatemnenim, pokud `reduced` nerika jinak.
pub fn find(w: u32, h: u32, refresh_hz: u32, reduced: Option<bool>) -> Option<&'static Entry> {
    let mut best: Option<&Entry> = None;
    for e in TABLE {
        let t = &e.timing;
        if t.hactive as u32 != w || t.vactive as u32 != h || t.refresh_hz() != refresh_hz {
            continue;
        }
        let is_rb = t.flags & flags::REDUCED != 0;
        match reduced {
            Some(r) if r != is_rb => continue,
            _ => {}
        }
        if best.is_none() || (!is_rb && reduced.is_none()) {
            best = Some(e);
        }
    }
    best
}

/// Zaznam podle id DMT.
pub fn by_id(id: u8) -> Option<&'static Entry> {
    TABLE.iter().find(|e| e.id == id)
}
