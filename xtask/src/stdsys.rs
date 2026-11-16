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
//!
//! Kopie zdroju i sysroot jsou spolecne vsem behum xtask nad stejnym
//! `target/`. Soubezne behy (`cargo xtask test` vickrat najednou) se proto
//! stridaji pres zamek `target/std.lock` a sysroot se prepise jen pri zmene
//! obsahu - jinak si behy mazaly adresar pod rukama (os error 145).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::arch::Arch;
use crate::{Options, Result, target_dir, workspace_root};

/// Aplikace se std, ktere se pribali do initfs (jmeno crate = jmeno
/// souboru).
pub const STD_PROGRAMS: &[&str] = &["hello-std", "std-net", "std-proc", "std-http", "ctest", "gui-probe", "volfs", "std-exec", "nettools", "sshtools"];

/// Aplikace se std pod dalsim jmenem v initfs (jmeno, puvodni): stejny
/// program bez manifestu (`std-exec-bez`: test, ze program bez manifestu
/// sit nedostane) a sitove prikazy shellu (jeden program `nettools`,
/// chovani podle `argv[0]`, kazdy s vlastnim manifestem).
pub const STD_ALIASES: &[(&str, &str)] =
    &[("std-exec-bez", "std-exec"), ("ping", "nettools"), ("nc", "nettools"), ("resolve", "nettools"), ("ssh", "sshtools"), ("scp", "sshtools"), ("sftp", "sshtools")];

/// Knihovny se std pro sysroot C (`csysroot.rs`): `libc.a`.
pub const STD_LIBS: &[&str] = &["cstd-libc"];

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

/// Cil std jako PIC pro `libc.so` a programy PIE (`env: dyn`,
/// docs/cs/cstd.md, kapitola libc.so).
pub fn target_name_dyn(arch: &Arch) -> String {
    format!("{}-unknown-magnumopus-dyn", arch.name)
}

fn target_json_dyn(arch: &Arch) -> PathBuf {
    workspace_root().join("third_party/rust-std/targets").join(format!("{}.json", target_name_dyn(arch)))
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
    git(&["config", "core.autocrlf", "false"])?;
    // Vychozi commit = neupraveny rust-src: `cargo xtask std-patch` pak
    // ulozi upravy v kopii zpet do patche (`git diff`).
    git(&["add", "-A"])?;
    git(&["-c", "user.name=xtask", "-c", "user.email=xtask@localhost", "commit", "-q", "-m", "rust-src"])?;
    git(&["apply", "--whitespace=nowarn", &patch_path.to_string_lossy()])
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

/// Vylucny zamek pripravy std (zdroje, sysroot, aplikace) pro soubezne
/// behy xtask.
pub fn lock_std() -> Result<std::fs::File> {
    crate::lock_shared("std")
}

/// Ulozi upravy v kopii zdroju std (`target/std-src/<nightly>/library`)
/// zpet do `third_party/rust-std/std.patch` (vcetne novych souboru).
/// Postup pri zmene lepidla: upravit kopii, `cargo xtask std` (preklad),
/// `cargo xtask std-patch`, commit patche.
pub fn save_patch() -> Result<PathBuf> {
    // Kopie zdroju je spolecna soubeznym behum (viz `lock_std`).
    let _lock = lock_std()?;
    let library = target_dir().join("std-src").join(nightly()?).join("library");
    let run = |args: &[&str]| -> Result<Vec<u8>> {
        let out = Command::new("git").args(args).current_dir(&library).output()?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(format!("git {} selhal: {}", args.join(" "), String::from_utf8_lossy(&out.stderr)).into())
        }
    };
    run(&["rev-parse", "--verify", "HEAD"]).map_err(|_| "kopie std nema vychozi commit; spust `cargo xtask std` (pripravi ji znovu)")?;
    run(&["add", "-A"])?;
    let mut diff = run(&["diff", "--cached", "--binary", "HEAD"])?;
    // Cesta k osrt v patchi je relativni (viz `prepare_sources`).
    let osrt = workspace_root().join("libs/osrt").display().to_string().replace('\\', "/");
    let text = String::from_utf8(diff).map_err(|_| "patch std neni UTF-8")?;
    diff = text.replace(&format!("path = \"{osrt}\""), "path = \"../../../../../libs/osrt\"").into_bytes();
    let patch_path = workspace_root().join("third_party/rust-std/std.patch");
    std::fs::write(&patch_path, &diff)?;
    // Kopie odpovida novemu patchi: otisk, aby se nepripravovala znovu.
    std::fs::write(library.parent().unwrap().join("patch.stamp"), patch_fingerprint(&diff))?;
    Ok(patch_path)
}

/// Sestavi sysroot std pro architekturu (inkrementalne); vraci cestu
/// sysrootu pro `--sysroot`.
pub fn build_sysroot(arch: &Arch) -> Result<PathBuf> {
    let _lock = lock_std()?;
    build_sysroot_locked(arch)
}

fn build_sysroot_locked(arch: &Arch) -> Result<PathBuf> {
    build_sysroot_target(target_name(arch), target_json(arch))
}

/// Sysroot std jako PIC (druhy cil, `env: dyn`) pro `libc.so`.
pub fn build_sysroot_dyn_locked(arch: &Arch) -> Result<PathBuf> {
    build_sysroot_target(target_name_dyn(arch), target_json_dyn(arch))
}

fn build_sysroot_target(target: String, json: PathBuf) -> Result<PathBuf> {
    let library = prepare_sources()?;
    let mut cmd = Command::new(cargo());
    cmd.current_dir(&library)
        .args(["build", "--offline", "--release", "-p", "sysroot", "--no-default-features"])
        .args(["--features", "compiler-builtins-mem"])
        .arg("--target")
        .arg(&json)
        .arg("-Zjson-target-spec")
        .arg("--target-dir")
        .arg(target_dir().join("std-build"))
        .args(["--message-format", "json-render-diagnostics"])
        .env("RUSTC_BOOTSTRAP", "1")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", ["-Zunstable-options", "-Zforce-unstable-if-unmarked"].join("\u{1f}"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    println!("xtask: std pro {target}");
    let out = cmd.output()?;
    if !out.status.success() {
        // Hlaseni cargo (chyby prekladu, zamky) ze stdout v JSON i stderr.
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines().filter(|l| l.contains("\"level\":\"error\"") || !l.starts_with('{')) {
            eprintln!("{line}");
        }
        eprintln!("{}", String::from_utf8_lossy(&out.stderr));
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
    // Otisk obsahu knihoven: cargo zmenu sysrootu u aplikaci nesleduje,
    // proto jde otisk do jejich priznaku prekladu (`build_apps`).
    let mut h = DefaultHasher::new();
    libs.sort();
    let mut data = Vec::new();
    for lib in &libs {
        let bytes = std::fs::read(lib)?;
        lib.file_name().hash(&mut h);
        bytes.hash(&mut h);
        data.push((lib.file_name().unwrap().to_owned(), bytes));
    }
    let fp = format!("{:016x}", h.finish());
    // Stejny obsah = nic neprepisovat (soubory i casy zustanou).
    if std::fs::read_to_string(dir.join("fingerprint")).is_ok_and(|s| s == fp) {
        return Ok(sysroot);
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    for (name, bytes) in data {
        std::fs::write(dir.join(name), bytes)?;
    }
    std::fs::write(dir.join("fingerprint"), fp)?;
    Ok(sysroot)
}

/// Prelozi aplikace se std; vraci adresar s binarkami.
pub fn build_apps(programs: &[&str], opts: &Options) -> Result<PathBuf> {
    let arch = opts.arch;
    // Zamek drzi i preklad aplikaci: cte sysroot, ktery jiny beh muze menit.
    let _lock = lock_std()?;
    let flags = app_rustflags_locked(arch)?;
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

/// Cil JSON (cesta) pro `--target`.
pub fn target_json_path(arch: &Arch) -> PathBuf {
    target_json(arch)
}

/// Prikaz cargo (z promenne `CARGO`).
pub fn cargo_cmd() -> Command {
    Command::new(cargo())
}

/// Priznaky prekladu aplikace se std (`CARGO_ENCODED_RUSTFLAGS`):
/// sysroot (sestavi ho), linker skript a otisk sysrootu. Volajici drzi
/// `lock_std` po celou dobu prekladu (sysroot se cte az pri linkovani).
pub fn app_rustflags_locked(arch: &Arch) -> Result<String> {
    let sysroot = build_sysroot_locked(arch)?;
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
    Ok(flags)
}

/// Sestavi `libc.so` (crate `libs/cstd-so`, `cdylib` cstd + std + osrt nad
/// sysrootem PIC); vraci cestu k souboru. Volajici drzi `lock_std`.
pub fn build_libc_so_locked(arch: &Arch, opts: &Options) -> Result<PathBuf> {
    let sysroot = build_sysroot_dyn_locked(arch)?;
    let fingerprint = std::fs::read_to_string(
        sysroot.join("lib/rustlib").join(target_name_dyn(arch)).join("lib/fingerprint"),
    )?;
    let flags = [
        format!("--sysroot={}", sysroot.display()),
        String::from("-Zunstable-options"),
        // Pravidla sdilenych knihoven systemu (docs/cs/dynamicke-linkovani.md):
        // SONAME, BIND_NOW, plne RELRO, zadne TEXTREL, segmenty po 64 KiB.
        String::from("-Clink-arg=--soname=libc.so"),
        String::from("-Clink-arg=-znow"),
        String::from("-Clink-arg=-zrelro"),
        String::from("-Clink-arg=-ztext"),
        String::from("-Clink-arg=-zmax-page-size=65536"),
        String::from("-Clink-arg=--hash-style=both"),
        format!("--cfg=mo_sysroot=\"{fingerprint}\""),
        String::from("--check-cfg=cfg(mo_sysroot,values(any()))"),
    ]
    .join("\u{1f}");
    let dir = target_dir().join("std-dyn");
    // Symboly definovane v C (mem*, funkce long double) rustc do cdylib
    // neexportuje: druhy verzovaci skript je jmenuje vyslovne (presne jmeno
    // ma v lld prednost pred `local: *` skriptu rustc).
    std::fs::create_dir_all(&dir)?;
    let list = std::fs::read_to_string(workspace_root().join("libs/cstd-so/export-c.txt"))?;
    let mut ver = String::from("{\n  global:\n");
    for name in list.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        ver.push_str(&format!("    {name};\n"));
    }
    ver.push_str("};\n");
    let ver_path = dir.join(format!("export-c-{}.ver", arch.name));
    if std::fs::read_to_string(&ver_path).ok().as_deref() != Some(ver.as_str()) {
        std::fs::write(&ver_path, &ver)?;
    }
    // Otisk seznamu v priznacich: zmena jen seznamu (stejna cesta skriptu)
    // by jinak libc.so znovu neslinkovala a novy symbol by chybel.
    let ver_hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ver.hash(&mut h);
        h.finish()
    };
    let flags = format!(
        "{flags}\u{1f}-Clink-arg=--version-script={}\u{1f}--cfg=mo_export=\"{ver_hash:016x}\"\u{1f}--check-cfg=cfg(mo_export,values(any()))",
        ver_path.display()
    );
    let mut cmd = Command::new(cargo());
    cmd.current_dir(workspace_root())
        .args(["build", "--package", "cstd-so"])
        .arg("--target")
        .arg(target_json_dyn(arch))
        .arg("-Zjson-target-spec")
        .arg("--target-dir")
        .arg(&dir)
        .env_remove("RUSTFLAGS")
        .env("CARGO_ENCODED_RUSTFLAGS", flags);
    if opts.release {
        cmd.arg("--release");
    }
    if !cmd.status()?.success() {
        return Err("build libc.so (cstd-so) selhal".into());
    }
    Ok(dir.join(target_name_dyn(arch)).join(if opts.release { "release" } else { "debug" }).join("libc.so"))
}
