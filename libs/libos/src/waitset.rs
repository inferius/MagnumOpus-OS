//! Nahrada `wait_many` pro promenny pocet objektu: stejne rozhrani (pole
//! `(handle, maska)` -> pozorovane signaly po polozkach), ale ceka se pres
//! port, takze pocet objektu omezuje jen pamet (ne `WAIT_MANY_MAX` = 64).
//!
//! Hodi se pro smycky, ktere uz pole polozek skladaji (compd, termd...);
//! nova smycka muze pouzit primo [`crate::Waiter`].
//!
//! - Registrace se drzi mezi volanimi: objekt, ktery byl v minulem volani
//!   a nic se nestalo, se znovu neregistruje. Po paketu (nebo zmene masky)
//!   se zaregistruje znovu; uz splnene signaly doruci paket hned, takze
//!   chovani je jako u `wait_many` (nic se neztrati, cteni po davkach OK).
//! - Objekt se pozna podle hodnoty handlu: ta obsahuje generaci slotu, po
//!   zavreni a novem otevreni ma jiny handle jinou hodnotu.
//! - Objekt, ktery v poli chybi, se zapomene (pozdni paket jeho registrace
//!   se zahodi).
//! - Port lze sdilet ([`WaitSet::port`]) pro vlastni registrace (napr.
//!   ploty: `Fence::wait_async`) s klici pod [`FOREIGN_KEYS`]: takovy paket
//!   probudi `wait` (vysledek bez signalu polozek); s [`WaitSet::keep_foreign`]
//!   si ho volajici vyzvedne ([`WaitSet::take_foreign`]), jinak se zahodi.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;

use abi::{Error, Result, Signals};

use crate::{Handle, Port, PortPacket};

/// Klice cizich registraci na sdilenem portu musi byt mensi (vlastni klice
/// `WaitSet` maji nejvyssi bit).
pub const FOREIGN_KEYS: u64 = 1 << 63;

struct Entry {
    key: u64,
    mask: Signals,
    armed: bool,
    /// Index polozky v poslednim volani.
    index: usize,
    /// Cislo volani, ve kterem objekt naposledy byl.
    epoch: u64,
}

pub struct WaitSet {
    port: Port,
    next: u64,
    epoch: u64,
    /// Hodnota handlu -> registrace.
    entries: BTreeMap<u32, Entry>,
    /// Klic -> hodnota handlu (paket -> polozka).
    keys: BTreeMap<u64, u32>,
    /// Cizi pakety (klic `< FOREIGN_KEYS`) pro volajiciho; `None` = zahodit.
    foreign: Option<Vec<PortPacket>>,
}

impl WaitSet {
    pub fn new() -> Result<Self> {
        Ok(WaitSet { port: Port::create()?, next: FOREIGN_KEYS, epoch: 0, entries: BTreeMap::new(), keys: BTreeMap::new(), foreign: None })
    }

    /// Port mnoziny pro vlastni registrace s klici `< FOREIGN_KEYS`.
    pub fn port(&self) -> &Port {
        &self.port
    }

    /// Cizi pakety schovavat pro [`WaitSet::take_foreign`] (volajici je
    /// musi vybirat, jinak fronta roste).
    pub fn keep_foreign(&mut self) {
        if self.foreign.is_none() {
            self.foreign = Some(Vec::new());
        }
    }

    /// Cizi pakety od posledniho vyzvednuti (v poradi doruceni).
    pub fn take_foreign(&mut self) -> Vec<PortPacket> {
        self.foreign.as_mut().map(core::mem::take).unwrap_or_default()
    }

    fn new_key(&mut self) -> u64 {
        let k = self.next;
        self.next += 1;
        k
    }

    /// Jako [`crate::wait_many`]: ceka, az aspon jedna polozka ma signal
    /// z masky, do `deadline` (`None` = bez limitu; uplynuti = `TIMED_OUT`).
    /// Vrati index prvni pripravene polozky (`items.len()` = probudil cizi
    /// paket) a signaly po polozkach (0 = nic se nestalo).
    pub fn wait(&mut self, items: &[(&Handle, Signals)], deadline: Option<u64>) -> Result<(usize, Vec<Signals>)> {
        self.epoch += 1;
        let epoch = self.epoch;
        for (i, (h, mask)) in items.iter().enumerate() {
            let raw = h.raw();
            let stale = match self.entries.get(&raw) {
                Some(e) => e.mask != *mask && e.armed,
                None => false,
            };
            if stale {
                // Cekajici registrace se starou maskou: novy klic, stara se
                // pozdeji zahodi.
                let old = self.entries.remove(&raw).unwrap();
                self.keys.remove(&old.key);
            }
            if !self.entries.contains_key(&raw) {
                let key = self.new_key();
                self.keys.insert(key, raw);
                self.entries.insert(raw, Entry { key, mask: *mask, armed: false, index: i, epoch });
            }
            let e = self.entries.get_mut(&raw).unwrap();
            e.index = i;
            e.epoch = epoch;
            e.mask = *mask;
            if !e.armed {
                self.port.wait_async(h, e.key, *mask)?;
                e.armed = true;
            }
        }
        // Objekty, ktere v poli nejsou, zapomenout.
        let keys = &mut self.keys;
        self.entries.retain(|_, e| {
            let keep = e.epoch == epoch;
            if !keep {
                keys.remove(&e.key);
            }
            keep
        });
        let mut out = vec![Signals(0); items.len()];
        let mut first = items.len();
        let mut p = match self.port.wait(deadline) {
            Ok(p) => p,
            Err(e) => return Err(e),
        };
        loop {
            if let Some(raw) = self.keys.get(&p.key)
                && let Some(e) = self.entries.get_mut(raw)
            {
                e.armed = false;
                out[e.index] = Signals(out[e.index].0 | p.signals);
                if Signals(p.signals).intersects(e.mask) {
                    first = first.min(e.index);
                }
            } else if p.key < FOREIGN_KEYS
                && let Some(f) = self.foreign.as_mut()
            {
                f.push(p);
            }
            p = match self.port.wait(Some(0)) {
                Ok(p) => p,
                Err(Error::TimedOut) => break,
                Err(e) => return Err(e),
            };
        }
        Ok((first, out))
    }
}
