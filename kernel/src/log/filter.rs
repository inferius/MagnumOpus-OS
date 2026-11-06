//! Filtrovani zaznamu za behu: globalni uroven a prepsani pro moduly.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use super::Level;
use crate::sync::SpinLock;

const DEFAULT_LEVEL: Level = if cfg!(debug_assertions) {
    Level::Debug
} else {
    Level::Info
};

static LEVEL: AtomicU8 = AtomicU8::new(DEFAULT_LEVEL as u8);
static CONSOLE_LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Nejvyssi uroven ze vsech prepsani (rychla kontrola bez zamku).
static OVERRIDE_MAX: AtomicU8 = AtomicU8::new(0);
static HAS_OVERRIDES: AtomicBool = AtomicBool::new(false);

const MAX_OVERRIDES: usize = 16;
static OVERRIDES: SpinLock<[Option<(&'static str, Level)>; MAX_OVERRIDES]> =
    SpinLock::new([None; MAX_OVERRIDES]);

/// Globalni uroven logovani.
#[allow(
    dead_code,
    reason = "API pro nastaveni logu (prikazova radka jadra, syscally)"
)]
pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// Uroven pro modul `target` a jeho podmoduly (bez prefixu `kernel::`,
/// napr. `"sched"` nebo `"arch::x86_64::apic"`). Ma prednost pred globalni.
pub fn set_target_level(target: &'static str, level: Level) {
    let mut overrides = OVERRIDES.lock();
    let slot = overrides
        .iter()
        .position(|o| o.is_some_and(|(t, _)| t == target))
        .or_else(|| overrides.iter().position(Option::is_none));
    if let Some(i) = slot {
        overrides[i] = Some((target, level));
    } else {
        super::warn!("prilis mnoho prepsani urovne logu, {target} ignorovan");
        return;
    }
    let max = overrides
        .iter()
        .flatten()
        .map(|&(_, l)| l as u8)
        .max()
        .unwrap_or(0);
    OVERRIDE_MAX.store(max, Ordering::Relaxed);
    HAS_OVERRIDES.store(true, Ordering::Release);
}

/// Zrusi prepsani urovne pro modul `target`.
pub fn clear_target_level(target: &str) {
    let mut overrides = OVERRIDES.lock();
    for o in overrides.iter_mut() {
        if o.is_some_and(|(t, _)| t == target) {
            *o = None;
        }
    }
    let max = overrides.iter().flatten().map(|&(_, l)| l as u8).max();
    OVERRIDE_MAX.store(max.unwrap_or(0), Ordering::Relaxed);
    HAS_OVERRIDES.store(max.is_some(), Ordering::Release);
}

/// Do jake urovne se zaznamy vypisuji i do framebufferu.
#[allow(
    dead_code,
    reason = "API pro nastaveni logu (prikazova radka jadra, syscally)"
)]
pub fn set_console_level(level: Option<Level>) {
    CONSOLE_LEVEL.store(level.map_or(0, |l| l as u8), Ordering::Relaxed);
}

pub fn console_level() -> Level {
    Level::from_u8(CONSOLE_LEVEL.load(Ordering::Relaxed)).unwrap_or(Level::Error)
}

fn matches(target: &str, prefix: &str) -> bool {
    target == prefix
        || target
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with("::"))
}

pub(super) fn enabled(level: Level, target: &str) -> bool {
    let global = LEVEL.load(Ordering::Relaxed);
    if !HAS_OVERRIDES.load(Ordering::Acquire) {
        return level as u8 <= global;
    }
    if level as u8 > global.max(OVERRIDE_MAX.load(Ordering::Relaxed)) {
        return false;
    }
    // Nejdelsi shodny prefix vyhrava. try_lock: kdyz zamek drzi nekdo jiny
    // (napr. logovani z preruseni behem `set_target_level`), plati globalni.
    let Some(overrides) = OVERRIDES.try_lock() else {
        return level as u8 <= global;
    };
    let specific = overrides
        .iter()
        .flatten()
        .filter(|(prefix, _)| matches(target, prefix))
        .max_by_key(|(prefix, _)| prefix.len());
    match specific {
        Some(&(_, l)) => level <= l,
        None => level as u8 <= global,
    }
}
