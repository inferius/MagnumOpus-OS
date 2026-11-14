//! Preklad kodu v C pro nase cile (z `build.rs`).
//!
//! Jedno misto, kde se rozhoduje, jak se C v systemu preklada: prekladac
//! (clang), cil, priznaky freestanding a bezpecnostni priznaky. Crate,
//! ktery obsahuje C (napr. `libs/acpica`), ma `build.rs` s par radky:
//!
//! ```ignore
//! cbuild::Build::new("acpica")
//!     .trust(cbuild::Trust::Vendored)
//!     .include("include")
//!     .files(&sources)
//!     .compile();
//! ```
//!
//! Vysledkem je staticka knihovna `lib<jmeno>.a` v `OUT_DIR`, kterou cargo
//! prilinkuje k programu (linkuje rust-lld spolu s kodem v Rustu).
//!
//! Proc vlastni modul a ne crate `cc`: cile `*-unknown-magnumopus`
//! a `*-unknown-none*` `cc` nezna jako "nas system", priznaky
//! by se skladaly na vic mistech a na Windows by hledal MSVC. Tady je
//! seznam priznaku na jednom miste a popsany (`docs/cs/preklad-c.md`).
//!
//! Prekladac: promenna `CLANG`, jinak `clang` v PATH, jinak bezne
//! instalacni misto LLVM. Archivator `llvm-ar` se hleda vedle clangu.

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

/// Jak moc kodu verime - urcuje varovani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Nas vlastni kod: `-Wall -Wextra -Werror` (varovani = chyba buildu).
    Own,
    /// Prevzaty kod (third_party): varovani se nevypisuji, kod se nemeni.
    /// Bezpecnostni priznaky plati stejne.
    Vendored,
}

/// Popis jedne knihovny v C.
pub struct Build {
    name: String,
    files: Vec<PathBuf>,
    includes: Vec<PathBuf>,
    /// Hlavicky prevzateho kodu (`-isystem`): jejich varovani se
    /// nehlasi ani vlastnimu kodu, ktery je vklada.
    system_includes: Vec<PathBuf>,
    defines: Vec<(String, Option<String>)>,
    flags: Vec<String>,
    trust: Trust,
}

/// Priznaky pro cil Rustu (`TARGET` z cargo).
///
/// - `*-unknown-magnumopus` (programy v userspace): hard-float ABI jako
///   Rust (x86_64: zakladni uroven x86-64-v2 = SSE4.2, POPCNT, CMPXCHG16B;
///   aarch64: NEON), takze C a Rust si predavaji `float`/`double`
///   v registrech FPU stejne. Jadro stav FPU vlaken uklada.
/// - `*-unknown-none*` (jadro): bez FPU a SIMD - jadro nesmi sahat na
///   registry FPU programu.
///
/// Na aarch64 vsude striktni zarovnani jako cil Rustu.
pub fn target_flags(target: &str) -> Result<&'static [&'static str], String> {
    match target {
        "x86_64-unknown-magnumopus" => Ok(&[
            "--target=x86_64-unknown-none-elf",
            "-march=x86-64-v2",
            "-mcmodel=small",
            // Nativni TLS v modelu local-exec (staticky ET_EXEC; `_Thread_local`
            // a `errno` v C sdili bloky TLS s Rustem, docs/cs/rust-std.md).
            "-ftls-model=local-exec",
        ]),
        "aarch64-unknown-magnumopus" => Ok(&[
            "--target=aarch64-unknown-none-elf",
            "-march=armv8-a+simd",
            "-mstrict-align",
            "-ftls-model=local-exec",
        ]),
        "x86_64-unknown-none" => Ok(&[
            "--target=x86_64-unknown-none-elf",
            "-mgeneral-regs-only",
            "-mno-red-zone",
            "-mcmodel=small",
        ]),
        "aarch64-unknown-none-softfloat" | "aarch64-unknown-none" => Ok(&[
            "--target=aarch64-unknown-none-elf",
            "-mgeneral-regs-only",
            "-mstrict-align",
        ]),
        _ => Err(format!(
            "cbuild: cil {target} neni podporovany (programy *-unknown-magnumopus, jadro x86_64-unknown-none a aarch64-unknown-none-softfloat)"
        )),
    }
}

/// Spolecne priznaky pro veskery kod v C (popis v `docs/cs/preklad-c.md`).
pub const COMMON_FLAGS: &[&str] = &[
    "-std=c11",
    // Bez systemovych hlavicek hostitele (Windows SDK, glibc): jen hlavicky
    // clangu (stddef.h, stdint.h, stdarg.h, ...) a nase (`libs/cstd-core/include`).
    "-ffreestanding",
    "-nostdlibinc",
    "-fno-pic",
    "-fno-common",
    "-ffunction-sections",
    "-fdata-sections",
    // AML interpret a podobny kod je bez optimalizace mnohonasobne pomalejsi;
    // v QEMU (TCG) by to bylo znat.
    "-O2",
    // --- Bezpecnost (viz docs/cs/preklad-c.md, "Jak se hlida bezpecnost") ---
    // Kanarek na zasobniku u funkci s poli a adresami lokalnich promennych;
    // hodnota je globalni `__stack_chk_guard` (libs/cstd-core), ne TLS.
    "-fstack-protector-strong",
    "-mstack-protector-guard=global",
    // Neinicializovane lokalni promenne = nuly (zadny unik starych dat
    // a predvidatelne chovani pri chybe).
    "-ftrivial-auto-var-init=zero",
    // Prekladac nesmi vyhazovat kontroly ukazatelu a preteceni podle
    // "nedefinovaneho chovani" (stary kod v C na nem casto stoji).
    "-fno-strict-aliasing",
    "-fwrapv",
    "-fno-delete-null-pointer-checks",
    // Nezname funkce bez prototypu jsou chyba (implicitni deklarace by vratila
    // int a usekla ukazatel).
    "-Werror=implicit-function-declaration",
    "-Werror=incompatible-pointer-types",
    "-Werror=int-conversion",
    "-Werror=return-type",
];

/// Najde clang: `CLANG`, PATH, bezna mista instalace LLVM.
pub fn find_clang() -> Result<PathBuf, String> {
    if let Some(p) = env::var_os("CLANG") {
        let p = PathBuf::from(p);
        return if p.exists() { Ok(p) } else { Err(format!("CLANG={} neexistuje", p.display())) };
    }
    let exe = if cfg!(windows) { "clang.exe" } else { "clang" };
    if let Some(path) = env::var_os("PATH") {
        for dir in env::split_paths(&path) {
            let p = dir.join(exe);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    let known: &[&str] = if cfg!(windows) {
        &["C:\\Program Files\\LLVM\\bin\\clang.exe"]
    } else {
        &["/usr/bin/clang", "/usr/local/bin/clang", "/opt/homebrew/opt/llvm/bin/clang"]
    };
    known
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| {
            String::from(
                "cbuild: clang nenalezen. Nainstalujte LLVM (Windows: winget install LLVM.LLVM, \
                 Linux: balicek clang a llvm) nebo nastavte promennou CLANG na cestu k clangu.",
            )
        })
}

/// `llvm-ar` vedle clangu (instalace LLVM ho ma vzdy), jinak z PATH.
fn find_ar(clang: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "llvm-ar.exe" } else { "llvm-ar" };
    if let Some(dir) = clang.parent() {
        let p = dir.join(exe);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Nejnovejsi cas zmeny souboru v adresari (rekurzivne): zmena kterekoli
/// hlavicky = prelozit vse znovu.
fn newest_in(dir: &Path, best: &mut Option<SystemTime>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            newest_in(&p, best);
        } else if let Some(t) = mtime(&p) {
            *best = Some(best.map_or(t, |b| b.max(t)));
        }
    }
}

impl Build {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            files: Vec::new(),
            includes: Vec::new(),
            system_includes: Vec::new(),
            defines: Vec::new(),
            flags: Vec::new(),
            trust: Trust::Own,
        }
    }

    pub fn trust(mut self, t: Trust) -> Self {
        self.trust = t;
        self
    }

    pub fn file(mut self, p: impl Into<PathBuf>) -> Self {
        self.files.push(p.into());
        self
    }

    pub fn files<P: Into<PathBuf> + Clone>(mut self, ps: &[P]) -> Self {
        self.files.extend(ps.iter().cloned().map(Into::into));
        self
    }

    pub fn include(mut self, p: impl Into<PathBuf>) -> Self {
        self.includes.push(p.into());
        self
    }

    /// Adresar hlavicek prevzateho kodu (`-isystem`).
    pub fn include_system(mut self, p: impl Into<PathBuf>) -> Self {
        self.system_includes.push(p.into());
        self
    }

    pub fn define(mut self, name: &str, value: Option<&str>) -> Self {
        self.defines.push((name.into(), value.map(Into::into)));
        self
    }

    /// Dalsi priznak prekladace (jen kdyz nestaci spolecne; uvest proc).
    pub fn flag(mut self, f: &str) -> Self {
        self.flags.push(f.into());
        self
    }

    /// Vsechny priznaky prekladace pro cil.
    fn command_flags(&self, target: &str) -> Result<Vec<String>, String> {
        let mut f: Vec<String> = target_flags(target)?.iter().map(|s| s.to_string()).collect();
        f.extend(COMMON_FLAGS.iter().map(|s| s.to_string()));
        match self.trust {
            Trust::Own => f.extend(["-Wall", "-Wextra", "-Werror"].map(String::from)),
            Trust::Vendored => f.push("-w".into()),
        }
        if env::var("DEBUG").is_ok_and(|v| v == "true") {
            f.push("-g".into());
        }
        for i in &self.includes {
            f.push(format!("-I{}", i.display()));
        }
        for i in &self.system_includes {
            f.push(String::from("-isystem"));
            f.push(i.display().to_string());
        }
        for (n, v) in &self.defines {
            f.push(match v {
                Some(v) => format!("-D{n}={v}"),
                None => format!("-D{n}"),
            });
        }
        f.extend(self.flags.iter().cloned());
        Ok(f)
    }

    /// Prelozi a vytvori `lib<jmeno>.a`; cargo ji prilinkuje. Pri chybe
    /// ukonci build.rs s vypisem prekladace.
    pub fn compile(self) {
        if let Err(e) = self.try_compile() {
            eprintln!("{e}");
            panic!("cbuild: preklad knihovny {} selhal", self.name);
        }
    }

    fn try_compile(&self) -> Result<(), String> {
        let target = env::var("TARGET").map_err(|_| "cbuild: chybi TARGET (volat z build.rs)")?;
        let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("cbuild: chybi OUT_DIR")?);
        let clang = find_clang()?;
        let flags = self.command_flags(&target)?;
        println!("cargo:rerun-if-env-changed=CLANG");
        for f in &self.files {
            println!("cargo:rerun-if-changed={}", f.display());
        }
        for i in self.includes.iter().chain(&self.system_includes) {
            println!("cargo:rerun-if-changed={}", i.display());
        }

        // Klic predchoziho buildu: priznaky + prekladac + nejnovejsi
        // hlavicka. Jiny klic = vse znovu; jinak jen zmenene zdroje.
        let mut newest = None;
        for i in self.includes.iter().chain(&self.system_includes) {
            newest_in(i, &mut newest);
        }
        let mut h = DefaultHasher::new();
        flags.hash(&mut h);
        clang.hash(&mut h);
        newest.hash(&mut h);
        let key = format!("{:016x}", h.finish());
        let obj_dir = out.join(format!("obj-{}", self.name));
        std::fs::create_dir_all(&obj_dir).map_err(|e| e.to_string())?;
        let stamp = obj_dir.join("stamp");
        let fresh = std::fs::read_to_string(&stamp).is_ok_and(|s| s == key);

        let objects: Vec<PathBuf> = self
            .files
            .iter()
            .map(|src| {
                let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                // Stejne jmeno v ruznych adresarich: pridat hash cesty.
                let mut ph = DefaultHasher::new();
                src.hash(&mut ph);
                obj_dir.join(format!("{stem}-{:08x}.o", ph.finish() as u32))
            })
            .collect();
        let todo: Vec<usize> = (0..self.files.len())
            .filter(|&i| {
                !fresh || match (mtime(&self.files[i]), mtime(&objects[i])) {
                    (Some(s), Some(o)) => s > o,
                    _ => true,
                }
            })
            .collect();

        let next = AtomicUsize::new(0);
        let errors = Mutex::new(Vec::new());
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
        std::thread::scope(|s| {
            for _ in 0..workers.min(todo.len().max(1)) {
                s.spawn(|| {
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&i) = todo.get(k) else { break };
                        let r = Command::new(&clang)
                            .args(&flags)
                            .arg("-c")
                            .arg(&self.files[i])
                            .arg("-o")
                            .arg(&objects[i])
                            .output();
                        let msg = match r {
                            Ok(o) if o.status.success() => continue,
                            Ok(o) => format!(
                                "{}:\n{}{}",
                                self.files[i].display(),
                                String::from_utf8_lossy(&o.stdout),
                                String::from_utf8_lossy(&o.stderr)
                            ),
                            Err(e) => format!("clang {} nejde spustit: {e}", clang.display()),
                        };
                        errors.lock().unwrap().push(msg);
                    }
                });
            }
        });
        let errors = errors.into_inner().unwrap();
        if !errors.is_empty() {
            let _ = std::fs::remove_file(&stamp);
            return Err(errors.join("\n"));
        }
        std::fs::write(&stamp, &key).map_err(|e| e.to_string())?;

        let lib = out.join(format!("lib{}.a", self.name));
        let _ = std::fs::remove_file(&lib);
        let ar = find_ar(&clang);
        // Seznam objektu v souboru odpovedi: stovky dlouhych cest by na
        // Windows prekrocily delku prikazove radky (32 KiB).
        let rsp = obj_dir.join("objects.rsp");
        let list: Vec<String> = objects.iter().map(|o| format!("\"{}\"", o.display().to_string().replace('\\', "/"))).collect();
        std::fs::write(&rsp, list.join("\n")).map_err(|e| e.to_string())?;
        let o = Command::new(&ar)
            .arg("rcs")
            .arg(&lib)
            .arg(format!("@{}", rsp.display()))
            .output()
            .map_err(|e| format!("{} nejde spustit: {e}", ar.display()))?;
        if !o.status.success() {
            return Err(format!("llvm-ar selhal: {}", String::from_utf8_lossy(&o.stderr)));
        }
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static={}", self.name);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
