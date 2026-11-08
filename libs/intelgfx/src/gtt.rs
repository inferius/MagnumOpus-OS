//! Globalni GTT (GGTT): tabulka stranek grafiky v BAR0 od 8 MiB, polozka
//! 8 B na stranku 4 KiB (`intel_ggtt.c`, `intel_gtt.h`, MIT): adresa
//! stranky (bity 45:12), bit 0 = platna, bit 1 = lokalni pamet karty
//! (diskretni, Gen12+).
//!
//! Rovina displeje adresuje framebuffer v GGTT (`PLANE_SURF`). Ovladac
//! tak zjisti, kde framebuffer firmwaru fyzicky lezi (stolen pamet nebo
//! pamet karty), a pro vlastni buffer zapise polozky sam.

use alloc::vec::Vec;

use crate::Mmio;
use crate::regs::{GGTT_OFFSET, PTE_ADDR_MASK, PTE_LM, PTE_PRESENT};

pub const PAGE: u64 = 4096;

/// Polozka GGTT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pte {
    pub addr: u64,
    /// Lokalni pamet karty (offset v LMEM), jinak fyzicka adresa systemu.
    pub local: bool,
}

fn reg(ggtt: u32) -> u32 {
    GGTT_OFFSET + (ggtt >> 12) * 8
}

pub fn read<M: Mmio + ?Sized>(m: &mut M, ggtt: u32) -> Option<Pte> {
    let v = m.read64(reg(ggtt));
    if v & PTE_PRESENT == 0 {
        return None;
    }
    Some(Pte {
        addr: v & PTE_ADDR_MASK,
        local: v & PTE_LM != 0,
    })
}

/// Fyzicky souvisly rozsah za GGTT adresou `ggtt` delky `len`:
/// `Some((zacatek, lokalni))` jen kdyz jsou stranky za sebou (framebuffer
/// firmwaru ve stolen pameti byva souvisly).
pub fn contiguous<M: Mmio + ?Sized>(m: &mut M, ggtt: u32, len: u64) -> Option<(u64, bool)> {
    let first = read(m, ggtt)?;
    let pages = len.div_ceil(PAGE);
    for i in 1..pages {
        let p = read(m, ggtt + (i * PAGE) as u32)?;
        if p.local != first.local || p.addr != first.addr + i * PAGE {
            return None;
        }
    }
    Some((first.addr, first.local))
}

/// Zapise polozky pro souvisly fyzicky rozsah (systemova pamet).
pub fn map<M: Mmio + ?Sized>(m: &mut M, ggtt: u32, phys: u64, len: u64, local: bool) {
    for i in 0..len.div_ceil(PAGE) {
        let v = ((phys + i * PAGE) & PTE_ADDR_MASK) | PTE_PRESENT | if local { PTE_LM } else { 0 };
        m.write64(reg(ggtt + (i * PAGE) as u32), v);
    }
}

/// Je rozsah GGTT volny (zadna platna polozka)?
pub fn is_free<M: Mmio + ?Sized>(m: &mut M, ggtt: u32, len: u64) -> bool {
    (0..len.div_ceil(PAGE)).all(|i| read(m, ggtt + (i * PAGE) as u32).is_none())
}

/// Puvodni polozky rozsahu (zaznam pro obnovu).
pub fn save<M: Mmio + ?Sized>(m: &mut M, ggtt: u32, len: u64) -> Vec<u64> {
    (0..len.div_ceil(PAGE)).map(|i| m.read64(reg(ggtt + (i * PAGE) as u32))).collect()
}
