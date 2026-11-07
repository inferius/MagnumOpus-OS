//! Spolecne pomucky pro radice v jadre (GIC, ITS, SMMU): pristup k registrum
//! zarizeni a pamet pro struktury, ktere cte hardware.

use super::regs::dcache_clean_invalidate;
use crate::mm::{self, PhysAddr};

/// Precte 32bitovy registr na virtualni adrese `addr`.
pub fn read32(addr: u64) -> u32 {
    // SAFETY: volajici predava adresu registru namapovaneho jako zarizeni
    // (`mm::map_mmio`); cteni registru radice nema jine ucinky.
    unsafe { (addr as *const u32).read_volatile() }
}

pub fn write32(addr: u64, v: u32) {
    // SAFETY: viz `read32`.
    unsafe { (addr as *mut u32).write_volatile(v) }
}

pub fn read64(addr: u64) -> u64 {
    // SAFETY: viz `read32`.
    unsafe { (addr as *const u64).read_volatile() }
}

pub fn write64(addr: u64, v: u64) {
    // SAFETY: viz `read32`.
    unsafe { (addr as *mut u64).write_volatile(v) }
}

/// Souvisla vynulovana pamet aspon `bytes` bajtu (2^n stranek, zarovnana na
/// svou velikost) pro struktury ctene hardwarem. Nuly jsou zapsane do
/// pameti (i pro zarizeni bez koherence s cache). `None` = nedostatek
/// pameti (za behu se vrati chyba, pri startu radicu `alloc_zeroed_boot`).
pub fn alloc_zeroed(bytes: u64) -> Option<PhysAddr> {
    let order = bytes.div_ceil(4096).next_power_of_two().trailing_zeros() as usize;
    let p = mm::phys::alloc_pages(order)?;
    // SAFETY: prave alokovane stranky v prime mape, nikdo jiny je nema.
    unsafe { p.to_virt().as_mut_ptr::<u8>().write_bytes(0, 4096 << order) };
    dcache_clean_invalidate(p.to_virt().as_u64(), 4096 << order);
    Some(p)
}

/// `alloc_zeroed` pri startu radice: bez pameti jadro stejne nemuze
/// pokracovat, panikuje se zpravou `what`.
pub fn alloc_zeroed_boot(bytes: u64, what: &str) -> PhysAddr {
    alloc_zeroed(bytes).unwrap_or_else(|| panic!("nedostatek pameti pro {what}"))
}
