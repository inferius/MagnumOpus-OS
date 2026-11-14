//! Zpusob `CMake`: CMake + Ninja s cizim cilem MagnumOpus.
//!
//! Soubor nastroju (`toolchain.cmake`) a modul platformy
//! (`Platform/MagnumOpus.cmake`) se generuji do pracovniho adresare
//! architektury z priznaku sysrootu C (stejne jako `clang.cfg`). Knihovna
//! se instaluje do predpony portu (`super::prefix`), programy z adresare
//! sestaveni jdou do initfs.
//!
//! Konfigurace se opakuje jen pri zmene voleb, souboru nastroju nebo
//! zdroje (otisk v `.konfigurace`); sestaveni je prirustkove (Ninja).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::tools::{Tool, require};
use super::{CPort, prefix};
use crate::arch::Arch;
use crate::{Result, csysroot, target_dir};

fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Retezec pro CMake v uvozovkach.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "/").replace('"', "\\\""))
}

/// `llvm-ar` / `llvm-ranlib` vedle clangu.
fn llvm_tool(clang: &Path, name: &str) -> PathBuf {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    clang.parent().map(|d| d.join(&exe)).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(exe))
}

/// Modul platformy: unixovy cil jen se statickymi knihovnami.
const PLATFORM: &str = "\
# Platforma MagnumOpus pro CMake (generuje xtask, src/cports/cmake.rs).
set(UNIX 1)
set(MAGNUMOPUS 1)
set(CMAKE_STATIC_LIBRARY_PREFIX \"lib\")
set(CMAKE_STATIC_LIBRARY_SUFFIX \".a\")
set(CMAKE_EXECUTABLE_SUFFIX \"\")
set(CMAKE_FIND_LIBRARY_PREFIXES \"lib\")
set(CMAKE_FIND_LIBRARY_SUFFIXES \".a\")
# Sdilene knihovny az s libc.so (docs/cs/dynamicke-linkovani.md).
set_property(GLOBAL PROPERTY TARGET_SUPPORTS_SHARED_LIBS FALSE)
";

/// Vygeneruje soubor nastroju a modul platformy; vraci cestu k souboru
/// nastroju.
pub fn toolchain(arch: &Arch, sysroot: &Path) -> Result<PathBuf> {
    let clang = cbuild::find_clang()?;
    let dir = target_dir().join("c-ports").join(arch.name).join("cmake");
    std::fs::create_dir_all(dir.join("Platform"))?;
    let cflags = csysroot::cflags(arch, sysroot)?;
    // Knihovna C az za objekty (staticke archivy se ctou v poradi).
    let ldflags: Vec<String> = csysroot::ldflags(sysroot).into_iter().filter(|f| f != "-lc").collect();
    let pfx = prefix(arch);
    let processor = if arch.name == "aarch64" { "aarch64" } else { "x86_64" };
    let mut t = String::from("# Soubor nastroju CMake pro MagnumOpus (generuje xtask, src/cports/cmake.rs).\n");
    t.push_str(&format!("list(APPEND CMAKE_MODULE_PATH {})\n", quoted(&slash(&dir))));
    t.push_str("set(CMAKE_SYSTEM_NAME MagnumOpus)\n");
    t.push_str(&format!("set(CMAKE_SYSTEM_PROCESSOR {processor})\n"));
    t.push_str(&format!("set(CMAKE_C_COMPILER {})\n", quoted(&slash(&clang))));
    t.push_str(&format!("set(CMAKE_AR {})\n", quoted(&slash(&llvm_tool(&clang, "llvm-ar")))));
    t.push_str(&format!("set(CMAKE_RANLIB {})\n", quoted(&slash(&llvm_tool(&clang, "llvm-ranlib")))));
    t.push_str(&format!("set(CMAKE_C_FLAGS_INIT {})\n", quoted(&cflags.join(" "))));
    t.push_str(&format!("set(CMAKE_EXE_LINKER_FLAGS_INIT {})\n", quoted(&ldflags.join(" "))));
    t.push_str("set(CMAKE_C_STANDARD_LIBRARIES \"-lc\")\n");
    // Zkusebni preklady jen do statickych knihoven (link by potreboval main).
    t.push_str("set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)\n");
    t.push_str(&format!("set(CMAKE_FIND_ROOT_PATH {} {})\n", quoted(&slash(sysroot)), quoted(&slash(&pfx))));
    t.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)\n");
    t.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)\n");
    t.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)\n");
    t.push_str("set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)\n");
    // pkg-config hostitele nesmi najit knihovny Windows / Linuxu.
    t.push_str("set(PKG_CONFIG_EXECUTABLE \"\" CACHE FILEPATH \"\" FORCE)\n");
    write_if_changed(&dir.join("Platform").join("MagnumOpus.cmake"), PLATFORM.as_bytes())?;
    let path = dir.join("toolchain.cmake");
    write_if_changed(&path, t.as_bytes())?;
    Ok(path)
}

fn write_if_changed(path: &Path, data: &[u8]) -> Result<()> {
    if std::fs::read(path).ok().as_deref() != Some(data) {
        std::fs::write(path, data)?;
    }
    Ok(())
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let st = cmd.status()?;
    if !st.success() {
        return Err(format!("{what} selhal ({st})").into());
    }
    Ok(())
}

pub fn build(arch: &Arch, sysroot: &Path, p: &CPort, src: &Path, work: &Path, options: &[&str], programs: &[(&str, &str)]) -> Result<Vec<(String, PathBuf)>> {
    let found = require(p.name, &[Tool::CMake, Tool::Ninja])?;
    let (cmake, ninja) = (&found[0], &found[1]);
    let tc = toolchain(arch, sysroot)?;
    let build = work.join("build");
    let pfx = prefix(arch);
    let mut h = DefaultHasher::new();
    options.hash(&mut h);
    std::fs::read(&tc)?.hash(&mut h);
    src.hash(&mut h);
    let stamp = build.join(".konfigurace");
    let want = format!("{:016x}", h.finish());
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_dir_all(&build);
        std::fs::create_dir_all(&build)?;
        let mut c = Command::new(cmake);
        c.arg("-G").arg("Ninja").arg("-S").arg(src).arg("-B").arg(&build);
        c.arg(format!("-DCMAKE_MAKE_PROGRAM={}", slash(ninja)));
        c.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", slash(&tc)));
        c.arg(format!("-DCMAKE_INSTALL_PREFIX={}", slash(&pfx)));
        c.arg("-DCMAKE_BUILD_TYPE=Release");
        c.args(options);
        run(&mut c, &format!("cmake (konfigurace {})", p.name))?;
        std::fs::write(&stamp, &want)?;
    }
    run(Command::new(cmake).arg("--build").arg(&build), &format!("cmake --build {}", p.name))?;
    run(Command::new(cmake).arg("--install").arg(&build), &format!("cmake --install {}", p.name))?;
    Ok(programs.iter().map(|(name, rel)| (name.to_string(), build.join(rel))).collect())
}
