//! Dynamicke linkovani (DL-2, `docs/cs/dynamicke-linkovani.md`): program PIE
//! v C (`user/dltest`) se zavadecem `ld.so` a dvema knihovnami z initfs.

use alloc::vec::Vec;
use core::time::Duration;

use libos::initfs::Initfs;
use libos::{Startup, time};

use super::{Test, TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[Test] = &[
    ("dynamicke linkovani: PIE s knihovnami (relokace, TLS, inicializatory)", program_with_libraries),
    ("dynamicke linkovani: ASLR - dva behy, ruzne baze", aslr_differs),
    ("dynamicke linkovani: chybejici knihovna -> kod 127", missing_library),
];

fn deadline() -> Option<u64> {
    Some(time::deadline_after(Duration::from_secs(20)))
}

/// Spusti program z initfs a vrati jeho navratovy kod.
fn run(s: &Startup, name: &str) -> Result<i64, alloc::string::String> {
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find(name).ok_or_else(|| alloc::format!("{name} neni v initfs"))?;
    let (p, _ch) = libos::spawn(name, image, Vec::new()).map_err(|e| fail("spusteni", e))?;
    p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))
}

fn program_with_libraries(s: &Startup) -> TestResult {
    let code = run(s, "dltest")?;
    // Bity 0-7: chyby v programu (viz user/dltest/main.c).
    check_eq!(code & 0xFF, 0, "kontroly programu (1 volani, 2 ukazatel na funkci, 4 inicializator, 8/16 TLS knihoven, 32/64 TLS programu)");
    Ok(())
}

fn aslr_differs(s: &Startup) -> TestResult {
    let a = run(s, "dltest")?;
    let b = run(s, "dltest")?;
    check!(a & 0xFF == 0 && b & 0xFF == 0, "oba behy bez chyby");
    check!(a >> 8 != b >> 8, alloc::format!("stejna baze programu v obou behech ({:#x})", a >> 8));
    Ok(())
}

fn missing_library(s: &Startup) -> TestResult {
    check_eq!(run(s, "dltest-missing")?, abi::auxv::EXIT_LOAD_FAILED, "kod zavadece pri chybejici knihovne");
    Ok(())
}
