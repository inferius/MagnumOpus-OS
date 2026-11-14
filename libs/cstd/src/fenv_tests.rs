//! fenv na hostiteli (x86_64): skutecne registry FP tohoto vlakna.

use crate::fenv::*;

#[test]
fn fenv_header_matches() {
    let h = include_str!("../include/fenv.h");
    let sect = if cfg!(target_arch = "x86_64") { h.split("#elif").next().unwrap() } else { h.split("#elif").nth(1).unwrap() };
    for (n, v) in [
        ("FE_INVALID", FE_INVALID),
        ("FE_DIVBYZERO", FE_DIVBYZERO),
        ("FE_OVERFLOW", FE_OVERFLOW),
        ("FE_UNDERFLOW", FE_UNDERFLOW),
        ("FE_INEXACT", FE_INEXACT),
        ("FE_ALL_EXCEPT", FE_ALL_EXCEPT),
        ("FE_TONEAREST", FE_TONEAREST),
    ] {
        assert!(sect.contains(&format!("#define {n} {v}\n")), "{n}");
    }
    for (n, v) in [("FE_DOWNWARD", FE_DOWNWARD), ("FE_UPWARD", FE_UPWARD), ("FE_TOWARDZERO", FE_TOWARDZERO)] {
        assert!(sect.contains(&format!("#define {n} {v:#x}\n")), "{n}");
    }
    // fenv_t: glibc x86_64 32 B, aarch64 8 B.
    let want = if cfg!(target_arch = "x86_64") { 32 } else { 8 };
    assert_eq!(core::mem::size_of::<Env>(), want);
}

#[test]
fn rounding_and_flags() {
    let mut saved = Env::default();
    // SAFETY: platny ukazatel.
    unsafe { fegetenv(&mut saved) };
    assert_eq!(fesetround(FE_UPWARD as i32), 0);
    assert_eq!(fegetround(), FE_UPWARD as i32);
    let x = core::hint::black_box(1.0f64) / core::hint::black_box(3.0f64);
    assert_eq!(fesetround(FE_DOWNWARD as i32), 0);
    let y = core::hint::black_box(1.0f64) / core::hint::black_box(3.0f64);
    assert!(x > y, "zaokrouhleni nahoru vs dolu");
    assert_eq!(fesetround(12345), -1);
    feclearexcept(FE_ALL_EXCEPT as i32);
    assert_eq!(fetestexcept(FE_ALL_EXCEPT as i32), 0);
    core::hint::black_box(core::hint::black_box(1.0f64) / core::hint::black_box(0.0f64));
    assert_ne!(fetestexcept(FE_DIVBYZERO as i32), 0);
    feclearexcept(FE_DIVBYZERO as i32);
    assert_eq!(fetestexcept(FE_DIVBYZERO as i32), 0);
    feraiseexcept(FE_OVERFLOW as i32);
    assert_ne!(fetestexcept(FE_OVERFLOW as i32), 0);
    // feholdexcept smaze priznaky, feupdateenv je vrati spolu s ulozenymi.
    let mut held = Env::default();
    // SAFETY: platne ukazatele.
    unsafe {
        feholdexcept(&mut held);
        assert_eq!(fetestexcept(FE_ALL_EXCEPT as i32), 0);
        feraiseexcept(FE_INEXACT as i32);
        feupdateenv(&held);
    }
    assert_ne!(fetestexcept(FE_OVERFLOW as i32), 0);
    assert_ne!(fetestexcept(FE_INEXACT as i32), 0);
    // Vychozi prostredi.
    // SAFETY: FE_DFL_ENV.
    unsafe { fesetenv(usize::MAX as *const Env) };
    assert_eq!(fegetround(), FE_TONEAREST as i32);
    assert_eq!(fetestexcept(FE_ALL_EXCEPT as i32), 0);
    // SAFETY: platny ukazatel.
    unsafe { fesetenv(&saved) };
}
