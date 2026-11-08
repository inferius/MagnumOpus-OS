//! Zaznam padu jadra (pstore, format `libs/crashrec`).
//!
//! Loader vyhradi oblast RAM na pevne adrese (`BootInfo::pstore_phys`).
//! Pri panice nebo fatalni vyjimce do ni [`on_fatal`] zapise hlavicku
//! a konec logu jadra (hlaseni pady, registry, zasobnik, posledni zpravy
//! jadra i sluzeb), ukaze obrazovku pady ([`crate::crashscreen`]) a po
//! odpoctu restartuje. Teply restart RAM nemaze, takze pri dalsim startu
//! zaznam cte sluzba logu (`CRASH_RECORD`), ulozi ho na disk a smaze.
//!
//! Vse v ceste pady je bez alokace a bez cekani na zamky.

use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use boot_info::{BootInfo, PHYS_MAP_BASE};

use crate::log;

/// Virtualni adresa a delka oblasti (0 = neni).
static AREA: AtomicU64 = AtomicU64::new(0);
static AREA_LEN: AtomicUsize = AtomicUsize::new(0);
/// Kod duvodu pady (ukazatel a delka `&'static str`).
static REASON_PTR: AtomicUsize = AtomicUsize::new(0);
static REASON_LEN: AtomicUsize = AtomicUsize::new(0);
/// Pad vyvolal test (`CRASH_RECORD` `TEST_PANIC`): po zapisu hned restart.
static TEST_PANIC: AtomicBool = AtomicBool::new(false);

/// Verze systemu do zaznamu.
const VERSION: &str = concat!("MagnumOpus ", env!("CARGO_PKG_VERSION"));

/// Prevezme oblast od loaderu a zaloguje, zda v ni je zaznam z minuleho
/// behu.
pub fn init(boot_info: &BootInfo) {
    if boot_info.pstore_phys == 0 || boot_info.pstore_len < crashrec::HEADER as u64 {
        log::warn!("zaznam padu: oblast RAM neni (loader zadnou pevnou adresu nedostal), pady se neulozi");
        return;
    }
    AREA.store(PHYS_MAP_BASE + boot_info.pstore_phys, Ordering::Release);
    AREA_LEN.store(boot_info.pstore_len as usize, Ordering::Release);
    match area().and_then(|a| crashrec::parse(a)) {
        Some((h, _, crc)) => log::warn!(
            "zaznam padu z minuleho behu: {} #{} (ulozi ho sluzba logu)",
            crashrec::field_str(&h.reason),
            core::str::from_utf8(&crashrec::crash_id(crc)).unwrap_or("?")
        ),
        None => log::info!(
            "zaznam padu: oblast {} KiB na {:#x}, z minuleho behu nic",
            boot_info.pstore_len / 1024,
            boot_info.pstore_phys
        ),
    }
}

fn area() -> Option<&'static [u8]> {
    let a = AREA.load(Ordering::Acquire);
    // SAFETY: oblast je od loaderu vyhrazena jen pro nas (druh Pstore),
    // namapovana v prime mape po celou dobu behu.
    (a != 0).then(|| unsafe { core::slice::from_raw_parts(a as *const u8, AREA_LEN.load(Ordering::Acquire)) })
}

/// # Safety
/// Zapis smi jen jeden (pad: ostatni CPU stoji; `clear` je kratky a pri
/// soubeznem padu nanejvys smaze podpis noveho zaznamu).
unsafe fn area_mut() -> Option<&'static mut [u8]> {
    let a = AREA.load(Ordering::Acquire);
    // SAFETY: viz `area`; volajici ruci za jedineho zapisovatele.
    (a != 0).then(|| unsafe { core::slice::from_raw_parts_mut(a as *mut u8, AREA_LEN.load(Ordering::Acquire)) })
}

/// Kapacita oblasti (0 = neni).
pub fn capacity() -> usize {
    if AREA.load(Ordering::Acquire) == 0 { 0 } else { AREA_LEN.load(Ordering::Acquire) }
}

/// Zkopiruje platny zaznam (hlavicka a text) do `out`; vraci delku, 0 =
/// zadny.
pub fn read(out: &mut alloc::vec::Vec<u8>) -> usize {
    let Some(a) = area() else { return 0 };
    match crashrec::parse(a) {
        Some((_, text, _)) => {
            let n = crashrec::HEADER + text.len();
            out.extend_from_slice(&a[..n]);
            n
        }
        None => 0,
    }
}

pub fn clear() {
    // SAFETY: smaze jen podpis; viz `area_mut`.
    if let Some(a) = unsafe { area_mut() } {
        crashrec::clear(a);
    }
}

/// Duvod pady pro zaznam a obrazovku (kod, napr. `KERNEL_PANIC`).
pub fn set_reason(reason: &'static str) {
    REASON_LEN.store(reason.len(), Ordering::Relaxed);
    REASON_PTR.store(reason.as_ptr() as usize, Ordering::Release);
}

fn reason() -> &'static str {
    let p = REASON_PTR.load(Ordering::Acquire);
    if p == 0 {
        return "KERNEL_FATAL";
    }
    // SAFETY: ukazatel a delka pochazi z `&'static str` v `set_reason`.
    unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(p as *const u8, REASON_LEN.load(Ordering::Relaxed))) }
}

/// Test: dalsi panika je vyvolana testem (zaznam ma priznak TEST a jadro
/// hned restartuje).
pub fn mark_test_panic() {
    TEST_PANIC.store(true, Ordering::Release);
}

pub fn is_test_panic() -> bool {
    TEST_PANIC.load(Ordering::Acquire)
}

/// Zapise zaznam pady (volat pri fatalni chybe, ostatni CPU uz stoji).
/// Vraci (hlavicka, CRC) pro obrazovku pady, `None` = oblast neni.
pub fn on_fatal() -> Option<(crashrec::Header, u32)> {
    let h = crashrec::Header {
        flags: if is_test_panic() { crashrec::flags::TEST } else { 0 },
        uptime_ns: crate::arch::now_ns(),
        // Realny cas by potreboval zamek hodin: sluzba logu ho doplni
        // z casu startu a `uptime_ns`.
        realtime_s: 0,
        cpu: crate::sched::try_current_cpu_index().unwrap_or(0) as u32,
        reason: crashrec::fixed(reason()),
        version: crashrec::fixed(VERSION),
    };
    // SAFETY: jediny zapisovatel (ostatni CPU zastavene, viz fatal_stop).
    let a = unsafe { area_mut() }?;
    log::with_ring_unlocked(|first, second| {
        crashrec::write(a, &h, &[first, second]);
    });
    let (_, _, crc) = crashrec::parse(a)?;
    Some((h, crc))
}

/// Text zaznamu (pro obrazovku pady) po zapisu.
pub fn text() -> &'static [u8] {
    area().and_then(|a| crashrec::parse(a)).map_or(&[], |(_, t, _)| t)
}
