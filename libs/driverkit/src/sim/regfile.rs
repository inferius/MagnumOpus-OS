//! Pole registru pro modely: hodnoty po resetu, bity jen pro cteni a bity
//! "zapis jednicky maze" (W1C). Usetri vetsinu kodu modelu: model resi jen
//! registry, kde zapis neco spousti.

use alloc::collections::BTreeMap;

/// Jeden registr modelu.
#[derive(Debug, Clone, Copy)]
struct Spec {
    value: u64,
    reset: u64,
    /// Bity, ktere ovladac nezmeni.
    read_only: u64,
    /// Bity, ktere ovladac maze zapisem jednicky.
    w1c: u64,
}

/// Registry modelu podle offsetu.
#[derive(Debug, Clone, Default)]
pub struct RegFile {
    regs: BTreeMap<u64, Spec>,
}

impl RegFile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Obycejny registr (cteni i zapis).
    pub fn rw(mut self, off: u64, reset: u64) -> Self {
        self.regs.insert(off, Spec { value: reset, reset, read_only: 0, w1c: 0 });
        self
    }

    /// Registr jen pro cteni (identifikace, verze).
    pub fn ro(mut self, off: u64, value: u64) -> Self {
        self.regs.insert(off, Spec { value, reset: value, read_only: u64::MAX, w1c: 0 });
        self
    }

    /// Registr s bity jen pro cteni (`read_only`) a W1C (`w1c`).
    pub fn reg(mut self, off: u64, reset: u64, read_only: u64, w1c: u64) -> Self {
        self.regs.insert(off, Spec { value: reset, reset, read_only, w1c });
        self
    }

    /// Ma model registr na offsetu?
    pub fn has(&self, off: u64) -> bool {
        self.regs.contains_key(&off)
    }

    /// Cteni ovladacem; neznamy offset = 0 (jako rezervovany registr).
    pub fn read(&self, off: u64) -> u64 {
        self.regs.get(&off).map_or(0, |s| s.value)
    }

    /// Zapis ovladacem (respektuje RO a W1C). Vraci `false` pro neznamy
    /// offset (model to muze zalogovat jako chybu ovladace).
    pub fn write(&mut self, off: u64, v: u64) -> bool {
        let Some(s) = self.regs.get_mut(&off) else {
            return false;
        };
        let writable = !s.read_only & !s.w1c;
        let cleared = s.value & !(v & s.w1c);
        s.value = (cleared & !writable) | (v & writable);
        true
    }

    /// Hodnota z pohledu zarizeni (bez masek).
    pub fn get(&self, off: u64) -> u64 {
        self.read(off)
    }

    /// Nastaveni hodnoty zarizenim (bez masek).
    pub fn set(&mut self, off: u64, v: u64) {
        if let Some(s) = self.regs.get_mut(&off) {
            s.value = v;
        }
    }

    /// Zarizeni nastavi bity (napr. stav preruseni).
    pub fn set_bits(&mut self, off: u64, bits: u64) {
        let v = self.get(off) | bits;
        self.set(off, v);
    }

    /// Zarizeni smaze bity.
    pub fn clear_bits(&mut self, off: u64, bits: u64) {
        let v = self.get(off) & !bits;
        self.set(off, v);
    }

    /// Reset vsech registru na hodnoty po resetu.
    pub fn reset(&mut self) {
        for s in self.regs.values_mut() {
            s.value = s.reset;
        }
    }
}
