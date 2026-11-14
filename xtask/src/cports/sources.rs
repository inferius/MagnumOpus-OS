//! Zpusob `Sources`: seznam zdroju prelozeny primo clangem (bez nastroju
//! navic). Objekty se prekladaji znovu jen pri zmene zdroje nebo priznaku;
//! link je vzdy.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::Prog;
use crate::arch::Arch;
use crate::{Result, csysroot, workspace_root};

fn source_path(src_dir: &Path, s: &str) -> PathBuf {
    match s.strip_prefix("ws:") {
        Some(rel) => workspace_root().join(rel),
        None => src_dir.join(s),
    }
}

fn newer(a: &Path, b: &Path) -> bool {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (m(a), m(b)) {
        (Some(x), Some(y)) => x > y,
        _ => true,
    }
}

/// Prelozi zdroj na objekt (znovu jen pri zmene zdroje nebo priznaku).
fn compile(clang: &Path, flags: &[String], src: &Path, obj_dir: &Path) -> Result<PathBuf> {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    flags.hash(&mut h);
    let obj = obj_dir.join(format!("{}-{:016x}.o", src.file_stem().unwrap_or_default().to_string_lossy(), h.finish()));
    if obj.exists() && !newer(src, &obj) {
        return Ok(obj);
    }
    let out = Command::new(clang).args(flags).arg("-c").arg(src).arg("-o").arg(&obj).output()?;
    if !out.status.success() {
        return Err(format!("preklad {} selhal:\n{}", src.display(), String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(obj)
}

pub fn build(arch: &Arch, sysroot: &Path, src: &Path, work: &Path, lib: &[&str], defines: &[&str], programs: &[Prog]) -> Result<Vec<(String, PathBuf)>> {
    let clang = cbuild::find_clang()?;
    let mut flags = csysroot::cflags(arch, sysroot)?;
    flags.extend(["-O2", "-w"].map(String::from));
    flags.extend(defines.iter().map(|d| d.to_string()));
    flags.push(format!("-I{}", src.display()));
    let ldflags = csysroot::ldflags(sysroot);
    let lib: Vec<PathBuf> = lib.iter().map(|s| compile(&clang, &flags, &source_path(src, s), work)).collect::<Result<_>>()?;
    let mut out = Vec::new();
    for prog in programs {
        let mut objs = lib.clone();
        for s in prog.sources {
            objs.push(compile(&clang, &flags, &source_path(src, s), work)?);
        }
        let exe = work.join(prog.name);
        let link = Command::new(&clang).args(cbuild::target_flags(arch.user_target)?).args(&objs).args(&ldflags).arg("-o").arg(&exe).output()?;
        if !link.status.success() {
            return Err(format!("link {} selhal:\n{}", prog.name, String::from_utf8_lossy(&link.stderr)).into());
        }
        out.push((String::from(prog.name), exe));
    }
    Ok(out)
}
