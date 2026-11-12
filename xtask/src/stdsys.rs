//! Rust `std` pro aplikace (cil `*-unknown-magnumopus`, docs/cs/rust-std.md).
//!
//! 1. Zdroje: kopie `library/` z komponenty `rust-src` pripnuteho nightly do
//!    `target/std-src/<nightly>/library` a na ni patch lepidla
//!    `third_party/rust-std/std.patch`. Kopie se pripravi znovu, kdyz se
//!    zmeni patch (otisk v souboru vedle kopie).
//! 2. Sysroot: `cargo build -p sysroot` v kopii (offline, zavislosti std
//!    jsou ve `vendor/` komponenty rust-src) pro JSON cil
//!    z `third_party/rust-std/targets`; `.rlib` a `.rmeta` z vystupu cargo
//!    se zkopiruji do `target/sysroot/<nightly>/lib/rustlib/<cil>/lib`.
//!    Cargo prekresli jen to, co se zmenilo (osrt, libos, patch).
//! 3. Aplikace: bezny `cargo build --target <json>` se `--sysroot`
//!    a linker skriptem `user/user-std.ld`.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::arch::Arch;
use crate::{Options, Result, target_dir, workspace_root};

/// Aplikace se std, ktere se pribali do initfs (jmeno crate = jmeno
/// souboru).
pub const STD_PROGRAMS: &[&str] = &["hello-std"];

/// Pripnuty nightly z `rust-toolchain.toml` (cesty std-src a sysrootu).
fn nightly() -> Result<String> {
    let text = std::fs::read_to_string(workspace_root().join("rust-toolchain.toml"))?;
    text.lines()
        .find_map(|l| l.trim().strip_prefix("channel").map(|r| r.trim_start_matches([' ', '=']).trim().trim_matches('"').to_string()))
        .ok_or_else(|| "rust-toolchain.toml: chybi channel".into())
}

/// Cil std pro architekturu (jmeno = jmeno JSON souboru).
pub fn target_name(arch: &Arch) -> String {
    format!("{}-unknown-magnumopus", arch.name)
}

fn target_json(arch: &Arch) -> PathBuf {
    workspace_root().join("third_party/rust-std/targets").join(format!("{}.json", target_name(arch)))
}

/// Sysroot pro aplikace se std (spolecny pro obe architektury).
pub fn sysroot_dir() -> Result<PathBuf> {
    Ok(target_dir().join("sysroot").join(nightly()?))
}

/// `library/` komponenty rust-src aktualniho toolchainu.
fn rust_src_library() -> Result<PathBuf> {
    let out = Command::new(rustc()).args(["--print", "sysroot"]).output()?;
    if !out.status.success() {
        return Err("rustc --print sysroot selhal".into());
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    let lib = sysroot.join("lib/rustlib/src/rust/library");
    if !lib.join("std/Cargo.toml").exists() {
        return Err(format!("chybi rust-src ({}); `rustup component add rust-src`", lib.display()).into());
    }
    Ok(lib)
}

fn rustc() -> std::ffi::OsString {
    std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into())
}

fn cargo() -> std::ffi::OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// Otisk patche a cilu (kdyz se zmeni, kopie zdroju se pripravi znovu).
fn patch_fingerprint(patch: &[u8]) -> String {
    let mut h = DefaultHasher::new();
    patch.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dst = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &dst)?;
        } else {
            std::fs::copy(entry.path(), &dst)?;
        }
    }
    Ok(())
}

/// Pripravi zdroje std s lepidlem; vraci cestu ke kopii `library/`.
pub fn prepare_sources() -> Result<PathBuf> {
    let patch_path = workspace_root().join("third_party/rust-std/std.patch");
    let patch = std::fs::read(&patch_path).map_err(|e| format!("{}: {e}", patch_path.display()))?;
    let root = target_dir().join("std-src").join(nightly()?);
    let library = root.join("library");
    let stamp = root.join("patch.stamp");
    let fp = patch_fingerprint(&patch);
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s == fp) && library.join("std/Cargo.toml").exists() {
        return Ok(library);
    }
    println!("xtask: pripravuji zdroje std ({})", library.display());
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    copy_dir(&rust_src_library()?, &library)?;
    // Vlastni repozitar git v kopii: `git apply` pak nepracuje vuci
    // repozitari MagnumOpus, ve kterem kopie lezi.
    let git = |args: &[&str]| -> Result<()> {
        let st = Command::new("git").args(args).current_dir(&library).stdout(Stdio::null()).status()?;
        if st.success() { Ok(()) } else { Err(format!("git {} selhal", args.join(" ")).into()) }
    };
    git(&["init", "-q"])?;
    git(&["-c", "core.autocrlf=false", "apply", "--whitespace=nowarn", &patch_path.to_string_lossy()])
        .map_err(|e| format!("{e}: patch std nesedi na rust-src tohoto nightly (aktualizace toolchainu = obnovit patch)"))?;
    // Cesta k osrt v patchi je relativni vuci target/ v repozitari; s jinym
    // CARGO_TARGET_DIR plati absolutni.
    let std_toml = library.join("std/Cargo.toml");
    let osrt = workspace_root().join("libs/osrt");
    let text = std::fs::read_to_string(&std_toml)?.replace(
        "path = \"../../../../../libs/osrt\"",
        &format!("path = \"{}\"", osrt.display().to_string().replace('\\', "/")),
    );
    std::fs::write(&std_toml, text)?;
    std::fs::write(&stamp, fp)?;
    Ok(library)
}

/// Sestavi sysroot std pro architekturu (inkrementalne); vraci cestu
/// sysrootu pro `--sysroot`.
pub fn build_sysroot(arch: &Arch) -> Result<PathBuf> {
    let library = prepare_sources()?;
    let target = target_name(arch);
    let mut cmd = Command::new(cargo());
    cmd.current_dir(&library)
        .args(["build", "--offline", "--release", "-p", "sysroot", "--no-default-features"])
        .args(["--features", "compiler-builtins-mem"])
        .arg("--target")
        .arg(target_json(arch))
        .arg("-Zjson-target-spec")
        .arg("--target-dir")
        .arg(target_dir().join("std-build"))
        .args(["--message-format", "json-render-diagnostics"])
        .env("RUSTC_BOOTSTRAP", "1")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", ["-Zunstable-options", "-Zforce-unstable-if-unmarked"].join("\u{1f}"))
        .stdout(Stdio::piped());
    println!("xtask: std pro {target}");
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(format!("sestaveni std pro {target} selhalo").into());
    }
    // Soubory knihoven z hlaseni cargo (compiler-artifact, pole filenames).
    let mut libs = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !line.contains("\"reason\":\"compiler-artifact\"") {
            continue;
        }
        let Some(start) = line.find("\"filenames\":[") else { continue };
        let rest = &line[start + 13..];
        let Some(end) = rest.find(']') else { continue };
        for f in rest[..end].split(',') {
            let f = f.trim().trim_matches('"').replace("\\\\", "\\");
            if f.ends_with(".rlib") || f.ends_with(".rmeta") {
                libs.push(PathBuf::from(f));
            }
        }
    }
    if !libs.iter().any(|p| p.file_name().is_some_and(|n| n == "libstd.rlib")) {
        return Err("sestaveni std: chybi libstd.rlib v hlaseni cargo".into());
    }
    let sysroot = sysroot_dir()?;
    let dir = sysroot.join("lib/rustlib").join(&target).join("lib");
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    // Otisk obsahu knihoven: cargo zmenu sysrootu u aplikaci nesleduje,
    // proto jde otisk do jejich priznaku prekladu (`build_apps`).
    let mut h = DefaultHasher::new();
    libs.sort();
    for lib in &libs {
        let data = std::fs::read(lib)?;
        lib.file_name().hash(&mut h);
        data.hash(&mut h);
        std::fs::write(dir.join(lib.file_name().unwrap()), data)?;
    }
    std::fs::write(dir.join("fingerprint"), format!("{:016x}", h.finish()))?;
    Ok(sysroot)
}

/// Prelozi aplikace se std; vraci adresar s binarkami.
pub fn build_apps(programs: &[&str], opts: &Options) -> Result<PathBuf> {
    let arch = opts.arch;
    let sysroot = build_sysroot(arch)?;
    let script = workspace_root().join("user/user-std.ld");
    let fingerprint = std::fs::read_to_string(
        sysroot.join("lib/rustlib").join(target_name(arch)).join("lib/fingerprint"),
    )?;
    let flags = [
        format!("--sysroot={}", sysroot.display()),
        format!("-Clink-arg=-T{}", script.display()),
        String::from("-Zunstable-options"),
        // Zmena std/osrt = novy otisk = aplikace se prelozi znovu.
        format!("--cfg=mo_sysroot=\"{fingerprint}\""),
        String::from("--check-cfg=cfg(mo_sysroot,values(any()))"),
    ]
    .join("\u{1f}");
    let apps_dir = target_dir().join("std-apps");
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace_root()).arg("build").arg("--target").arg(target_json(arch)).arg("-Zjson-target-spec");
    for p in programs {
        cmd.args(["--package", p]);
    }
    if opts.release {
        cmd.arg("--release");
    }
    cmd.arg("--target-dir")
        .arg(&apps_dir)
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", flags);
    if !cmd.status()?.success() {
        return Err(format!("build aplikaci se std ({}) selhal", programs.join(", ")).into());
    }
    Ok(apps_dir.join(target_name(arch)).join(if opts.release { "release" } else { "debug" }))
}
