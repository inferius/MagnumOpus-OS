//! Prevzate aplikace z crates.io prelozene pro cil magnumopus (overeni std
//! v praxi, docs/cs/rust-std.md, kapitola Porty).
//!
//! Kazdy port je pevna verze crate s binarkou; `cargo install --locked`
//! pouzije Cargo.lock z publikovaneho balicku (crates.io balicky jsou
//! nemenne, takze pevna verze = pevne zavislosti). Zdroje se stahuji do
//! mezipameti cargo jen poprve, sestaveni je v `target/ports/`.
//!
//! Zavislost, ktera nas system nezna (ma jen vetve `unix`/`windows`), se
//! opravi malym patchem z `third_party/ports/`: xtask zkopiruje jeji zdroj
//! z mezipameti cargo do `target/ports/src/`, aplikuje patch a preda ho
//! cargo jako `[patch.crates-io]`. Patche jsou kandidati na upstream.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::arch::Arch;
use crate::{Result, stdsys, target_dir, workspace_root};

/// Oprava zavislosti portu.
pub struct CratePatch {
    pub krate: &'static str,
    pub version: &'static str,
    /// Patch (`git diff`) v repozitari.
    pub file: &'static str,
}

/// Prevzata aplikace.
pub struct Port {
    /// Crate na crates.io.
    pub krate: &'static str,
    /// Pevna verze.
    pub version: &'static str,
    /// Binarky, ktere jdou do initfs (jmeno = soubor v initfs).
    pub bins: &'static [&'static str],
    pub patches: &'static [CratePatch],
}

pub const PORTS: &[Port] = &[Port {
    krate: "ripgrep",
    version: "15.2.0",
    bins: &["rg"],
    // Paralelni pruchod adresaru: obecna vetev vracela "unsupported
    // platform" misto zaznamu bez cisla inode.
    patches: &[CratePatch { krate: "ignore", version: "0.4.29", file: "third_party/ports/ignore-0.4.29.patch" }],
}];

fn cargo_home() -> PathBuf {
    std::env::var_os("CARGO_HOME").map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).unwrap_or_default();
        PathBuf::from(home).join(".cargo")
    })
}

/// Rozbaleny zdroj crate v mezipameti cargo (`registry/src/<index>/`).
fn registry_src(krate: &str, version: &str) -> Option<PathBuf> {
    let dir = format!("{krate}-{version}");
    std::fs::read_dir(cargo_home().join("registry/src"))
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path().join(&dir))
        .find(|p| p.join("Cargo.toml").exists())
}

/// Stahne crate do mezipameti cargo (pomocny manifest + `cargo fetch`).
fn fetch(krate: &str, version: &str) -> Result<()> {
    let dir = target_dir().join("ports/fetch").join(format!("{krate}-{version}"));
    std::fs::create_dir_all(dir.join("src"))?;
    std::fs::write(
        dir.join("Cargo.toml"),
        format!("[package]\nname = \"fetch\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[dependencies]\n{krate} = \"={version}\"\n\n[workspace]\n"),
    )?;
    std::fs::write(dir.join("src/lib.rs"), "")?;
    let ok = stdsys::cargo_cmd().arg("fetch").current_dir(&dir).status()?.success();
    if ok { Ok(()) } else { Err(format!("cargo fetch {krate} {version} selhal").into()) }
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

/// Zdroj zavislosti s patchem; vraci cestu pro `[patch.crates-io]`.
fn patched(p: &CratePatch) -> Result<PathBuf> {
    let patch_path = workspace_root().join(p.file);
    let patch = std::fs::read(&patch_path).map_err(|e| format!("{}: {e}", patch_path.display()))?;
    let mut h = DefaultHasher::new();
    patch.hash(&mut h);
    let fp = format!("{:016x}", h.finish());
    let dst = target_dir().join("ports/src").join(format!("{}-{}", p.krate, p.version));
    let stamp = dst.with_extension("stamp");
    if std::fs::read_to_string(&stamp).is_ok_and(|s| s == fp) && dst.join("Cargo.toml").exists() {
        return Ok(dst);
    }
    let src = match registry_src(p.krate, p.version) {
        Some(s) => s,
        None => {
            fetch(p.krate, p.version)?;
            registry_src(p.krate, p.version).ok_or_else(|| format!("{} {} neni v mezipameti cargo", p.krate, p.version))?
        }
    };
    if dst.exists() {
        std::fs::remove_dir_all(&dst)?;
    }
    copy_dir(&src, &dst)?;
    // Vlastni repozitar git v kopii (lezi uvnitr repozitare MagnumOpus).
    let git = |args: &[&str]| -> Result<()> {
        let ok = Command::new("git").args(args).current_dir(&dst).stdout(Stdio::null()).status()?.success();
        if ok { Ok(()) } else { Err(format!("git {} selhal ({})", args.join(" "), p.krate).into()) }
    };
    git(&["init", "-q"])?;
    git(&["config", "core.autocrlf", "false"])?;
    git(&["apply", "--whitespace=nowarn", &patch_path.to_string_lossy()])
        .map_err(|e| format!("{e}: patch {} nesedi na {} {}", p.file, p.krate, p.version))?;
    std::fs::write(&stamp, fp)?;
    Ok(dst)
}

/// Sestavi porty pro architekturu; vraci (jmeno v initfs, cesta binarky).
pub fn build(arch: &Arch) -> Result<Vec<(String, PathBuf)>> {
    // Spolecny sysroot std: zamek po celou dobu prekladu portu.
    let _lock = stdsys::lock_std()?;
    let flags = stdsys::app_rustflags_locked(arch)?;
    let base = target_dir().join("ports");
    let mut out = Vec::new();
    for p in PORTS {
        let root = base.join(arch.name).join(p.krate);
        println!("xtask: port {} {}", p.krate, p.version);
        let mut cmd = stdsys::cargo_cmd();
        cmd.args(["install", p.krate, "--version", p.version, "--locked", "--force", "--quiet"])
            .arg("--target")
            .arg(stdsys::target_json_path(arch))
            .arg("-Zjson-target-spec")
            .arg("--root")
            .arg(&root)
            .arg("--target-dir")
            .arg(base.join("build"))
            .env_remove("RUSTFLAGS")
            .env("CARGO_ENCODED_RUSTFLAGS", &flags);
        for cp in p.patches {
            let dir = patched(cp)?.display().to_string().replace('\\', "/");
            cmd.arg("--config").arg(format!("patch.crates-io.{}.path=\"{dir}\"", cp.krate));
        }
        if !cmd.status()?.success() {
            return Err(format!("port {} {} se nepodarilo prelozit", p.krate, p.version).into());
        }
        for b in p.bins {
            out.push((String::from(*b), root.join("bin").join(b)));
        }
    }
    Ok(out)
}
