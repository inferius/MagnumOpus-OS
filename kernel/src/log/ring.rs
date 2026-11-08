//! Kruhovy buffer poslednich zaznamu logu.
//!
//! Uklada hotove radky (UTF-8, ukoncene `\n`). Pri preteceni se prepisuji
//! nejstarsi data; ctenar pozna ztratu podle celkove pozice zapisu.

use crate::sync::SpinLock;

/// 512 KiB: cely start systemu (stovky radku) se vejde i s ladicim
/// vystupem, nez ho sluzba logu ulozi na disk.
const CAPACITY: usize = 512 * 1024;

struct Ring {
    data: [u8; CAPACITY],
    /// Celkovy pocet kdy zapsanych bajtu (pozice dalsiho zapisu).
    written: u64,
}

static RING: SpinLock<Ring> = SpinLock::new(Ring {
    data: [0; CAPACITY],
    written: 0,
});

pub fn push(bytes: &[u8]) {
    // try_lock: logovani nesmi nikdy uviznout (napr. vnorene logovani
    // z NMI); v nejhorsim se zaznam do bufferu nedostane, na seriovku ano.
    let Some(mut ring) = RING.try_lock() else {
        return;
    };
    for &b in bytes {
        let pos = (ring.written % CAPACITY as u64) as usize;
        ring.data[pos] = b;
        ring.written += 1;
    }
}

/// Zkopiruje do `out` data od pozice `cursor` (celkovy pocet zapsanych
/// bajtu). Starsi data, ktera uz buffer prepsal, preskoci: vraci skutecny
/// zacatek a pocet zkopirovanych bajtu.
pub fn read_from(cursor: u64, out: &mut [u8]) -> (u64, usize) {
    let ring = RING.lock();
    let written = ring.written;
    let start = cursor.clamp(written.saturating_sub(CAPACITY as u64), written);
    let n = ((written - start) as usize).min(out.len());
    for (i, o) in out[..n].iter_mut().enumerate() {
        *o = ring.data[((start + i as u64) % CAPACITY as u64) as usize];
    }
    (start, n)
}

/// Obsah bufferu pri pade (bez zamku): `f(starsi usek, novejsi usek)`.
///
/// # Safety
/// Ostatni CPU stoji (fatal_stop); zamek mohl drzet preruseny kod.
pub unsafe fn with_unlocked(f: impl FnOnce(&[u8], &[u8])) {
    // SAFETY: podle kontraktu volajiciho.
    let ring = unsafe { RING.force_get() };
    let written = ring.written;
    if written <= CAPACITY as u64 {
        f(&ring.data[..written as usize], &[]);
    } else {
        let start = (written % CAPACITY as u64) as usize;
        f(&ring.data[start..], &ring.data[..start]);
    }
}

/// Zavola `f` na obsah bufferu od nejstarsiho po nejnovejsi (dva useky,
/// pokud data pretekla). Vraci celkovy pocet kdy zapsanych bajtu.
pub fn snapshot(mut f: impl FnMut(&[u8])) -> u64 {
    let ring = RING.lock();
    let written = ring.written;
    if written <= CAPACITY as u64 {
        f(&ring.data[..written as usize]);
    } else {
        let start = (written % CAPACITY as u64) as usize;
        f(&ring.data[start..]);
        f(&ring.data[..start]);
    }
    written
}
