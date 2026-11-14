//! Sysroot C pro cizi build systemy a porty (docs/cs/cstd.md, kapitola
//! Sysroot C): `target/c-sysroot/<arch>/`
//!
//! - `include/` (hlavicky cstd) a `include-core/` (zaklad cstd-core;
//!   `stdio.h` apod. z cstd ho doplnuji pres `#include_next`),
//! - `lib/libc.a` (cstd + std + osrt vcetne `_start`), `lib/magnumopus.ld`
//!   (linker skript programu se std),
//! - `clang.cfg` - priznaky pro `clang --config=...` (preklad i link).
//!
//! Prepisuje se jen pri zmene obsahu (soubezne behy xtask, viz
//! `stdsys::lock_std`; volajici drzi zamek std).

use std::path::{Path, PathBuf};

use crate::arch::Arch;
use crate::{Result, target_dir, workspace_root};

/// Adresar sysrootu C pro architekturu.
pub fn dir(arch: &Arch) -> PathBuf {
    target_dir().join("c-sysroot").join(arch.name)
}

/// Cil Rustu pro programy (`x86_64-unknown-magnumopus`, ...).
fn rust_target(arch: &Arch) -> &'static str {
    arch.user_target
}

fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Priznaky prekladu programu v C proti sysrootu (bez vlastnich `-I`).
pub fn cflags(arch: &Arch, sysroot: &Path) -> Result<Vec<String>> {
    let mut f: Vec<String> = cbuild::target_flags(rust_target(arch))?.iter().map(|s| s.to_string()).collect();
    // Hostovane C (plna libc), ne -ffreestanding; hlavicky jen nase
    // a vestavene clangu (stddef.h, stdint.h, ...).
    f.extend(
        [
            "-nostdlibinc",
            "-fno-pic",
            "-fno-common",
            "-ftls-model=local-exec",
            "-ffunction-sections",
            "-fdata-sections",
            "-fstack-protector-strong",
            "-mstack-protector-guard=global",
            "-ftrivial-auto-var-init=zero",
            "-fno-strict-aliasing",
            "-fwrapv",
            "-fno-delete-null-pointer-checks",
            "-D__magnumopus__=1",
        ]
        .map(String::from),
    );
    f.push(String::from("-isystem"));
    f.push(slash(&sysroot.join("include")));
    f.push(String::from("-isystem"));
    f.push(slash(&sysroot.join("include-core")));
    Ok(f)
}

/// Priznaky linkovani programu (staticky ET_EXEC s libc.a).
pub fn ldflags(sysroot: &Path) -> Vec<String> {
    vec![
        String::from("-nostdlib"),
        String::from("-static"),
        String::from("-fuse-ld=lld"),
        format!("-Wl,-T,{}", slash(&sysroot.join("lib/magnumopus.ld"))),
        String::from("-Wl,--gc-sections"),
        format!("-L{}", slash(&sysroot.join("lib"))),
        String::from("-lc"),
    ]
}

fn write_if_changed(path: &Path, data: &[u8]) -> Result<()> {
    if std::fs::read(path).ok().as_deref() == Some(data) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, data)?;
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &target)?;
        } else {
            write_if_changed(&target, &std::fs::read(e.path())?)?;
        }
    }
    Ok(())
}

/// Slozi sysroot z prelozene `libc.a` (adresar vystupu `stdsys::build_apps`).
pub fn assemble(arch: &Arch, std_bin: &Path) -> Result<PathBuf> {
    let root = dir(arch);
    copy_tree(&workspace_root().join("libs/cstd/include"), &root.join("include"))?;
    copy_tree(&workspace_root().join("libs/cstd-core/include"), &root.join("include-core"))?;
    let libc = std::fs::read(std_bin.join("libc.a")).map_err(|e| format!("libc.a: {e}"))?;
    write_if_changed(&root.join("lib/libc.a"), &libc)?;
    write_if_changed(&root.join("lib/magnumopus.ld"), &std::fs::read(workspace_root().join("user/user-std.ld"))?)?;
    let mut cfg = String::from("# Sysroot C MagnumOpus: clang --config=<tento soubor> (preklad i link).\n");
    for f in cflags(arch, &root)?.into_iter().chain(ldflags(&root)) {
        cfg.push_str(&f);
        cfg.push('\n');
    }
    write_if_changed(&root.join("clang.cfg"), cfg.as_bytes())?;
    Ok(root)
}
