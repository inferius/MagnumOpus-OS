//! Zaznam stavu displeje od firmwaru (GOP) pro bezpecny navrat.
//!
//! Ovladac ho zapise do objektu pameti od initu **driv, nez poprve
//! zmeni registr** displeje. Objekt drzi init (prezije pad ovladace);
//! nova instance po padu ho precte a zapise registry zpet v poradi
//! zaznamu (napr. nejdriv vypnout rovinu, pak adresa, nakonec zapnout).
//!
//! ```text
//! "DSNP" | verze u32 | ovladac u32 | sirka u32 | vyska u32 | krok B u32
//!        | offset framebufferu u64 | pocet u32 | (registr u32, hodnota u32) * pocet
//! ```
//!
//! Vse little-endian. Platny je jen uplny zaznam (magic se zapisuje
//! posledni), takze pad uprostred zapisu nechava zaznam neplatny a nova
//! instance nic neprepisuje.

use alloc::vec::Vec;

const MAGIC: [u8; 4] = *b"DSNP";
const VERSION: u32 = 1;
const HEADER: usize = 4 + 4 + 4 + 4 + 4 + 4 + 8 + 4;
/// Nejvic registru v zaznamu.
pub const MAX_REGS: usize = 1024;

/// Ovladac, ktery zaznam vytvoril (jiny ho nesmi pouzit).
pub mod driver {
    pub const BOCHS: u32 = 1;
    pub const INTEL: u32 = 2;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub driver: u32,
    /// Rezim firmwaru.
    pub w: u32,
    pub h: u32,
    pub stride: u32,
    /// Kde v pameti zarizeni je framebuffer firmwaru (offset v aperture/VRAM).
    pub fb_offset: u64,
    /// Registry v poradi obnovy.
    pub regs: Vec<(u32, u32)>,
}

impl Snapshot {
    pub fn size(&self) -> usize {
        HEADER + self.regs.len() * 8
    }

    /// Zapise zaznam do `out` (magic nakonec). `false` = nevejde se.
    pub fn write(&self, out: &mut [u8]) -> bool {
        if self.regs.len() > MAX_REGS || out.len() < self.size() {
            return false;
        }
        out[..4].fill(0);
        let mut at = 4;
        let mut put = |b: &[u8]| {
            out[at..at + b.len()].copy_from_slice(b);
            at += b.len();
        };
        put(&VERSION.to_le_bytes());
        put(&self.driver.to_le_bytes());
        put(&self.w.to_le_bytes());
        put(&self.h.to_le_bytes());
        put(&self.stride.to_le_bytes());
        put(&self.fb_offset.to_le_bytes());
        put(&(self.regs.len() as u32).to_le_bytes());
        for &(r, v) in &self.regs {
            put(&r.to_le_bytes());
            put(&v.to_le_bytes());
        }
        out[..4].copy_from_slice(&MAGIC);
        true
    }

    /// Precte platny zaznam ovladace `driver`.
    pub fn read(b: &[u8], driver: u32) -> Option<Self> {
        if b.len() < HEADER || b[..4] != MAGIC {
            return None;
        }
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        if u32_at(4) != VERSION || u32_at(8) != driver {
            return None;
        }
        let n = u32_at(32) as usize;
        if n > MAX_REGS || b.len() < HEADER + n * 8 {
            return None;
        }
        let mut regs = Vec::with_capacity(n);
        for i in 0..n {
            let at = HEADER + i * 8;
            regs.push((u32_at(at), u32_at(at + 4)));
        }
        let mut off = [0u8; 8];
        off.copy_from_slice(&b[24..32]);
        Some(Snapshot {
            driver,
            w: u32_at(12),
            h: u32_at(16),
            stride: u32_at(20),
            fb_offset: u64::from_le_bytes(off),
            regs,
        })
    }

    /// Zneplatni zaznam (napr. po uspesne obnove a cistem konci).
    pub fn clear(b: &mut [u8]) {
        let n = b.len().min(4);
        b[..n].fill(0);
    }
}
