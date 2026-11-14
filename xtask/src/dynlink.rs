//! Dynamicke linkovani v obrazu (`docs/cs/dynamicke-linkovani.md`):
//!
//! - zavadec `user/ld` jako static-PIE (`lib/ld.so` v initfs): vlastni
//!   volani cargo s `relocation-model=pie` (i core a alloc z build-std
//!   v PIC) a linkem `-pie --no-dynamic-linker`, vlastni adresar cilu,
//! - testovaci knihovny a programy v C (`user/dltest`) pres `cbuild`
//!   (`-fPIC`/`-fPIE`, link `ld.lld`).

use std::env;
use std::path::PathBuf;
use std::process::Command;

use cbuild::{Build, Link};

use crate::{Options, Result, profile_dir, target_dir, workspace_root};

/// Adresar cilu zavadece (jine priznaky nez ostatni programy).
fn ld_target_dir() -> PathBuf {
    target_dir().join("user-pie")
}

/// Prelozi zavadec; vraci cestu k `ld`.
pub fn build_ld(opts: &Options) -> Result<PathBuf> {
    let a = opts.arch;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let json = workspace_root().join("third_party/rust-std/targets").join(format!("{}.json", a.user_target));
    let mut cmd = Command::new(cargo);
    cmd.current_dir(workspace_root())
        .arg("build")
        .arg("--target")
        .arg(&json)
        .args(["-Zjson-target-spec", "-Zbuild-std=core,alloc", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--package", "ld"]);
    for krate in ["core", "alloc", "compiler_builtins"] {
        for (k, v) in [("opt-level", "3"), ("debug-assertions", "false"), ("overflow-checks", "false")] {
            cmd.arg("--config").arg(format!("profile.dev.package.{krate}.{k}={v}"));
        }
    }
    if opts.release {
        cmd.arg("--release");
    }
    // static-PIE: kod pozicne nezavisly, link bez zavadece; jen relativni
    // relokace (zavadec je provede sam, `user/ld/src/start.rs`). Relokace
    // v kodu (TEXTREL) = chyba linku.
    let sep = char::from(0x1F).to_string();
    let flags = [
        "-Crelocation-model=pie",
        "-Cforce-frame-pointers=yes",
        "-Clink-arg=-pie",
        "-Clink-arg=--no-dynamic-linker",
        "-Clink-arg=-ztext",
        "-Clink-arg=-znow",
        "-Clink-arg=-zmax-page-size=65536",
        "-Clink-arg=-zpack-relative-relocs",
        // Bez ladicich sekci (initfs); tabulka symbolu zustava (symbolizace).
        "-Clink-arg=--strip-debug",
    ]
    .join(&sep);
    cmd.env("CARGO_ENCODED_RUSTFLAGS", flags).arg("--target-dir").arg(ld_target_dir());
    let status = cmd.status()?;
    if !status.success() {
        return Err("build zavadece ld selhal".into());
    }
    Ok(ld_target_dir().join(a.user_target).join(profile_dir(opts)).join("ld"))
}

/// Testovaci moduly v C (`user/dltest`): (jmeno v initfs, obsah).
pub fn build_tests(opts: &Options) -> Result<Vec<(String, Vec<u8>)>> {
    let a = opts.arch;
    let src = workspace_root().join("user").join("dltest");
    let out = target_dir().join("dltest").join(a.name).join(profile_dir(opts));
    std::fs::create_dir_all(&out)?;
    let t = a.user_target;
    let lib2 = out.join("libdltest2.so.1");
    Build::new("dltest2").file(src.join("libdltest2.c")).link(t, &out, &lib2, &Link::shared("libdltest2.so.1"))?;
    let lib = out.join("libdltest.so.1");
    Build::new("dltest").file(src.join("libdltest.c")).link(t, &out, &lib, &Link::shared("libdltest.so.1").needs(&lib2))?;
    let none = out.join("libdlnone.so.1");
    Build::new("dlnone").file(src.join("none.c")).link(t, &out, &none, &Link::shared("libdlnone.so.1"))?;
    let prog = out.join("dltest");
    Build::new("dltest-main")
        .file(src.join("crt.c"))
        .file(src.join("main.c"))
        .link(t, &out, &prog, &Link::pie().needs(&lib).needs(&lib2))?;
    let missing = out.join("dltest-missing");
    Build::new("dltest-missing")
        .file(src.join("crt.c"))
        .file(src.join("missing.c"))
        .link(t, &out, &missing, &Link::pie().needs(&none))?;
    let read = |p: &PathBuf| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    // libdlnone se do obrazu zamerne nedava (test chybejici knihovny).
    Ok(vec![
        (String::from("lib/libdltest2.so.1"), read(&lib2)?),
        (String::from("lib/libdltest.so.1"), read(&lib)?),
        (String::from("dltest"), read(&prog)?),
        (String::from("dltest-missing"), read(&missing)?),
    ])
}
