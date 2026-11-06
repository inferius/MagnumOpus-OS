//! Testy spolecneho typu chyb. Testy ACPI tabulek konkretni architektury
//! jsou v `arch/<cil>/tests.rs` (`arch::TESTS`).

use crate::error::Error;

pub const TESTS: &[(&str, fn())] = &[("chyby: stabilni kody a nazvy", error_codes)];

fn error_codes() {
    // Kody jsou soucast ABI syscallu, nesmi se menit.
    assert_eq!(Error::BadHandle.code(), -1);
    assert_eq!(Error::TimedOut.code(), -4);
    assert_eq!(Error::Corrupted.code(), -10);
    assert_eq!(Error::AccessDenied.name(), "ACCESS_DENIED");
}
