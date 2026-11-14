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
    ("dynamicke linkovani: vlakna libos s TLS knihoven, dl_iterate_phdr, r_debug, fini", rust_program),
    ("dynamicke linkovani: libd sdili sablony knihoven mezi procesy", libd_shares),
    ("dynamicke linkovani: rezerva statickeho TLS z voleb spusteni (manifest)", tls_reserve),
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

fn rust_program(s: &Startup) -> TestResult {
    let code = run(s, "dltest-rs")?;
    // Bity viz user/dltest-rs/src/main.rs (E_*).
    check_eq!(
        code,
        0,
        "dltest-rs: 1 hlavni vlakno, 2 TLS vlaken, 4 tabulka zavadece, 8 dl_iterate_phdr, 16 r_debug, 32 fini, 64 vlakna, \
         128 dl* zaklad/dladdr, 256 pecet kodu/RELRO, 512 dlopen s TLS, 1024 line TLS ve vlaknech, 2048 chyba rezervy"
    );
    Ok(())
}

/// Dva behy `dltest`: druhy nevytvori zadnou novou sablonu (ld.so,
/// libdltest, libdltest2 uz libd ma), jen zasahy mezipameti.
fn libd_shares(s: &Startup) -> TestResult {
    run(s, "dltest")?;
    let a = libos::libd::stats().map_err(|e| fail("statistika libd", e))?;
    check_eq!(run(s, "dltest")? & 0xFF, 0, "druhy beh dltest");
    let b = libos::libd::stats().map_err(|e| fail("statistika libd", e))?;
    check_eq!(b.templates, a.templates, "zadna nova sablona pri druhem behu");
    check!(b.hits >= a.hits + 3, alloc::format!("zasahy mezipameti {} -> {}", a.hits, b.hits));
    Ok(())
}

/// `dltest-dl` nahraje pres `dlopen` knihovnu se 4 KiB TLS initial-exec:
/// s vychozi rezervou (2 KiB) odmitnuto s radou v `dlerror` (kod 1),
/// s rezervou 8 KiB z voleb spusteni (`[app] static_tls_reserve`) uspech.
fn tls_reserve(s: &Startup) -> TestResult {
    check_eq!(run(s, "dltest-dl")?, 1, "vychozi rezerva: odmitnuto s radou static_tls_reserve");
    let fs = Initfs::open(s).map_err(|e| fail("initfs", e))?;
    let image = fs.find("dltest-dl").ok_or("dltest-dl neni v initfs")?;
    let job = libos::job::current().map_err(|e| fail("job", e))?;
    let opts = libos::spawn::Options { static_tls_reserve: Some(8192), ..Default::default() };
    let (p, _ch) = libos::spawn::spawn_with(&job, "dltest-dl", image, Vec::new(), opts).map_err(|e| fail("spusteni", e))?;
    let code = p.wait_exit(deadline()).map_err(|e| fail("konec programu", e))?;
    check_eq!(code, 0, "rezerva 8 KiB: knihovna nahrana a TLS sedi");
    Ok(())
}
