//! Zpusob `Configure`: skript `configure` (autotools, FFmpeg) + make.
//!
//! Argumenty maji zastupne hodnoty ({prefix}, {host}, {arch}, {cc}, {ar},
//! {nm}, {ranlib}, {strip}, {cflags}, {ldflags}, u portu se sdilenymi
//! knihovnami navic {ldexeflags}, {ldsoflags}, {libs}), protoze kazdy
//! skript chce cil jinak (autotools `--host=`, FFmpeg `--target-os= --arch=
//! --cc=`). Nastroje LLVM jsou jen jmenem (adresar clangu je v PATH,
//! configure FFmpeg nezvlada cesty s mezerou). Konfigurace se opakuje jen
//! pri zmene argumentu nebo zdroje; sestaveni mimo strom zdroju.
//!
//! Port se sdilenymi knihovnami (FFmpeg nad `libc.so`) bere priznaky
//! `csysroot::dyn_parts`; knihovny jdou do initfs `lib/` pod jmenem soname.
//! Na Windows `sh`, `make` a `pkgconf` z MSYS2 (`tools::unix_env`).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::tools::{Tool, require, sh_path, unix_env};
use super::{CPort, prefix};
use crate::arch::Arch;
use crate::{Result, csysroot};

fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Dosadi zastupne hodnoty do argumentu.
pub fn expand(arg: &str, vars: &[(&str, String)]) -> String {
    let mut s = arg.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// Zastupne hodnoty pro architekturu (`shared` = port nad `libc.so`).
pub fn vars(arch: &Arch, sysroot: &Path, shared: bool) -> Result<Vec<(&'static str, String)>> {
    let mut v = vec![
        ("prefix", slash(&prefix(arch))),
        ("host", format!("{}-unknown-magnumopus", arch.name)),
        ("arch", arch.name.to_string()),
        ("cc", String::from("clang")),
        ("ar", String::from("llvm-ar")),
        ("nm", String::from("llvm-nm")),
        ("ranlib", String::from("llvm-ranlib")),
        ("strip", String::from("llvm-strip")),
    ];
    if shared {
        let d = csysroot::dyn_parts(arch, sysroot)?;
        v.push(("cflags", d.cflags.join(" ")));
        v.push(("ldflags", d.ld.join(" ")));
        v.push(("ldexeflags", d.ld_exe.join(" ")));
        v.push(("ldsoflags", d.ld_shared.join(" ")));
        v.push(("libs", d.libs.join(" ")));
    } else {
        v.push(("cflags", csysroot::cflags(arch, sysroot)?.join(" ")));
        v.push(("ldflags", csysroot::ldflags(sysroot).into_iter().filter(|f| f != "-lc").collect::<Vec<_>>().join(" ")));
    }
    Ok(v)
}

/// Vystup: programy a sdilene knihovny (jmeno v initfs, cesta).
pub type Built = (Vec<(String, PathBuf)>, Vec<(String, PathBuf)>);

pub fn build(
    arch: &Arch,
    sysroot: &Path,
    p: &CPort,
    src: &Path,
    work: &Path,
    args: &[&str],
    programs: &[(&str, &str)],
    libraries: &[(&str, &str)],
) -> Result<Built> {
    let shared = !libraries.is_empty();
    let found = require(p.name, &[Tool::Sh, Tool::Make, Tool::PkgConf])?;
    let (sh, make) = (&found[0], &found[1]);
    if arch.name == "x86_64" && args.iter().any(|a| a.contains("nasm")) {
        require(p.name, &[Tool::Nasm])?;
    }
    let clang = cbuild::find_clang()?;
    let vars = vars(arch, sysroot, shared)?;
    let expanded: Vec<String> = args.iter().map(|a| expand(a, &vars)).collect();
    let build = work.join("build");
    let mut h = DefaultHasher::new();
    expanded.hash(&mut h);
    src.hash(&mut h);
    let want = format!("{:016x}", h.finish());
    let stamp = build.join(".konfigurace");
    let pkg = sh_path(&prefix(arch).join("lib").join("pkgconfig"));
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_dir_all(&build);
        std::fs::create_dir_all(&build)?;
        let mut c = Command::new(sh);
        c.arg(src.join("configure")).args(&expanded).current_dir(&build);
        c.env("PKG_CONFIG_LIBDIR", &pkg);
        if !shared {
            c.env("LIBS", "-lc");
        }
        unix_env(&mut c, &clang);
        let st = c.status()?;
        if !st.success() {
            return Err(format!("configure portu {} selhal ({st}); log: {}", p.name, build.join("ffbuild/config.log").display()).into());
        }
        std::fs::write(&stamp, &want)?;
    }
    if !shared {
        let outputs: Vec<PathBuf> = programs.iter().map(|(_, rel)| build.join(rel)).collect();
        super::relink_on_libc_change(sysroot, &build, &outputs)?;
    }
    let jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    for target in [vec![format!("-j{jobs}")], vec![String::from("install")]] {
        let mut c = Command::new(make);
        c.args(&target).current_dir(&build).env("PKG_CONFIG_LIBDIR", &pkg);
        unix_env(&mut c, &clang);
        let st = c.status()?;
        if !st.success() {
            return Err(format!("make {} portu {} selhal ({st})", target.join(" "), p.name).into());
        }
    }
    let progs = programs.iter().map(|(name, rel)| (name.to_string(), build.join(rel))).collect();
    let libs = libraries.iter().map(|(name, rel)| (name.to_string(), build.join(rel))).collect();
    Ok((progs, libs))
}
