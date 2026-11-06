//! Kontrola a korekce posunu citace cyklu (x86: TSC) noveho CPU proti BSP.
//! Na architekturach se zarucene synchronnim citacem
//! (`arch::CYCLE_COUNTER_SYNCED`, napr. aarch64) se nepouziva.
//!
//! Hodiny jadra predpokladaji stejny citac na vsech CPU. Firmware ho
//! obvykle synchronizuje, ale neni to zaruceno. Mereni: BSP si poznamena
//! cas t0, da signal, AP odpovi svou hodnotou a BSP si poznamena t1. Posun
//! = hodnota AP - (t0 + t1) / 2, s nejistotou t1 - t0. Z nekolika kol se
//! bere to s nejmensi nejistotou. Posun vetsi nez nejistota se opravi
//! (pokud to CPU umi), jinak se aspon zaloguje.

use core::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};

use crate::{arch, log};

const ROUNDS: u32 = 8;
/// Jak dlouho AP ceka na mereni (pozde nabehnuvsi CPU se nesmi zaseknout).
const ANSWER_TIMEOUT_NS: u64 = 100_000_000;

static GO: AtomicU32 = AtomicU32::new(0);
static DONE: AtomicU32 = AtomicU32::new(0);
static AP_VALUE: AtomicU64 = AtomicU64::new(0);
static FIX: AtomicI64 = AtomicI64::new(0);
static FIX_READY: AtomicBool = AtomicBool::new(false);

/// Pripravi mereni pro dalsi AP (vola BSP pred jeho startem).
pub fn reset() {
    GO.store(0, Ordering::Release);
    DONE.store(0, Ordering::Release);
    FIX_READY.store(false, Ordering::Release);
}

/// Strana BSP: zmeri posun prave nastartovaneho AP a preda mu korekci.
pub fn measure_and_fix(hw_id: u32) {
    let mut best_window = u64::MAX;
    let mut best_offset = 0i64;
    for round in 1..=ROUNDS {
        let t0 = arch::cycle_counter();
        GO.store(round, Ordering::Release);
        while DONE.load(Ordering::Acquire) != round {
            core::hint::spin_loop();
        }
        let t1 = arch::cycle_counter();
        let window = t1.wrapping_sub(t0);
        if window < best_window {
            best_window = window;
            let mid = t0 + window / 2;
            best_offset = AP_VALUE.load(Ordering::Acquire).wrapping_sub(mid) as i64;
        }
    }
    let fix = if best_offset.unsigned_abs() > best_window {
        log::warn!(
            "citac cyklu CPU ({} {hw_id}) je posunuty o {best_offset} tiku proti BSP (nejistota {best_window}), opravuji",
            arch::HW_ID_NAME
        );
        -best_offset
    } else {
        log::debug!(
            "citac cyklu CPU ({} {hw_id}): posun {best_offset} tiku, v ramci nejistoty {best_window}",
            arch::HW_ID_NAME
        );
        0
    };
    FIX.store(fix, Ordering::Release);
    FIX_READY.store(true, Ordering::Release);
}

/// Strana AP: odpovida na mereni a aplikuje korekci.
pub fn answer() {
    let deadline = arch::now_ns().saturating_add(ANSWER_TIMEOUT_NS);
    for round in 1..=ROUNDS {
        while GO.load(Ordering::Acquire) != round {
            if arch::now_ns() > deadline {
                log::warn!("mereni posunu citace cyklu neprobehlo, pokracuji bez nej");
                return;
            }
            core::hint::spin_loop();
        }
        AP_VALUE.store(arch::cycle_counter(), Ordering::Release);
        DONE.store(round, Ordering::Release);
    }
    while !FIX_READY.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }
    let fix = FIX.load(Ordering::Acquire);
    if fix != 0 && !arch::adjust_cycle_counter(fix) {
        log::error!(
            "CPU neumi opravit citac cyklu (posun {} tiku): cas na tomto CPU bude posunuty",
            -fix
        );
    }
}
