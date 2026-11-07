//! Bariery pameti pro ovladace: poradi pristupu do pameti sdilene se
//! zarizenim (DMA) a do registru zarizeni (MMIO).
//!
//! Atomicke operace jazyka (`core::sync::atomic::fence`) radi pristupy jen
//! mezi procesory. Zarizeni je dalsi pozorovatel: na aarch64 `fence` vlozi
//! `dmb ish` (jen vnitrni sdilitelna domena, kam zarizeni nemusi patrit)
//! a zapis do registru zarizeni muze predbehnout drivejsi zapisy do pameti.
//! Ovladac proto pouziva tyto funkce:
//!
//! | funkce       | kdy                                                   | x86_64          | aarch64      |
//! |--------------|-------------------------------------------------------|-----------------|--------------|
//! | [`dma_wmb`]  | data/deskriptory v DMA pameti pred zapisem indexu     | `fence(SeqCst)` | `dmb oshst`  |
//! | [`dma_rmb`]  | precteny index od zarizeni pred ctenim jeho dat       | `fence(SeqCst)` | `dmb oshld`  |
//! | [`mmio_wmb`] | vsechny zapisy do pameti pred zapisem do registru     | `fence(SeqCst)` | `dsb st`     |
//!
//! Na x86_64 je to stejna instrukce jako drive (`mfence`): pamet je tam
//! koherentni a razeni silne, bariera hlavne brani prekladaci presouvat
//! pristupy.
//!
//! Priklad (virtqueue):
//!
//! ```ignore
//! write_descriptors();
//! libos::barrier::dma_wmb();   // deskriptory driv nez index
//! write_avail_index();
//! libos::barrier::mmio_wmb();  // index driv nez "doorbell"
//! notify_register.write(queue);
//! ```

#[cfg(target_arch = "aarch64")]
use core::arch::asm;
#[cfg(not(target_arch = "aarch64"))]
use core::sync::atomic::{Ordering, fence};

/// Zapisy do DMA pameti pred touto barierou zarizeni uvidi driv nez zapisy
/// do DMA pameti za ni.
#[inline]
pub fn dma_wmb() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: bariera jen radi pristupy, nemeni stav pameti.
    unsafe {
        asm!("dmb oshst", options(nostack, preserves_flags))
    };
    #[cfg(not(target_arch = "aarch64"))]
    fence(Ordering::SeqCst);
}

/// Cteni z DMA pameti za touto barierou se provedou az po cteni pred ni
/// (napr. index dokoncenych pozadavku -> jejich data).
#[inline]
pub fn dma_rmb() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: viz `dma_wmb`.
    unsafe {
        asm!("dmb oshld", options(nostack, preserves_flags))
    };
    #[cfg(not(target_arch = "aarch64"))]
    fence(Ordering::SeqCst);
}

/// Vsechny zapisy do pameti pred touto barierou jsou dokoncene drive, nez
/// zarizeni uvidi nasledujici zapis do sveho registru (doorbell).
#[inline]
pub fn mmio_wmb() {
    #[cfg(target_arch = "aarch64")]
    // SAFETY: viz `dma_wmb`.
    unsafe {
        asm!("dsb st", options(nostack, preserves_flags))
    };
    #[cfg(not(target_arch = "aarch64"))]
    fence(Ordering::SeqCst);
}
