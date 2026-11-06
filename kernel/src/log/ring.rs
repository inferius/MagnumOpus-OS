//! Kruhovy buffer poslednich zaznamu logu.
//!
//! Uklada hotove radky (UTF-8, ukoncene `\n`). Pri preteceni se prepisuji
//! nejstarsi data; ctenar pozna ztratu podle celkove pozice zapisu.

use crate::sync::SpinLock;

const CAPACITY: usize = 64 * 1024;

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
