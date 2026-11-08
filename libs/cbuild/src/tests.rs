//! Testy priznaku prekladu (na hostiteli).

use super::*;

#[test]
fn znamy_cil_ma_priznaky_bez_fpu() {
    for t in ["x86_64-unknown-none", "aarch64-unknown-none-softfloat"] {
        let f = target_flags(t).unwrap();
        assert!(f.contains(&"-mgeneral-regs-only"), "{t}");
    }
    assert!(target_flags("aarch64-unknown-none-softfloat").unwrap().contains(&"-mstrict-align"));
}

#[test]
fn neznamy_cil_je_chyba() {
    assert!(target_flags("x86_64-pc-windows-msvc").is_err());
}

#[test]
fn spolecne_priznaky_obsahuji_ochrany() {
    for f in ["-fstack-protector-strong", "-mstack-protector-guard=global", "-ftrivial-auto-var-init=zero", "-ffreestanding", "-nostdlibinc"] {
        assert!(COMMON_FLAGS.contains(&f), "{f}");
    }
}

#[test]
fn vlastni_kod_ma_varovani_jako_chyby() {
    let b = Build::new("x").define("A", Some("1")).include("inc");
    let f = b.command_flags("x86_64-unknown-none").unwrap();
    assert!(f.iter().any(|s| s == "-Werror"));
    assert!(f.iter().any(|s| s == "-DA=1"));
    assert!(f.iter().any(|s| s == "-Iinc"));
    let v = Build::new("y").trust(Trust::Vendored).command_flags("x86_64-unknown-none").unwrap();
    assert!(!v.iter().any(|s| s == "-Werror"));
}
