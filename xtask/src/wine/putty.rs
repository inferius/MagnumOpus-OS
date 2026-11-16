//! PuTTY pro test Windows vrstvy (W4, docs/cs/windows-vrstva.md, kapitola
//! W4): pinnuty zdroj (HTTPS, SHA-256, podpis overeny pri pripnuti, viz
//! THIRD-PARTY.md), preklad na hostiteli clangem pro
//! `x86_64-pc-windows-msvc` nad hlavickami a importnimi knihovnami Wine
//! (`target/wine/<arch>/sdk` z `build_wine.sh`), CMake + Ninja z projektu
//! PuTTY. Vysledek `putty.exe` jde k testovacim programum Windows
//! (`target/wine/<arch>/tests`, na datovem disku `/apps/org.magnumopus.wintest`).
//!
//! Zadne nastroje navic: CMake, Ninja a LLVM (clang, lld-link, llvm-rc) jsou
//! stejne jako u portu C.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cports::{Build, CPort};
use crate::{Result, target_dir};

/// PuTTY 0.85 (16. 8. 2026), licence MIT. SHA-256 ze `sha256sums`
/// projektu; archiv overen podpisem klice PuTTY Releases 2026
/// (1A07 DAC0 F220 CF12 248E A719 5C67 D0F8 8BC3 E5B5).
pub const PUTTY: CPort = CPort {
    name: "putty",
    url: "https://the.earth.li/~sgtatham/putty/0.85/putty-0.85.tar.gz",
    file: "putty-0.85.tar.gz",
    sha256: "13fd4db2936d03b73812a7bcc2a658e4dd29cc776a56c3670a7fc6f1a0ee8af8",
    top: "putty-0.85",
    patches: &[],
    build: Build::Sources { lib: &[], defines: &[], programs: &[] },
    data: &[],
    dynamic: false,
};

/// Importni knihovny pro kazdy program (CRT Wine, start, pomocne funkce
/// prekladace); knihovny Windows dava PuTTY sam (`platform_libraries`).
// `/manifest:no` az na konci (za `/MANIFEST:EMBED` od CMake): manifest dava PuTTY
// ve svem .rc (dva by byly duplicitni prostredek).
const STD_LIBS: &str = "-lwinecrt0 -lucrtbase -lkernel32 -lntdll -lcompiler-rt -Wl,/manifest:no";

fn slash(p: &Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// Soubor nastroju CMake: Windows (MSVC ABI) s CRT a hlavickami Wine, bez
/// systemovych hlavicek a knihoven hostitele (`-nostdlibinc`,
/// `/nodefaultlib`).
fn toolchain(clang: &Path, sdk: &Path, rc: &Path, crt: &Path) -> String {
    let tool = |n: &str| slash(&clang.with_file_name(format!("{n}{}", std::env::consts::EXE_SUFFIX)));
    let inc = [sdk.join("include"), sdk.join("include").join("msvcrt"), sdk.join("include-gen")];
    let cflags: Vec<String> = inc.iter().map(|d| format!("-I{}", slash(d))).collect();
    let rcflags: Vec<String> = inc.iter().map(|d| format!("/I {}", slash(d))).collect();
    let mut t = String::from("# Soubor nastroju CMake pro programy Windows nad SDK Wine (generuje xtask, src/wine/putty.rs).\n");
    t.push_str("set(CMAKE_SYSTEM_NAME Windows)\nset(CMAKE_SYSTEM_PROCESSOR AMD64)\n");
    t.push_str(&format!("set(CMAKE_C_COMPILER \"{}\")\n", slash(clang)));
    t.push_str("set(CMAKE_C_COMPILER_TARGET x86_64-pc-windows-msvc)\n");
    t.push_str(&format!("set(CMAKE_RC_COMPILER \"{}\")\n", slash(rc)));
    t.push_str(&format!("set(CMAKE_AR \"{}\")\n", tool("llvm-ar")));
    t.push_str(&format!("set(CMAKE_C_FLAGS_INIT \"-nostdlibinc -D_UCRT -mno-stack-arg-probe -Wno-everything {}\")\n", cflags.join(" ")));
    t.push_str(&format!("set(CMAKE_RC_FLAGS_INIT \"/X {}\")\n", rcflags.join(" ")));
    t.push_str(&format!(
        "set(CMAKE_EXE_LINKER_FLAGS_INIT \"-fuse-ld=lld -nostdlib -Wl,/nodefaultlib -Wl,/libpath:{} -Wl,/entry:mainCRTStartup -Wl,/dynamicbase -Wl,/nxcompat\")\n",
        slash(&sdk.join("lib"))
    ));
    t.push_str(&format!("set(CMAKE_C_STANDARD_LIBRARIES \"{} {STD_LIBS}\" CACHE STRING \"\" FORCE)\n", slash(crt)));
    // Zkusebni preklady (check_symbol_exists) jen do knihovny: bez linku.
    t.push_str("set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)\n");
    t
}

fn run(cmd: &mut Command, what: &str) -> Result<()> {
    let st = cmd.status()?;
    if !st.success() {
        return Err(format!("{what} selhal ({st})").into());
    }
    Ok(())
}

/// Prelozi `putty.exe` do `tests` (vedle `wintest.exe`). `out` =
/// `target/wine/<arch>` (s `sdk` z `build_wine.sh`).
pub fn build(out: &Path) -> Result<PathBuf> {
    let sdk = out.join("sdk");
    if !sdk.join("lib").join("winecrt0.lib").is_file() {
        return Err("chybi SDK Wine (target/wine/<arch>/sdk): nejdriv `cargo xtask wine`".into());
    }
    let cmake = crate::cports::tools::find(crate::cports::tools::Tool::CMake).ok_or("PuTTY: chybi cmake (winget install Kitware.CMake)")?;
    let ninja = crate::cports::tools::find(crate::cports::tools::Tool::Ninja).ok_or("PuTTY: chybi ninja (winget install Ninja-build.Ninja)")?;
    let clang = cbuild::find_clang()?;
    let src = crate::cports::fetch::prepare(&PUTTY)?;
    let work = target_dir().join("wine").join("putty");
    std::fs::create_dir_all(&work)?;
    // Prekladac prostredku: obalka llvm-rc s jinym jmenem. Pro `llvm-rc`
    // pouzije CMake pravidlo `cmake_llvm_rc`, ktere priznaky RC (`-c1252`
    // od PuTTY, `/I`) posle i clangu na predzpracovani; obalka dostane
    // pravidlo rc.exe a llvm-rc predzpracuje sam (clang vedle nej).
    let rc = work.join(if cfg!(windows) { "rc.cmd" } else { "rc.sh" });
    let llvm_rc = clang.with_file_name(format!("llvm-rc{}", std::env::consts::EXE_SUFFIX));
    let rc_text = if cfg!(windows) { format!("@\"{}\" %*\r\n", llvm_rc.display()) } else { format!("#!/bin/sh\nexec '{}' \"$@\"\n", llvm_rc.display()) };
    if std::fs::read_to_string(&rc).ok().as_deref() != Some(rc_text.as_str()) {
        std::fs::write(&rc, &rc_text)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&rc, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    // Start s WinMain a podpora TLS (wincrt.c, nas kod).
    let crt = work.join("wincrt.obj");
    let inc = sdk.join("include");
    run(
        Command::new(&clang)
            .args(["--target=x86_64-pc-windows-msvc", "-nostdlibinc", "-D_UCRT", "-mno-stack-arg-probe", "-O2", "-c"])
            .arg(format!("-I{}", slash(&inc)))
            .arg(format!("-I{}", slash(&inc.join("msvcrt"))))
            .arg(format!("-I{}", slash(&sdk.join("include-gen"))))
            .arg(crate::workspace_root().join("xtask/src/wine/wincrt.c"))
            .arg("-o")
            .arg(&crt),
        "preklad wincrt.c",
    )?;
    let tc_text = toolchain(&clang, &sdk, &rc, &crt);
    let tc = work.join("toolchain.cmake");
    if std::fs::read_to_string(&tc).ok().as_deref() != Some(tc_text.as_str()) {
        std::fs::write(&tc, &tc_text)?;
    }
    let build = work.join("build");
    let mut h = DefaultHasher::new();
    tc_text.hash(&mut h);
    src.hash(&mut h);
    PUTTY.sha256.hash(&mut h);
    let want = format!("{:016x}", h.finish());
    let stamp = build.join(".konfigurace");
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_dir_all(&build);
        std::fs::create_dir_all(&build)?;
        let mut c = Command::new(&cmake);
        c.arg("-G").arg("Ninja").arg("-S").arg(&src).arg("-B").arg(&build);
        c.arg(format!("-DCMAKE_MAKE_PROGRAM={}", slash(&ninja)));
        c.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", slash(&tc)));
        c.arg("-DCMAKE_BUILD_TYPE=Release");
        // GSSAPI (Kerberos) test nepotrebuje; bez git popisu verze (zdroj
        // je v target/ naseho repozitare).
        c.arg("-DPUTTY_GSSAPI=OFF");
        c.env("GIT_CEILING_DIRECTORIES", slash(&work));
        run(&mut c, "cmake (konfigurace PuTTY)")?;
        std::fs::write(&stamp, &want)?;
    }
    run(Command::new(&cmake).arg("--build").arg(&build).arg("--target").arg("putty"), "cmake --build PuTTY")?;
    let tests = out.join("tests");
    std::fs::create_dir_all(&tests)?;
    let dst = tests.join("putty.exe");
    std::fs::copy(build.join("putty.exe"), &dst)?;
    println!("xtask: PuTTY {} -> {}", PUTTY.top, dst.display());
    Ok(dst)
}
