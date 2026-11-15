//! Zpusob `Sources`: seznam zdroju prelozeny primo clangem (bez nastroju
//! navic). Objekty se prekladaji znovu jen pri zmene zdroje nebo priznaku;
//! link je vzdy. Zdroje `.cc`/`.cpp` preklada clang++ proti libc++
//! (`cxx.rs`, sestavena pred porty) a program se pak linkuje s libc++.

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

/// `-I<relativni cesta>` v `defines` je vuci zdroji portu.
fn include_in(src: &Path, d: &str) -> String {
    match d.strip_prefix("-I") {
        Some(rel) if !Path::new(rel).is_absolute() => format!("-I{}", src.join(rel).display()),
        _ => d.to_string(),
    }
}

fn is_cxx(s: &str) -> bool {
    s.ends_with(".cc") || s.ends_with(".cpp") || s.ends_with(".cxx")
}

pub fn build(arch: &Arch, sysroot: &Path, src: &Path, work: &Path, lib: &[&str], defines: &[&str], programs: &[Prog]) -> Result<Vec<(String, PathBuf)>> {
    let clang = cbuild::find_clang()?;
    let clangxx = super::cmake::llvm_tool(&clang, "clang++");
    let common: Vec<String> =
        ["-O2", "-w"].map(String::from).into_iter().chain(defines.iter().map(|d| include_in(src, d))).chain([format!("-I{}", src.display())]).collect();
    let mut flags = csysroot::cflags(arch, sysroot)?;
    flags.extend(common.iter().cloned());
    let mut cxxflags = super::cxx::cxxflags(arch, sysroot, false)?;
    cxxflags.extend(common);
    let obj = |s: &str| -> Result<PathBuf> {
        if is_cxx(s) { compile(&clangxx, &cxxflags, &source_path(src, s), work) } else { compile(&clang, &flags, &source_path(src, s), work) }
    };
    let lib_cxx = lib.iter().any(|s| is_cxx(s));
    let lib: Vec<PathBuf> = lib.iter().map(|s| obj(s)).collect::<Result<_>>()?;
    let mut out = Vec::new();
    for prog in programs {
        let mut objs = lib.clone();
        for s in prog.sources {
            objs.push(obj(s)?);
        }
        let cxx = lib_cxx || prog.sources.iter().any(|s| is_cxx(s));
        let (linker, ldflags) =
            if cxx { (&clangxx, super::cxx::cxx_ldflags(arch, sysroot, false)) } else { (&clang, csysroot::ldflags(sysroot)) };
        let exe = work.join(prog.name);
        let link = Command::new(linker).args(cbuild::target_flags(arch.user_target)?).args(&objs).args(&ldflags).arg("-o").arg(&exe).output()?;
        if !link.status.success() {
            return Err(format!("link {} selhal:\n{}", prog.name, String::from_utf8_lossy(&link.stderr)).into());
        }
        out.push((String::from(prog.name), exe));
    }
    Ok(out)
}
