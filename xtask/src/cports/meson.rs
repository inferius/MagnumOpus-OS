//! Zpusob `Meson`: Meson + Ninja s cizim cilem MagnumOpus (dav1d, pozdeji
//! FriBidi, HarfBuzz).
//!
//! Soubor pro krizovy preklad (`cross.txt`) se generuje do pracovniho
//! adresare z priznaku sysrootu C pro sdilene knihovny (`csysroot::dyn_parts`):
//! knihovny portu jsou staticke s `-fPIC` (`b_staticpic`), aby sly do sdilene
//! knihovny jineho portu (dav1d do `libavcodec.so`); zkusebni programy Mesonu
//! (`has_function`) se linkuji jako PIE nad `libc.so`.
//!
//! Meson z MSYS2 je skript Pythonu: spousti se pres `sh` v prostredi
//! [`super::tools::unix_env`], cesty dostava ve tvaru `/j/...`. Po instalaci
//! se v souborech `.pc` prepise predpona na tvar pro clang (`J:/...`).
//! Konfigurace se opakuje jen pri zmene voleb, souboru cross nebo zdroje.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::process::Command;

use super::tools::{Tool, require, sh_path, unix_env};
use super::{CPort, prefix};
use crate::arch::Arch;
use crate::{Result, csysroot};

fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Pole retezcu pro soubor Mesonu (`['a', 'b']`).
fn array(items: &[String]) -> String {
    let q: Vec<String> = items.iter().map(|s| format!("'{}'", s.replace('\\', "/").replace('\'', "\\'"))).collect();
    format!("[{}]", q.join(", "))
}

/// Soubor pro krizovy preklad architektury.
pub fn cross_file(arch: &Arch, sysroot: &Path) -> Result<String> {
    let d = csysroot::dyn_parts(arch, sysroot)?;
    let link: Vec<String> = d.ld.iter().chain(&d.ld_exe).chain(&d.libs).cloned().collect();
    let cpu = if arch.name == "aarch64" { "aarch64" } else { "x86_64" };
    Ok(format!(
        "# Krizovy preklad pro MagnumOpus (generuje xtask, src/cports/meson.rs).\n\
         [binaries]\n\
         c = 'clang'\n\
         ar = 'llvm-ar'\n\
         strip = 'llvm-strip'\n\
         nasm = 'nasm'\n\
         pkg-config = 'pkgconf'\n\
         \n\
         [built-in options]\n\
         c_args = {}\n\
         c_link_args = {}\n\
         \n\
         [host_machine]\n\
         system = 'magnumopus'\n\
         cpu_family = '{cpu}'\n\
         cpu = '{cpu}'\n\
         endian = 'little'\n",
        array(&d.cflags),
        array(&link)
    ))
}

#[cfg(test)]
pub fn array_for_test(items: &[String]) -> String {
    array(items)
}

/// Spusti Meson pres `sh` (skript Pythonu i na Windows).
fn meson(sh: &Path, clang: &Path, args: &[String], what: &str) -> Result<()> {
    let mut c = Command::new(sh);
    c.arg("-c").arg("exec meson \"$@\"").arg("meson").args(args);
    unix_env(&mut c, clang);
    let st = c.status()?;
    if !st.success() {
        return Err(format!("{what} selhal ({st})").into());
    }
    Ok(())
}

/// Prepise `prefix=` v nainstalovanych `.pc` na tvar `J:/...` (clang na
/// Windows cesty `/j/...` nezna).
fn fix_pkgconfig(pfx: &Path, files: &[&str]) -> Result<()> {
    for f in files {
        let path = pfx.join("lib").join("pkgconfig").join(f);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let fixed: Vec<String> = text
            .lines()
            .map(|l| if l.starts_with("prefix=") { format!("prefix={}", slash(pfx)) } else { l.to_string() })
            .collect();
        std::fs::write(&path, fixed.join("\n") + "\n")?;
    }
    Ok(())
}

pub fn build(arch: &Arch, sysroot: &Path, p: &CPort, src: &Path, work: &Path, options: &[&str], pkgconfig: &[&str]) -> Result<()> {
    let found = require(p.name, &[Tool::Sh, Tool::Meson, Tool::Ninja, Tool::PkgConf])?;
    let sh = &found[0];
    if arch.name == "x86_64" {
        // dav1d a FFmpeg maji rucni SIMD v syntaxi nasm (aarch64: clang).
        require(p.name, &[Tool::Nasm])?;
    }
    let clang = cbuild::find_clang()?;
    let cross = work.join("cross.txt");
    let text = cross_file(arch, sysroot)?;
    if std::fs::read_to_string(&cross).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&cross, &text)?;
    }
    let build = work.join("build");
    let pfx = prefix(arch);
    let mut h = DefaultHasher::new();
    options.hash(&mut h);
    text.hash(&mut h);
    src.hash(&mut h);
    let stamp = build.join(".konfigurace");
    let want = format!("{:016x}", h.finish());
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_dir_all(&build);
        let mut args = vec![
            String::from("setup"),
            sh_path(&build),
            sh_path(src),
            format!("--cross-file={}", sh_path(&cross)),
            format!("--prefix={}", sh_path(&pfx)),
            String::from("--libdir=lib"),
            String::from("--buildtype=release"),
            String::from("-Ddefault_library=static"),
            String::from("-Db_staticpic=true"),
        ];
        args.extend(options.iter().map(|o| o.to_string()));
        meson(sh, &clang, &args, &format!("meson setup {}", p.name))?;
        std::fs::write(&stamp, &want)?;
    }
    meson(sh, &clang, &[String::from("compile"), String::from("-C"), sh_path(&build)], &format!("meson compile {}", p.name))?;
    meson(sh, &clang, &[String::from("install"), String::from("-C"), sh_path(&build), String::from("--quiet")], &format!("meson install {}", p.name))?;
    fix_pkgconfig(&pfx, pkgconfig)
}

