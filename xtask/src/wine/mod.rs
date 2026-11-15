//! Windows vrstva: sestaveni Wine 11 pro MagnumOpus (docs/cs/windows-vrstva.md,
//! etapa W0). `cargo xtask wine [--keep-going]`.
//!
//! Wine se stavi ve WSL (rozhodnuti Q12 A): `configure` a nastroje Wine chteji
//! Unix hostitele. DLL jako PE preklada clang 23 (`--with-mingw=clang`),
//! unixovou cast obal `tools/wsl/mos-cc` nad sysrootem C (PIE a sdilene
//! knihovny nad `libc.so`). Archiv je pripnuty (HTTPS, SHA-256), zaplaty
//! v `third_party/wine/`. Vysledek v `target/wine/<arch>/`.
//!
//! Jen x86_64 (rozhodnuti Q11 A, aarch64 = zdokumentovane omezeni).

mod wsl;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use crate::arch::Arch;
use crate::cports::{Build, CPort};
use crate::{Result, target_dir, workspace_root};

/// Wine 11.0 (stabilni rada). SHA-256 spocteny z archivu overeneho proti
/// podepsanemu seznamu SHA-512 na dl.winehq.org (`sha512sums.asc`).
pub const WINE: CPort = CPort {
    name: "wine",
    url: "https://dl.winehq.org/wine/source/11.0/wine-11.0.tar.xz",
    file: "wine-11.0.tar.xz",
    sha256: "c07a6857933c1fc60dff5448d79f39c92481c1e9db5aa628db9d0358446e0701",
    top: "wine-11.0",
    patches: &[
        "0001-config-sub-magnumopus.patch",
        "0002-ntdll-signal-x86_64-magnumopus.patch",
        "0003-server-no-fork-magnumopus.patch",
        "0004-server-memfd-sections-magnumopus.patch",
        "0005-ntdll-spawn-process-magnumopus.patch",
        "0006-process-memory-magnumopus.patch",
        "0007-server-signals-magnumopus.patch",
    ],
    build: Build::Sources { lib: &[], defines: &[], programs: &[] },
    data: &[],
    dynamic: true,
};

/// Adresar zaplat Wine v repozitari.
fn patch_dir() -> PathBuf {
    workspace_root().join("third_party").join("wine")
}

/// Vystup sestaveni pro architekturu.
pub fn out_dir(arch: &Arch) -> PathBuf {
    target_dir().join("wine").join(arch.name)
}

/// Otisk zdroje: archiv a obsah zaplat (zmena = novy zdroj i konfigurace).
pub fn source_key(sha256: &str, patches: &[(String, Vec<u8>)]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    sha256.hash(&mut h);
    patches.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn read_patches() -> Result<Vec<(String, Vec<u8>)>> {
    WINE.patches
        .iter()
        .map(|f| Ok((f.to_string(), std::fs::read(patch_dir().join(f)).map_err(|e| format!("zaplata Wine {f}: {e}"))?)))
        .collect()
}

fn wsl_path(p: &Path) -> String {
    crate::qemu::to_wsl_paths(&p.display().to_string())
}

/// Sestavi Wine pro architekturu (predpoklad: `cargo xtask build`, ktery
/// pripravi sysroot C s `libc.so`).
pub fn build(arch: &Arch, keep_going: bool) -> Result<PathBuf> {
    if arch.name != "x86_64" {
        return Err("Windows vrstva je jen pro x86_64 (docs/cs/windows-vrstva.md, Q11 A: aarch64 je omezeni)".into());
    }
    let sysroot = crate::csysroot::dir(arch);
    if !sysroot.join("lib/libc.so").is_file() || !sysroot.join("lib/crtdso.o").is_file() {
        return Err("chybi dynamicka cast sysrootu C (lib/libc.so): nejdriv `cargo xtask build`".into());
    }
    let distro = wsl::distro();
    wsl::require_tools(&distro)?;
    let archive = crate::cports::fetch::fetch(&WINE)?;
    let patches = read_patches()?;
    let key = source_key(WINE.sha256, &patches);
    let out = out_dir(arch);
    std::fs::create_dir_all(&out)?;
    let script = workspace_root().join("tools/wsl/build_wine.sh");
    let env = [
        ("WINE_TARBALL", wsl_path(&archive)),
        ("WINE_TOP", WINE.top.to_string()),
        ("WINE_PATCHES", wsl_path(&patch_dir())),
        ("WINE_KEY", key),
        ("MOS_SYSROOT", wsl_path(&sysroot)),
        ("MOS_TOOLS", wsl_path(&workspace_root().join("tools/wsl"))),
        ("WINE_OUT", wsl_path(&out)),
        ("WINE_KEEP_GOING", String::from(if keep_going { "1" } else { "0" })),
    ];
    println!("xtask: Wine {} ve WSL ({distro}), vystup {}", WINE.top, out.display());
    wsl::run_script(&distro, &script, &env)?;
    Ok(out)
}
