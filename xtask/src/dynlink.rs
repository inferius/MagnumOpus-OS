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

/// Prelozi zavadec; vraci cestu k `ld`. static-PIE: kod pozicne
/// nezavisly, link bez zavadece; jen relativni relokace (zavadec je
/// provede sam, `user/ld/src/start.rs`).
pub fn build_ld(opts: &Options) -> Result<PathBuf> {
    build_pie(opts, "ld", "user-pie", &["-Clink-arg=--no-dynamic-linker"])
}

/// Testovaci program v Rustu (libos) jako PIE se zavadecem a knihovnami
/// `user/dltest` (DL-3: vlakna s TLS knihoven). Bezne programy v Rustu
/// zustavaji staticke.
pub fn build_rust_test(opts: &Options, libs: &[PathBuf]) -> Result<PathBuf> {
    let mut args: Vec<String> = vec![
        format!("-Clink-arg=--dynamic-linker={}", cbuild::INTERP),
        String::from("-Clink-arg=--allow-shlib-undefined"),
    ];
    for l in libs {
        args.push(format!("-Clink-arg={}", l.display()));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    build_pie(opts, "dltest-rs", "user-pie-test", &refs)
}

/// Program v Rustu jako PIE (`relocation-model=pie`, core a alloc z
/// build-std v PIC, link `-pie -z now -z relro`). Vlastni adresar cilu:
/// jine priznaky nez ostatni programy (jinak by se vse prekladalo znovu).
fn build_pie(opts: &Options, package: &str, dir: &str, extra: &[&str]) -> Result<PathBuf> {
    let a = opts.arch;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let json = workspace_root().join("third_party/rust-std/targets").join(format!("{}.json", a.user_target));
    let mut cmd = Command::new(cargo);
    cmd.current_dir(workspace_root())
        .arg("build")
        .arg("--target")
        .arg(&json)
        .args(["-Zjson-target-spec", "-Zbuild-std=core,alloc", "-Zbuild-std-features=compiler-builtins-mem"])
        .args(["--package", package]);
    for krate in ["core", "alloc", "compiler_builtins"] {
        for (k, v) in [("opt-level", "3"), ("debug-assertions", "false"), ("overflow-checks", "false")] {
            cmd.arg("--config").arg(format!("profile.dev.package.{krate}.{k}={v}"));
        }
    }
    if opts.release {
        cmd.arg("--release");
    }
    let sep = char::from(0x1F).to_string();
    let mut flags: Vec<&str> = vec![
        "-Crelocation-model=pie",
        "-Cforce-frame-pointers=yes",
        "-Clink-arg=-pie",
        // Relokace v kodu (TEXTREL) = chyba linku (W^X).
        "-Clink-arg=-ztext",
        "-Clink-arg=-znow",
        "-Clink-arg=-zrelro",
        "-Clink-arg=-zmax-page-size=65536",
        "-Clink-arg=-zpack-relative-relocs",
        // Bez ladicich sekci (initfs); tabulka symbolu zustava (symbolizace).
        "-Clink-arg=--strip-debug",
    ];
    flags.extend_from_slice(extra);
    let target = target_dir().join(dir);
    cmd.env("CARGO_ENCODED_RUSTFLAGS", flags.join(&sep)).arg("--target-dir").arg(&target);
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("build {package} (PIE) selhal").into());
    }
    Ok(target.join(a.user_target).join(profile_dir(opts)).join(package))
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
    // DL-4b: knihovny pro dlopen (dynamicke TLS, initial-exec do rezervy,
    // prilis velke initial-exec) a program pro test rezervy.
    let plug = out.join("libdlplug.so.1");
    let mut b = Build::new("dlplug").file(src.join("plug.c")).file(src.join("plug_gd.c"));
    if a.name == "x86_64" {
        // TLSDESC i na x86_64 (vychozi je __tls_get_addr; plug_gd.c ho
        // pouzije stejne, model je na jednotku - tady pro obe).
        b = b.flag("-mtls-dialect=gnu2");
    }
    b.link(t, &out, &plug, &Link::shared("libdlplug.so.1").needs(&lib2))?;
    let plug_gd = out.join("libdlplug-gd.so.1");
    Build::new("dlplug-gd")
        .file(src.join("plug.c"))
        .file(src.join("plug_gd.c"))
        .link(t, &out, &plug_gd, &Link::shared("libdlplug-gd.so.1").needs(&lib2))?;
    let stat = out.join("libdlstatic.so.1");
    Build::new("dlstatic")
        .file(src.join("static.c"))
        .flag("-ftls-model=initial-exec")
        .link(t, &out, &stat, &Link::shared("libdlstatic.so.1").needs(&lib2))?;
    let big = out.join("libdlbig.so.1");
    Build::new("dlbig")
        .file(src.join("big.c"))
        .flag("-ftls-model=initial-exec")
        .link(t, &out, &big, &Link::shared("libdlbig.so.1").needs(&lib2))?;
    let prog_dl = out.join("dltest-dl");
    Build::new("dltest-dl")
        .file(src.join("crt.c"))
        .file(src.join("main_dl.c"))
        .link(t, &out, &prog_dl, &Link::pie().needs(&lib2))?;
    // DL-4c: program a knihovny ze souboru (test je zkopiruje na disk do
    // /dl4c/{bin,lib}; v initfs mimo lib/, takze je libd podle jmena nenajde).
    let orig2 = out.join("libdlorigin2.so.1");
    Build::new("dlorigin2").file(src.join("origin2.c")).link(t, &out, &orig2, &Link::shared("libdlorigin2.so.1").needs(&lib2))?;
    let orig = out.join("libdlorigin.so.1");
    Build::new("dlorigin")
        .file(src.join("origin.c"))
        .link(t, &out, &orig, &Link::shared("libdlorigin.so.1").needs(&orig2).needs(&lib2).runpath("$ORIGIN"))?;
    let orig3 = out.join("libdlorigin3.so.1");
    Build::new("dlorigin3")
        .file(src.join("origin3.c"))
        .link(t, &out, &orig3, &Link::shared("libdlorigin3.so.1").needs(&orig2).needs(&lib2).runpath("$ORIGIN"))?;
    let prog_origin = out.join("dltest-origin");
    Build::new("dltest-origin")
        .file(src.join("crt.c"))
        .file(src.join("main_origin.c"))
        .link(t, &out, &prog_origin, &Link::pie().needs(&orig).needs(&lib2).runpath("$ORIGIN/../lib"))?;
    let read = |p: &PathBuf| std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()));
    let rust = build_rust_test(opts, &[lib.clone(), lib2.clone()])?;
    // libdlnone se do obrazu zamerne nedava (test chybejici knihovny).
    Ok(vec![
        (String::from("dltest-rs"), read(&rust)?),
        (String::from("lib/libdltest2.so.1"), read(&lib2)?),
        (String::from("lib/libdltest.so.1"), read(&lib)?),
        (String::from("dltest"), read(&prog)?),
        (String::from("dltest-missing"), read(&missing)?),
        (String::from("lib/libdlplug.so.1"), read(&plug)?),
        (String::from("lib/libdlplug-gd.so.1"), read(&plug_gd)?),
        (String::from("lib/libdlstatic.so.1"), read(&stat)?),
        (String::from("lib/libdlbig.so.1"), read(&big)?),
        (String::from("dltest-dl"), read(&prog_dl)?),
        (String::from("dl4c/bin/dltest-origin"), read(&prog_origin)?),
        (String::from("dl4c/lib/libdlorigin.so.1"), read(&orig)?),
        (String::from("dl4c/lib/libdlorigin2.so.1"), read(&orig2)?),
        (String::from("dl4c/lib/libdlorigin3.so.1"), read(&orig3)?),
    ])
}

/// Test libc.so (docs/cs/cstd.md): program PIE v C `cdyn` (crt1.c ze
/// sysrootu C + main.c) a plugin `libcplug.so`, oba nad `libc.so`.
/// Vraci soubory do initfs.
pub fn build_cdyn(opts: &Options, libc_so: &std::path::Path, sysroot: &std::path::Path) -> Result<Vec<(String, Vec<u8>)>> {
    let builtins = sysroot.join("lib/libbuiltins.a");
    let a = opts.arch;
    let t = a.user_target;
    let src = workspace_root().join("user").join("cdyn");
    let out = target_dir().join("cdyn").join(a.name).join(profile_dir(opts));
    std::fs::create_dir_all(&out)?;
    let inc = |b: Build| b.include(workspace_root().join("libs/cstd/include")).include(workspace_root().join("libs/cstd-core/include"));
    let plug = out.join("libcplug.so");
    inc(Build::new("cplug")).file(src.join("plug.c")).link(t, &out, &plug, &Link::shared("libcplug.so").needs(libc_so).object(&builtins))?;
    let prog = out.join("cdyn");
    inc(Build::new("cdyn"))
        .file(workspace_root().join("libs/cstd/crt/crt1.c"))
        .file(src.join("main.c"))
        .link(t, &out, &prog, &Link::pie().needs(libc_so).object(&builtins))?;
    Ok(vec![
        (String::from("cdyn"), std::fs::read(&prog)?),
        (String::from("lib/libcplug.so"), std::fs::read(&plug)?),
        (String::from("lib/libc.so"), std::fs::read(libc_so)?),
    ])
}

/// Proces dekoderu medii `ffdec` (user/ffdec, docs/cs/multimedia.md):
/// program PIE v C nad `libc.so` a knihovnami FFmpeg z predpony portu
/// (`prefix/lib/libav*.so`, hlavicky `prefix/include`), rozhrani jadra
/// `magnumopus/sys.h` (libs/mosys v libc.so).
pub fn build_ffdec(opts: &Options, libc_so: &std::path::Path, sysroot: &std::path::Path, prefix: &std::path::Path) -> Result<(String, Vec<u8>)> {
    let builtins = sysroot.join("lib/libbuiltins.a");
    let a = opts.arch;
    let src = workspace_root().join("user").join("ffdec").join("src");
    let out = target_dir().join("ffdec").join(a.name).join(profile_dir(opts));
    std::fs::create_dir_all(&out)?;
    let lib = prefix.join("lib");
    let mut link = Link::pie().needs(libc_so).object(&builtins);
    for l in ["libavformat.so", "libavcodec.so", "libswscale.so", "libswresample.so", "libavutil.so"] {
        link = link.needs(lib.join(l));
    }
    let prog = out.join("ffdec");
    Build::new("ffdec")
        .include(workspace_root().join("libs/cstd/include"))
        .include(workspace_root().join("libs/cstd-core/include"))
        .include(workspace_root().join("libs/mosys/include"))
        .include(prefix.join("include"))
        .flag("-O2")
        .file(workspace_root().join("libs/cstd/crt/crt1.c"))
        .file(src.join("proto.c"))
        .file(src.join("ffdec.c"))
        .link(a.user_target, &out, &prog, &link)?;
    Ok((String::from("ffdec"), std::fs::read(&prog)?))
}
