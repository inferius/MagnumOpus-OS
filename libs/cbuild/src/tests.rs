//! Testy priznaku prekladu (na hostiteli).

use super::*;

#[test]
fn programy_maji_hard_float() {
    let x = target_flags("x86_64-unknown-magnumopus").unwrap();
    assert!(x.contains(&"-march=x86-64-v2") && !x.contains(&"-mgeneral-regs-only"));
    let a = target_flags("aarch64-unknown-magnumopus").unwrap();
    assert!(a.contains(&"-mstrict-align") && !a.contains(&"-mgeneral-regs-only"));
}

#[test]
fn znamy_cil_jadra_ma_priznaky_bez_fpu() {
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
    let f = b.command_flags("x86_64-unknown-magnumopus").unwrap();
    assert!(f.iter().any(|s| s == "-Werror"));
    assert!(f.iter().any(|s| s == "-DA=1"));
    assert!(f.iter().any(|s| s == "-Iinc"));
    let v = Build::new("y").trust(Trust::Vendored).command_flags("x86_64-unknown-magnumopus").unwrap();
    assert!(!v.iter().any(|s| s == "-Werror"));
}

#[test]
fn pic_prepne_fno_pic() {
    let b = Build::new("x");
    let f = b.flags_for("x86_64-unknown-magnumopus", crate::Code::Shared).unwrap();
    assert!(f.iter().any(|s| s == "-fPIC") && !f.iter().any(|s| s == "-fno-pic"));
    let f = b.flags_for("x86_64-unknown-magnumopus", crate::Code::Pie).unwrap();
    assert!(f.iter().any(|s| s == "-fPIE"));
    let f = b.flags_for("x86_64-unknown-magnumopus", crate::Code::Static).unwrap();
    assert!(f.iter().any(|s| s == "-fno-pic"));
}

#[test]
fn link_sdilena_a_pie() {
    let f = link_flags("aarch64-unknown-magnumopus", &Link::shared("libx.so.1").runpath("$ORIGIN")).unwrap();
    for x in ["-shared", "-soname=libx.so.1", "now", "relro", "text", "max-page-size=65536", "--enable-new-dtags", "-rpath=$ORIGIN", "aarch64elf"] {
        assert!(f.iter().any(|s| s == x), "{x}: {f:?}");
    }
    let f = link_flags("x86_64-unknown-magnumopus", &Link::pie()).unwrap();
    assert!(f.iter().any(|s| s == "-pie") && f.iter().any(|s| s == "--dynamic-linker=/system/lib/ld.so"));
    assert!(link_flags("x86_64-unknown-none", &Link::pie()).is_err(), "jadro sdilene knihovny nema");
}

/// Skutecny preklad a link (preskoci se bez clangu): knihovna se SONAME,
/// program PIE s PT_INTERP a DT_NEEDED na ni, obe architektury.
#[test]
fn link_skutecne_moduly() {
    if find_clang().is_err() {
        eprintln!("clang neni, test preskocen");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cbuild-link-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.c"), "int lib_value = 3;\nint lib_fn(int x) { return x + lib_value; }\n").unwrap();
    std::fs::write(dir.join("main.c"), "extern int lib_fn(int);\nint run(void) { return lib_fn(4); }\n").unwrap();
    for (target, machine) in [("x86_64-unknown-magnumopus", elf::Machine::X86_64), ("aarch64-unknown-magnumopus", elf::Machine::Aarch64)] {
        let out = dir.join(target);
        let lib = out.join("libt.so.1");
        std::fs::create_dir_all(&out).unwrap();
        Build::new("t").file(dir.join("lib.c")).link(target, &out, &lib, &Link::shared("libt.so.1")).unwrap();
        let pie = out.join("prog");
        Build::new("prog").file(dir.join("main.c")).link(target, &out, &pie, &Link::pie().needs(&lib).runpath("$ORIGIN")).unwrap();
        let data = std::fs::read(&lib).unwrap();
        let e = elf::ElfFile::parse_object(&data, machine).unwrap();
        let d = e.dynamic().unwrap().unwrap();
        assert_eq!(e.dyn_str(&d, d.soname.unwrap()).unwrap(), "libt.so.1");
        assert!(d.bind_now());
        assert!(e.symbols(&d).unwrap().lookup("lib_fn").unwrap().is_some());
        let data = std::fs::read(&pie).unwrap();
        let e = elf::ElfFile::parse_object(&data, machine).unwrap();
        assert_eq!(e.interp().unwrap(), Some(INTERP));
        let d = e.dynamic().unwrap().unwrap();
        assert!(d.pie());
        let needed: Vec<&str> = d.needed().iter().map(|&o| e.dyn_str(&d, o).unwrap()).collect();
        assert_eq!(needed, ["libt.so.1"]);
        assert_eq!(e.dyn_str(&d, d.runpath.unwrap()).unwrap(), "$ORIGIN");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
