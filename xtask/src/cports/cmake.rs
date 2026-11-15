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

pub(super) fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Retezec pro CMake v uvozovkach.
fn quoted(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "/").replace('"', "\\\""))
}

/// `llvm-ar` / `llvm-ranlib` vedle clangu.
pub(super) fn llvm_tool(clang: &Path, name: &str) -> PathBuf {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    clang.parent().map(|d| d.join(&exe)).filter(|p| p.is_file()).unwrap_or_else(|| PathBuf::from(exe))
}

/// Modul platformy: unixovy cil; staticky soubor nastroju jen se
/// statickymi knihovnami, dynamicky (`MAGNUMOPUS_DYNAMIC`, PIC nad
/// `libc.so`) i se sdilenymi bez verze v SONAME (`/system/lib`, libd).
const PLATFORM: &str = "\
# Platforma MagnumOpus pro CMake (generuje xtask, src/cports/cmake.rs).
set(UNIX 1)
set(MAGNUMOPUS 1)
set(CMAKE_STATIC_LIBRARY_PREFIX \"lib\")
set(CMAKE_STATIC_LIBRARY_SUFFIX \".a\")
set(CMAKE_EXECUTABLE_SUFFIX \"\")
set(CMAKE_FIND_LIBRARY_PREFIXES \"lib\")
if(MAGNUMOPUS_DYNAMIC)
  set(CMAKE_FIND_LIBRARY_SUFFIXES \".so\" \".a\")
  set(CMAKE_SHARED_LIBRARY_PREFIX \"lib\")
  set(CMAKE_SHARED_LIBRARY_SUFFIX \".so\")
  set(CMAKE_SHARED_MODULE_PREFIX \"lib\")
  set(CMAKE_SHARED_MODULE_SUFFIX \".so\")
  foreach(lang C CXX)
    set(CMAKE_SHARED_LIBRARY_${lang}_FLAGS \"-fPIC\")
    set(CMAKE_SHARED_LIBRARY_CREATE_${lang}_FLAGS \"-shared\")
    set(CMAKE_SHARED_LIBRARY_SONAME_${lang}_FLAG \"-Wl,-soname,\")
  endforeach()
  # Zavadec hleda knihovny jen v /system/lib: bez RPATH a bez libfoo.so.1.
  set(CMAKE_PLATFORM_NO_VERSIONED_SONAME 1)
  set(CMAKE_SKIP_RPATH 1)
  set_property(GLOBAL PROPERTY TARGET_SUPPORTS_SHARED_LIBS TRUE)
else()
  set(CMAKE_FIND_LIBRARY_SUFFIXES \".a\")
  # Sdilene knihovny jen s dynamickym souborem nastroju.
  set_property(GLOBAL PROPERTY TARGET_SUPPORTS_SHARED_LIBS FALSE)
endif()
";

/// Vygeneruje soubor nastroju a modul platformy; vraci cestu k souboru
/// nastroju.
pub fn toolchain(arch: &Arch, sysroot: &Path) -> Result<PathBuf> {
    toolchain_for(arch, sysroot, false)
}

/// Soubor nastroju: staticky (programy ET_EXEC s `libc.a`) nebo dynamicky
/// (`toolchain-dyn.cmake`: PIC, programy PIE a sdilene knihovny nad
/// `libc.so`; vyzaduje dynamickou cast sysrootu `csysroot::add_dynamic`).
/// Oba nastavuji C i C++ (clang++ vedle clangu).
pub fn toolchain_for(arch: &Arch, sysroot: &Path, dynamic: bool) -> Result<PathBuf> {
    let clang = cbuild::find_clang()?;
    let dir = target_dir().join("c-ports").join(arch.name).join("cmake");
    std::fs::create_dir_all(dir.join("Platform"))?;
    let (cflags, ldflags, shared) = if dynamic {
        // Knihovna C (libc.so) a pomocne funkce prekladace az za objekty.
        let tail = |f: &String| f == "-l:libc.so" || f == "-l:libbuiltins.a" || f == "-Wl,-Bdynamic";
        let exe: Vec<String> = csysroot::ldflags_dyn(sysroot).into_iter().filter(|f| !tail(f)).collect();
        // Sdilena knihovna: -Bdynamic zustava na zacatku (zavislosti .so
        // pred libc.so, napr. libunwind.so v libc++.so).
        let so: Vec<String> =
            csysroot::ldflags_so(sysroot).into_iter().filter(|f| f == "-Wl,-Bdynamic" || !tail(f)).collect();
        (csysroot::cflags_dyn(arch, sysroot)?, exe, so)
    } else {
        // Knihovna C az za objekty (staticke archivy se ctou v poradi).
        let exe: Vec<String> = csysroot::ldflags(sysroot).into_iter().filter(|f| f != "-lc").collect();
        (csysroot::cflags(arch, sysroot)?, exe, Vec::new())
    };
    let libs = if dynamic { "-Wl,-Bdynamic -l:libc.so -l:libbuiltins.a" } else { "-lc" };
    let pfx = prefix(arch);
    let processor = if arch.name == "aarch64" { "aarch64" } else { "x86_64" };
    let mut t = String::from("# Soubor nastroju CMake pro MagnumOpus (generuje xtask, src/cports/cmake.rs).\n");
    t.push_str(&format!("list(APPEND CMAKE_MODULE_PATH {})\n", quoted(&slash(&dir))));
    t.push_str("set(CMAKE_SYSTEM_NAME MagnumOpus)\n");
    t.push_str(&format!("set(CMAKE_SYSTEM_PROCESSOR {processor})\n"));
    if dynamic {
        t.push_str("set(MAGNUMOPUS_DYNAMIC 1)\n");
    }
    t.push_str(&format!("set(CMAKE_C_COMPILER {})\n", quoted(&slash(&clang))));
    t.push_str(&format!("set(CMAKE_CXX_COMPILER {})\n", quoted(&slash(&llvm_tool(&clang, "clang++")))));
    t.push_str(&format!("set(CMAKE_AR {})\n", quoted(&slash(&llvm_tool(&clang, "llvm-ar")))));
    t.push_str(&format!("set(CMAKE_RANLIB {})\n", quoted(&slash(&llvm_tool(&clang, "llvm-ranlib")))));
    t.push_str(&format!("set(CMAKE_C_FLAGS_INIT {})\n", quoted(&cflags.join(" "))));
    // Assembler (.S, libunwind) taky pro cil, jinak by clang prekladal pro hostitele.
    t.push_str(&format!("set(CMAKE_ASM_FLAGS_INIT {})\n", quoted(&cflags.join(" "))));
    t.push_str(&format!("set(CMAKE_CXX_FLAGS_INIT {})\n", quoted(&cflags.join(" "))));
    t.push_str(&format!("set(CMAKE_EXE_LINKER_FLAGS_INIT {})\n", quoted(&ldflags.join(" "))));
    if dynamic {
        t.push_str(&format!("set(CMAKE_SHARED_LINKER_FLAGS_INIT {})\n", quoted(&shared.join(" "))));
        t.push_str(&format!("set(CMAKE_MODULE_LINKER_FLAGS_INIT {})\n", quoted(&shared.join(" "))));
    }
    t.push_str(&format!("set(CMAKE_C_STANDARD_LIBRARIES {})\n", quoted(libs)));
    t.push_str(&format!("set(CMAKE_CXX_STANDARD_LIBRARIES {})\n", quoted(libs)));
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
    let path = dir.join(if dynamic { "toolchain-dyn.cmake" } else { "toolchain.cmake" });
    write_if_changed(&path, t.as_bytes())?;
    Ok(path)
}

pub(super) fn write_if_changed(path: &Path, data: &[u8]) -> Result<()> {
    if std::fs::read(path).ok().as_deref() != Some(data) {
        std::fs::write(path, data)?;
    }
    Ok(())
}

pub(super) fn run(cmd: &mut Command, what: &str) -> Result<()> {
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
