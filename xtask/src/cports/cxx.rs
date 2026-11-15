//! Beh C++ (krok 7a, docs/cs/cpp.md): libunwind, libc++abi a libc++
//! z `llvm-project` (stejna verze jako clang hostitele) a program `cpptest`.
//!
//! Dve konfigurace CMake nad `runtimes/`:
//!
//! - staticka (`toolchain.cmake`, bez PIC): `libc++.a`, `libc++abi.a`,
//!   `libunwind.a` a hlavicky `include/c++/v1` v predpone portu,
//! - dynamicka (`toolchain-dyn.cmake`, PIC nad `libc.so`): jedna sdilena
//!   `libc++.so` (libc++ + libc++abi + libunwind uvnitr, jako Android)
//!   v predpone `prefix-dyn`, do initfs jako `/system/lib/libc++.so`.
//!
//! Etapa 7a-1 je bez sirokych znaku a locale (`<iostream>`, `<locale>`
//! az 7a-2 s funkcemi wchar a locale v cstd).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::cmake::{run, slash, toolchain_for, write_if_changed};
use super::tools::{Tool, require};
use super::{Build, CPort, Output, fetch, prefix};
use crate::arch::Arch;
use crate::{Result, csysroot, target_dir, workspace_root};

/// llvm-project 23.1.5 = clang hostitele (ABI hlavicek a vestavenych
/// funkci sedi).
pub const LLVM: CPort = CPort {
    name: "llvm-runtimes",
    url: "https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/llvm-project-23.1.5.src.tar.xz",
    file: "llvm-project-23.1.5.src.tar.xz",
    sha256: "fb50b1eac93ddeb3aa8dab4cfe324dff74f4a9c2bdee6160c9e36188f8d06a6e",
    top: "llvm-project-23.1.5.src",
    // random_device nad getentropy, chrono nad clock_gettime (__magnumopus__).
    patches: &["0001-magnumopus.patch"],
    build: Build::CMake { options: &[], programs: &[] },
    data: &[],
};

/// Casti archivu, ktere runtime potrebuje (z 179 MB asi 40 MB).
const MEMBERS: &[&str] = &[
    "llvm-project-23.1.5.src/runtimes",
    "llvm-project-23.1.5.src/libcxx",
    "llvm-project-23.1.5.src/libcxxabi",
    "llvm-project-23.1.5.src/libunwind",
    "llvm-project-23.1.5.src/cmake",
    "llvm-project-23.1.5.src/llvm/cmake",
    "llvm-project-23.1.5.src/llvm/utils/llvm-lit",
    // Sdilene hlavicky LLVM libc (libc++ z nich bere prevod textu na
    // plovouci carku pro std::from_chars).
    "llvm-project-23.1.5.src/libc",
];

/// Testy a dokumentace runtime se nerozbaluji.
const EXCLUDE: &[&str] = &[
    "*/libcxx/test",
    "*/libcxx/docs",
    "*/libcxxabi/test",
    "*/libunwind/test",
    "*/libunwind/docs",
    "*/libc/test",
    "*/libc/docs",
    "*/libc/benchmarks",
    "*/libc/fuzzing",
    "*/libc/utils",
];

/// Volby spolecne obema konfiguracim.
const COMMON: &[&str] = &[
    "-DLLVM_ENABLE_RUNTIMES=libunwind;libcxxabi;libcxx",
    "-DLLVM_ENABLE_PER_TARGET_RUNTIME_DIR=OFF",
    "-DLLVM_INCLUDE_TESTS=OFF",
    "-DLLVM_INCLUDE_DOCS=OFF",
    "-DLIBCXX_INCLUDE_TESTS=OFF",
    "-DLIBCXX_INCLUDE_BENCHMARKS=OFF",
    "-DLIBCXXABI_INCLUDE_TESTS=OFF",
    "-DLIBUNWIND_INCLUDE_TESTS=OFF",
    "-DLIBUNWIND_INCLUDE_DOCS=OFF",
    // Vlakna nad pthread z cstd (Q7 A).
    "-DLIBCXX_HAS_PTHREAD_API=ON",
    "-DLIBCXXABI_HAS_PTHREAD_API=ON",
    // Bez libgcc / compiler-rt: pomocne funkce prekladace jsou v libc.a
    // (staticky) a v libbuiltins.a (dynamicky).
    "-DLIBUNWIND_USE_COMPILER_RT=OFF",
    "-DLIBCXXABI_USE_COMPILER_RT=OFF",
    "-DLIBCXX_USE_COMPILER_RT=OFF",
    // Zkusebni preklady jsou jen do statickych knihoven (link nezkousi),
    // takze CMake by "nasel" kazdou knihovnu: libgcc, libdl, libatomic ani
    // samostatne libpthread/librt nejsou (vse je v libc.a / libc.so, ta se
    // pridava sama jako standardni knihovna - "-lc" by v libc++.so
    // staticky prilinkoval druhou kopii runtime).
    "-DLIBCXX_HAS_GCC_LIB=OFF",
    "-DLIBCXX_HAS_GCC_S_LIB=OFF",
    "-DLIBCXX_HAS_ATOMIC_LIB=OFF",
    "-DLIBCXX_HAS_PTHREAD_LIB=OFF",
    "-DLIBCXX_HAS_RT_LIB=OFF",
    "-DLIBCXXABI_HAS_C_LIB=OFF",
    "-DLIBCXXABI_HAS_GCC_LIB=OFF",
    "-DLIBCXXABI_HAS_GCC_S_LIB=OFF",
    "-DLIBCXXABI_HAS_DL_LIB=OFF",
    "-DLIBCXXABI_HAS_PTHREAD_LIB=OFF",
    "-DLIBUNWIND_HAS_C_LIB=OFF",
    "-DLIBUNWIND_HAS_ROOT_LIB=OFF",
    "-DLIBUNWIND_HAS_GCC_LIB=OFF",
    "-DLIBUNWIND_HAS_GCC_S_LIB=OFF",
    "-DLIBUNWIND_HAS_DL_LIB=OFF",
    "-DLIBUNWIND_HAS_PTHREAD_LIB=OFF",
    "-DLIBUNWIND_HAS_BSD_LIB=OFF",
    "-DLIBCXXABI_USE_LLVM_UNWINDER=ON",
    "-DLIBCXX_CXX_ABI=libcxxabi",
    // 7a-1: bez sirokych znaku a locale (7a-2 po cstd-wchar).
    "-DLIBCXX_ENABLE_WIDE_CHARACTERS=OFF",
    "-DLIBCXX_ENABLE_LOCALIZATION=OFF",
    "-DLIBCXX_ENABLE_TIME_ZONE_DATABASE=OFF",
    "-DLIBCXX_ENABLE_ABI_LINKER_SCRIPT=OFF",
    "-DLIBCXX_INSTALL_MODULES=OFF",
    "-DLIBCXX_ENABLE_VENDOR_AVAILABILITY_ANNOTATIONS=OFF",
];

/// Staticke knihovny pro programy ET_EXEC.
const STATIC: &[&str] = &[
    "-DLIBUNWIND_ENABLE_SHARED=OFF",
    "-DLIBUNWIND_ENABLE_STATIC=ON",
    "-DLIBCXXABI_ENABLE_SHARED=OFF",
    "-DLIBCXXABI_ENABLE_STATIC=ON",
    "-DLIBCXXABI_ENABLE_STATIC_UNWINDER=ON",
    "-DLIBCXX_ENABLE_SHARED=OFF",
    "-DLIBCXX_ENABLE_STATIC=ON",
];

/// Jedna sdilena `libc++.so` s abi a unwinderem uvnitr.
const SHARED: &[&str] = &[
    "-DLIBUNWIND_ENABLE_SHARED=OFF",
    "-DLIBUNWIND_ENABLE_STATIC=ON",
    "-DLIBCXXABI_ENABLE_SHARED=OFF",
    "-DLIBCXXABI_ENABLE_STATIC=ON",
    "-DLIBCXXABI_ENABLE_STATIC_UNWINDER=ON",
    "-DLIBCXXABI_STATICALLY_LINK_UNWINDER_IN_SHARED_LIBRARY=ON",
    "-DLIBCXX_ENABLE_SHARED=ON",
    "-DLIBCXX_ENABLE_STATIC=OFF",
    "-DLIBCXX_STATICALLY_LINK_ABI_IN_SHARED_LIBRARY=ON",
    // libc++.so je v DT_NEEDED od startu (staticke TLS zavadece) jako
    // libc.so: TLS initial-exec, bez __tls_get_addr (libc++ linkuje
    // s -z defs a __tls_get_addr dava az ld.so). dlopen libc++.so do
    // procesu bez ni pri startu by potreboval rezervu static_tls_reserve.
    "-DLIBCXX_ADDITIONAL_COMPILE_FLAGS=-ftls-model=initial-exec",
    "-DLIBCXXABI_ADDITIONAL_COMPILE_FLAGS=-ftls-model=initial-exec",
    "-DLIBUNWIND_ADDITIONAL_COMPILE_FLAGS=-ftls-model=initial-exec",
];

/// Predpona dynamicke konfigurace (`libc++.so` a jeji hlavicky).
pub fn prefix_dyn(arch: &Arch) -> PathBuf {
    target_dir().join("c-ports").join(arch.name).join("prefix-dyn")
}

/// Priznaky prekladu C++ proti predpone (`dynamic` = PIE nad libc.so):
/// hlavicky libc++ pred hlavickami C (obalove hlavicky `<cstdio>` ...
/// pouzivaji `#include_next`).
pub fn cxxflags(arch: &Arch, sysroot: &Path, dynamic: bool) -> Result<Vec<String>> {
    let mut f = if dynamic { csysroot::cflags_dyn(arch, sysroot)? } else { csysroot::cflags(arch, sysroot)? };
    let pfx = if dynamic { prefix_dyn(arch) } else { prefix(arch) };
    let at = f.iter().position(|x| x == "-isystem").unwrap_or(f.len());
    f.splice(at..at, [String::from("-nostdinc++"), String::from("-isystem"), slash(&pfx.join("include/c++/v1"))]);
    Ok(f)
}

/// Priznaky linkovani programu v C++ (knihovny C++ pred knihovnou C).
pub fn cxx_ldflags(arch: &Arch, sysroot: &Path, dynamic: bool) -> Vec<String> {
    if dynamic {
        let mut f = csysroot::ldflags_dyn(sysroot);
        let at = f.iter().position(|x| x == "-l:libc.so").unwrap_or(f.len());
        f.splice(at..at, [format!("-L{}", slash(&prefix_dyn(arch).join("lib"))), String::from("-l:libc++.so")]);
        f
    } else {
        let mut f = csysroot::ldflags(sysroot);
        let at = f.iter().position(|x| x == "-lc").unwrap_or(f.len());
        f.splice(at..at, [format!("-L{}", slash(&prefix(arch).join("lib"))), "-lc++".into(), "-lc++abi".into(), "-lunwind".into()]);
        f
    }
}

/// Konfigurace a sestaveni jedne konfigurace runtimes; instalace do `pfx`.
fn build_config(arch: &Arch, sysroot: &Path, src: &Path, work: &Path, pfx: &Path, dynamic: bool) -> Result<()> {
    let found = require(LLVM.name, &[Tool::CMake, Tool::Ninja])?;
    let (cmake, ninja) = (&found[0], &found[1]);
    let tc = toolchain_for(arch, sysroot, dynamic)?;
    let build = work.join(if dynamic { "build-dyn" } else { "build" });
    let extra = if dynamic { SHARED } else { STATIC };
    let mut h = DefaultHasher::new();
    COMMON.hash(&mut h);
    extra.hash(&mut h);
    std::fs::read(&tc)?.hash(&mut h);
    src.hash(&mut h);
    let stamp = build.join(".konfigurace");
    let want = format!("{:016x}", h.finish());
    if std::fs::read_to_string(&stamp).ok().as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_dir_all(&build);
        std::fs::create_dir_all(&build)?;
        let mut c = Command::new(cmake);
        c.arg("-G").arg("Ninja").arg("-S").arg(src.join("runtimes")).arg("-B").arg(&build);
        c.arg(format!("-DCMAKE_MAKE_PROGRAM={}", slash(ninja)));
        c.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", slash(&tc)));
        c.arg(format!("-DCMAKE_INSTALL_PREFIX={}", slash(pfx)));
        c.arg("-DCMAKE_BUILD_TYPE=Release");
        c.args(COMMON).args(extra);
        run(&mut c, &format!("cmake (konfigurace {}{})", LLVM.name, if dynamic { " dyn" } else { "" }))?;
        std::fs::write(&stamp, &want)?;
    }
    run(Command::new(cmake).arg("--build").arg(&build), &format!("cmake --build {}", LLVM.name))?;
    run(Command::new(cmake).arg("--install").arg(&build), &format!("cmake --install {}", LLVM.name))?;
    Ok(())
}

/// `clang++.cfg` (staticky) a `clang++-dyn.cfg` (PIE nad libc.so) v
/// predpone portu pro cizi build systemy: `clang++ --config=...`.
fn write_cfg(arch: &Arch, sysroot: &Path) -> Result<()> {
    for dynamic in [false, true] {
        let mut cfg = String::from("# C++ pro MagnumOpus: clang++ --config=<tento soubor> (preklad i link).\n");
        for f in cxxflags(arch, sysroot, dynamic)?.into_iter().chain(cxx_ldflags(arch, sysroot, dynamic)) {
            cfg.push_str(&f);
            cfg.push('\n');
        }
        let name = if dynamic { "clang++-dyn.cfg" } else { "clang++.cfg" };
        write_if_changed(&prefix(arch).join(name), cfg.as_bytes())?;
    }
    Ok(())
}

/// Prelozi a slinkuje program v C++; zdroje `.c` prekladac C (ramce C
/// s tabulkami rozbalovani), ostatni clang++.
fn build_program(arch: &Arch, sysroot: &Path, srcs: &[PathBuf], out: &Path, dynamic: bool) -> Result<()> {
    let clang = cbuild::find_clang()?;
    let clangxx = super::cmake::llvm_tool(&clang, "clang++");
    let mut objs = Vec::new();
    for s in srcs {
        let is_c = s.extension().is_some_and(|e| e == "c");
        let obj = out.with_file_name(format!(
            "{}-{}.o",
            out.file_name().unwrap_or_default().to_string_lossy(),
            s.file_stem().unwrap_or_default().to_string_lossy()
        ));
        let mut c = Command::new(if is_c { &clang } else { &clangxx });
        if is_c {
            c.args(if dynamic { csysroot::cflags_dyn(arch, sysroot)? } else { csysroot::cflags(arch, sysroot)? });
            c.args(["-O2", "-Wall"]);
        } else {
            c.args(cxxflags(arch, sysroot, dynamic)?).args(["-O2", "-std=c++20", "-Wall"]);
        }
        let o = c.arg("-c").arg(s).arg("-o").arg(&obj).output()?;
        if !o.status.success() {
            return Err(format!("preklad {} selhal:\n{}", s.display(), String::from_utf8_lossy(&o.stderr)).into());
        }
        objs.push(obj);
    }
    let o = Command::new(&clangxx)
        .args(cbuild::target_flags(arch.user_target)?)
        .args(&objs)
        .args(cxx_ldflags(arch, sysroot, dynamic))
        .arg("-o")
        .arg(out)
        .output()?;
    if !o.status.success() {
        return Err(format!("preklad {} selhal:\n{}", out.display(), String::from_utf8_lossy(&o.stderr)).into());
    }
    Ok(())
}

/// Sestavi beh C++ pro architekturu (staticky i `libc++.so`) a testovaci
/// programy; vraci vystupy do initfs.
pub fn build(arch: &Arch, sysroot: &Path) -> Result<Vec<Output>> {
    let src = fetch::prepare_subset(&LLVM, MEMBERS, EXCLUDE)?;
    let work = target_dir().join("c-ports").join(arch.name).join(LLVM.name);
    std::fs::create_dir_all(&work)?;
    println!("xtask: port C++ {} ({})", LLVM.name, arch.name);
    build_config(arch, sysroot, &src, &work, &prefix(arch), false)?;
    build_config(arch, sysroot, &src, &work, &prefix_dyn(arch), true)?;
    write_cfg(arch, sysroot)?;
    let test_src = [workspace_root().join("user/cpptest/cpptest.cpp"), workspace_root().join("user/cpptest/cframe.c")];
    let exe = work.join("cpptest");
    build_program(arch, sysroot, &test_src, &exe, false)?;
    let exe_dyn = work.join("cpptest-dyn");
    build_program(arch, sysroot, &test_src, &exe_dyn, true)?;
    Ok(vec![
        Output::Program(String::from("cpptest"), exe),
        // ET_DYN cele (zavadec cte dynamickou cast).
        Output::Data(String::from("cpptest-dyn"), exe_dyn),
        Output::Data(String::from("lib/libc++.so"), prefix_dyn(arch).join("lib/libc++.so")),
    ])
}
