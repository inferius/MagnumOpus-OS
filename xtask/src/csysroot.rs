//! Sysroot C pro cizi build systemy a porty (docs/cs/cstd.md, kapitola
//! Sysroot C): `target/c-sysroot/<arch>/`
//!
//! - `include/` (hlavicky cstd a `mosdl.h`) a `include-core/` (zaklad cstd-core;
//!   `stdio.h` apod. z cstd ho doplnuji pres `#include_next`),
//! - `lib/libc.a` (cstd + std + osrt vcetne `_start`), prazdne `libm.a`,
//!   `libpthread.a`, `librt.a` (vse je v libc.a), `lib/magnumopus.ld`
//!   (linker skript programu se std),
//! - `clang.cfg` - priznaky pro `clang --config=...` (preklad i link).
//!
//! Prepisuje se jen pri zmene obsahu (soubezne behy xtask, viz
//! `stdsys::lock_std`; volajici drzi zamek std).

use std::path::{Path, PathBuf};

use crate::arch::Arch;
use crate::{Result, target_dir, workspace_root};

/// Knihovny, ktere jsou soucasti `libc.a` (prazdne archivy pro `-l...`).
const EMPTY_LIBS: &[&str] = &["libm.a", "libpthread.a", "librt.a"];

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
            // Tabulky rozbalovani i v C: vyjimka C++ smi projit ramcem C
            // (zpetna volani, docs/cs/cpp.md). Cil *-none-elf je sam nedela.
            "-funwind-tables",
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
        // Tabulka .eh_frame_hdr pro libunwind (C++); ovladac clangu pro
        // cil *-none-elf ji sam nepozada.
        String::from("-Wl,--eh-frame-hdr"),
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
    // Rozhrani pro port SDL (symboly v libc.a).
    copy_tree(&workspace_root().join("libs/mosdl/include"), &root.join("include"))?;
    let libc = std::fs::read(std_bin.join("libc.a")).map_err(|e| format!("libc.a: {e}"))?;
    write_if_changed(&root.join("lib/libc.a"), &libc)?;
    // Jako musl: matematika, vlakna a hodiny jsou v libc.a, ale build
    // systemy linkuji `-lm -lpthread -lrt` (CMake SDL, configure). Prazdne
    // archivy, aby link prosel.
    for name in EMPTY_LIBS {
        write_if_changed(&root.join("lib").join(name), b"!<arch>\n")?;
    }
    write_if_changed(&root.join("lib/magnumopus.ld"), &std::fs::read(workspace_root().join("user/user-std.ld"))?)?;
    let mut cfg = String::from("# Sysroot C MagnumOpus: clang --config=<tento soubor> (preklad i link).\n");
    for f in cflags(arch, &root)?.into_iter().chain(ldflags(&root)) {
        cfg.push_str(&f);
        cfg.push('\n');
    }
    write_if_changed(&root.join("clang.cfg"), cfg.as_bytes())?;
    Ok(root)
}

/// Priznaky prekladu programu PIE nad `libc.so` (bez `-ftls-model`: TLS
/// programu urci prekladac, knihovny jdou pres zavadec).
pub fn cflags_dyn(arch: &Arch, sysroot: &Path) -> Result<Vec<String>> {
    let mut f = cflags(arch, sysroot)?;
    f.retain(|x| x != "-fno-pic" && x != "-ftls-model=local-exec");
    f.push(String::from("-fPIE"));
    Ok(f)
}

/// Priznaky linkovani programu PIE nad `libc.so` (pravidla zavadece:
/// `PT_INTERP`, BIND_NOW, RELRO, bez TEXTREL, segmenty 64 KiB).
pub fn ldflags_dyn(sysroot: &Path) -> Vec<String> {
    vec![
        String::from("-nostdlib"),
        String::from("-fuse-ld=lld"),
        String::from("-Wl,-pie"),
        String::from("-Wl,--dynamic-linker=/system/lib/ld.so"),
        String::from("-Wl,-z,now"),
        String::from("-Wl,-z,relro"),
        String::from("-Wl,-z,text"),
        String::from("-Wl,-z,max-page-size=65536"),
        String::from("-Wl,--hash-style=both"),
        String::from("-Wl,--eh-frame-hdr"),
        String::from("-Wl,--allow-shlib-undefined"),
        slash(&sysroot.join("lib/crt1.o")),
        format!("-L{}", slash(&sysroot.join("lib"))),
        // Ovladac clangu pro baremetal linkuje staticky (-Bstatic) a vzal by
        // libc.a: dynamicky a primo libc.so.
        String::from("-Wl,-Bdynamic"),
        String::from("-l:libc.so"),
        // Pomocne funkce prekladace (aarch64 long double __addtf3 ...,
        // 128bitove deleni) staticky do programu, jako libgcc.a.
        String::from("-l:libbuiltins.a"),
    ]
}

/// Priznaky linkovani sdilene knihovny nad `libc.so` (`libc++.so`): jako
/// [`ldflags_dyn`], ale `-shared` bez `crt1.o` a bez PIE.
pub fn ldflags_so(sysroot: &Path) -> Vec<String> {
    ldflags_dyn(sysroot)
        .into_iter()
        .map(|f| if f.ends_with("crt1.o") { slash(&sysroot.join("lib/crtdso.o")) } else { f })
        .filter(|f| f != "-Wl,-pie" && !f.starts_with("-Wl,--dynamic-linker"))
        // Ovladac clangu pro cil *-none-elf `-shared` linkeru nepreda.
        .chain([String::from("-shared"), String::from("-Wl,-shared")])
        .collect()
}

/// Dynamicka cast sysrootu: `lib/libc.so`, `lib/crt1.o` (z
/// `libs/cstd/crt/crt1.c`) a `clang-dyn.cfg` pro programy PIE.
pub fn add_dynamic(arch: &Arch, root: &Path, libc_so: &Path) -> Result<()> {
    write_if_changed(&root.join("lib/libc.so"), &std::fs::read(libc_so)?)?;
    let clang = cbuild::find_clang()?;
    builtins(arch, root, &clang)?;
    // crt1.o (start programu PIE) a crtdso.o (__dso_handle knihovny).
    for name in ["crt1", "crtdso"] {
        let obj = root.join(format!("lib/{name}.o"));
        let src = workspace_root().join(format!("libs/cstd/crt/{name}.c"));
        let out = std::process::Command::new(&clang)
            .args(cflags_dyn(arch, root)?)
            .args(["-c", "-O2"])
            .arg(&src)
            .arg("-o")
            .arg(&obj)
            .output()?;
        if !out.status.success() {
            return Err(format!("{name}.o: {}", String::from_utf8_lossy(&out.stderr)).into());
        }
    }
    let mut cfg = String::from("# Sysroot C MagnumOpus, programy PIE nad libc.so: clang --config=<tento soubor>.\n");
    for f in cflags_dyn(arch, root)?.into_iter().chain(ldflags_dyn(root)) {
        cfg.push_str(&f);
        cfg.push('\n');
    }
    write_if_changed(&root.join("clang-dyn.cfg"), cfg.as_bytes())?;
    Ok(())
}

/// Prelozi program v C jen pres `clang --config=clang-dyn.cfg` (overeni
/// cesty pro cizi build systemy); vraci obsah binarky.
pub fn build_with_cfg(root: &Path, src: &str, name: &str) -> Result<Vec<u8>> {
    let clang = cbuild::find_clang()?;
    let out = root.join("test").join(name);
    std::fs::create_dir_all(root.join("test"))?;
    let o = std::process::Command::new(&clang)
        .arg(format!("--config={}", slash(&root.join("clang-dyn.cfg"))))
        .arg("-Wno-unused-command-line-argument")
        .arg(workspace_root().join(src))
        .arg("-o")
        .arg(&out)
        .output()?;
    if !o.status.success() {
        return Err(format!("{name} pres clang-dyn.cfg: {}", String::from_utf8_lossy(&o.stderr)).into());
    }
    Ok(std::fs::read(&out)?)
}

/// `lib/libbuiltins.a`: compiler_builtins ze sysrootu std jako PIC (bez
/// clenu `lib.rmeta`), staticky do programu PIE - libc.so pomocne funkce
/// prekladace neexportuje (rustc je v cdylib skryva).
fn builtins(arch: &Arch, root: &Path, clang: &Path) -> Result<()> {
    let dir = crate::stdsys::sysroot_dir()?.join("lib/rustlib").join(crate::stdsys::target_name_dyn(arch)).join("lib");
    let rlib = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            n.starts_with("libcompiler_builtins-") && n.ends_with(".rlib")
        })
        .ok_or("compiler_builtins chybi v sysrootu std jako PIC")?;
    let out = root.join("lib/libbuiltins.a");
    let data = std::fs::read(&rlib)?;
    let stamp = root.join("lib/libbuiltins.stamp");
    let id = format!("{}:{}", rlib.display(), data.len());
    if out.exists() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(id.as_str()) {
        return Ok(());
    }
    std::fs::write(&out, &data)?;
    let ar = clang.with_file_name(if cfg!(windows) { "llvm-ar.exe" } else { "llvm-ar" });
    let ok = std::process::Command::new(&ar).arg("d").arg(&out).arg("lib.rmeta").status()?.success();
    if !ok {
        return Err("llvm-ar: odebrani lib.rmeta z libbuiltins.a selhalo".into());
    }
    std::fs::write(&stamp, id)?;
    Ok(())
}
