//! Prideleni rozsahu oblasti sdilene pameti zarizeni (kam se mapuji bloby).

use alloc::vec::Vec;

/// First fit nad seznamem volnych rozsahu; vse zarovnane na `align`
/// (stranka hostitele muze byt 64 KiB).
#[derive(Debug, Clone)]
pub struct RangeAlloc {
    free: Vec<(u64, u64)>,
    align: u64,
}

impl RangeAlloc {
    pub fn new(len: u64, align: u64) -> Self {
        let len = len - len % align;
        Self { free: if len > 0 { alloc::vec![(0, len)] } else { Vec::new() }, align }
    }

    /// Volny rozsah aspon `size` bajtu; vraci offset.
    pub fn alloc(&mut self, size: u64) -> Option<u64> {
        let size = size.max(1).checked_next_multiple_of(self.align)?;
        let i = self.free.iter().position(|&(_, l)| l >= size)?;
        let (off, len) = self.free[i];
        if len == size {
            self.free.remove(i);
        } else {
            self.free[i] = (off + size, len - size);
        }
        Some(off)
    }

    /// Vrati rozsah (`off` a `size` jako u `alloc`); sousedni se slouci.
    pub fn release(&mut self, off: u64, size: u64) {
        let Some(size) = size.max(1).checked_next_multiple_of(self.align) else { return };
        self.free.push((off, size));
        self.free.sort_unstable();
        let mut out: Vec<(u64, u64)> = Vec::with_capacity(self.free.len());
        for &(o, l) in &self.free {
            match out.last_mut() {
                Some(last) if last.0 + last.1 == o => last.1 += l,
                _ => out.push((o, l)),
            }
        }
        self.free = out;
    }

    /// Volne bajty celkem.
    pub fn free_bytes(&self) -> u64 {
        self.free.iter().map(|f| f.1).sum()
    }
}
