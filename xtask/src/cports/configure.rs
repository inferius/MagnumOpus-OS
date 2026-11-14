//! Zpusob `Configure`: skript `configure` (autotools, FFmpeg) + make.
//!
//! Argumenty maji zastupne hodnoty ({prefix}, {host}, {arch}, {cc}, {ar},
//! {cflags}, {ldflags}), protoze kazdy skript chce cil jinak (autotools
//! `--host=`, FFmpeg `--target-os= --arch= --cc=`). Konfigurace se opakuje
//! jen pri zmene argumentu nebo zdroje; sestaveni mimo strom zdroju.
//!
//! Na Windows potrebuje `sh` (Git) a `make` (MSYS2), jinak radu k instalaci;
//! overi se s prvnim portem tohoto druhu (FFmpeg).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::tools::{Tool, require};
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

pub fn build(arch: &Arch, sysroot: &Path, p: &CPort, src: &Path, work: &Path, args: &[&str], programs: &[(&str, &str)]) -> Result<Vec<(String, PathBuf)>> {
    let found = require(p.name, &[Tool::Sh, Tool::Make])?;
    let (sh, make) = (&found[0], &found[1]);
    let clang = cbuild::find_clang()?;
    let ar = clang.with_file_name(if cfg!(windows) { "llvm-ar.exe" } else { "llvm-ar" });
    let cflags = csysroot::cflags(arch, sysroot)?.join(" ");
    let ldflags = csysroot::ldflags(sysroot).into_iter().filter(|f| f != "-lc").collect::<Vec<_>>().join(" ");
    let vars = [
        ("prefix", slash(&prefix(arch))),
        ("host", format!("{}-unknown-magnumopus", arch.name)),
        ("arch", arch.name.to_string()),
        ("cc", slash(&clang)),
        ("ar", slash(&ar)),
        ("cflags", cflags),
        ("ldflags", ldflags),
    ];
    let expanded: Vec<String> = args.iter().map(|a| expand(a, &vars)).collect();
    let build = work.join("build");
    let mut h = DefaultHasher::new();
    expanded.hash(&mut h);
    src.hash(&mut h);
    let want = format!("{:016x}", h.finish());
    let stamp = build.join(".konfigurace");
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_dir_all(&build);
        std::fs::create_dir_all(&build)?;
        let st = Command::new(sh).arg(src.join("configure")).args(&expanded).env("LIBS", "-lc").current_dir(&build).status()?;
        if !st.success() {
            return Err(format!("configure portu {} selhal ({st}); log: {}", p.name, build.join("config.log").display()).into());
        }
        std::fs::write(&stamp, &want)?;
    }
    let jobs = std::thread::available_parallelism().map_or(4, |n| n.get());
    for target in [vec![format!("-j{jobs}")], vec![String::from("install")]] {
        let st = Command::new(make).args(&target).current_dir(&build).status()?;
        if !st.success() {
            return Err(format!("make {} portu {} selhal ({st})", target.join(" "), p.name).into());
        }
    }
    Ok(programs.iter().map(|(name, rel)| (name.to_string(), build.join(rel))).collect())
}
