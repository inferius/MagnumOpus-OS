//! Sdilene knihovny (`ET_DYN`, `-shared`) a programy PIE v C: preklad
//! s `-fPIC` a link primo pres `ld.lld` (`docs/cs/dynamicke-linkovani.md`).
//!
//! Proc `ld.lld` primo a ne driver clang: pro cile `*-unknown-none-elf`
//! clang `-shared` ignoruje (baremetal toolchain) a cil linuxu by tahl
//! predpoklady hostitele (crt, libc). Tady jsou vsechny priznaky linkeru
//! na jednom miste a stejne pro x86_64 i aarch64.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Build, find_clang, write_rsp};

/// Cesta zavadece v programech PIE (`PT_INTERP`).
pub const INTERP: &str = "/system/lib/ld.so";

/// Co se linkuje.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// Sdilena knihovna; soubor se jmenuje presne jako `soname` (s hlavni
    /// verzi, napr. `libavcodec.so.61`).
    Shared { soname: String },
    /// Program PIE se zavadecem [`INTERP`].
    Pie,
}

/// Popis linkovani.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// Knihovny, na kterych modul zavisi (`DT_NEEDED` = jejich SONAME).
    pub needed: Vec<PathBuf>,
    /// `DT_RUNPATH` (napr. `$ORIGIN`); `None` = jen `/system/lib`.
    pub runpath: Option<String>,
    /// Dalsi objekty (napr. crt programu) pred objekty knihovny.
    pub extra_objects: Vec<PathBuf>,
}

impl Link {
    pub fn shared(soname: &str) -> Self {
        Self { kind: LinkKind::Shared { soname: soname.into() }, needed: Vec::new(), runpath: None, extra_objects: Vec::new() }
    }

    pub fn pie() -> Self {
        Self { kind: LinkKind::Pie, needed: Vec::new(), runpath: None, extra_objects: Vec::new() }
    }

    pub fn needs(mut self, lib: impl Into<PathBuf>) -> Self {
        self.needed.push(lib.into());
        self
    }

    pub fn runpath(mut self, p: &str) -> Self {
        self.runpath = Some(p.into());
        self
    }

    pub fn object(mut self, o: impl Into<PathBuf>) -> Self {
        self.extra_objects.push(o.into());
        self
    }
}

/// Priznaky `ld.lld` (bez objektu a vystupu). Pro kazdy modul:
/// - `-z now` + `-z relro`: vse se resi hned, GOT je po relokaci jen pro
///   cteni (pecet RELRO); line PLT zavadec nema,
/// - `-z text`: relokace v kodu (TEXTREL) = chyba linku (W^X; zavadec je
///   by stejne odmitl),
/// - `-z max-page-size=65536`: segmenty po 64 KiB jako programy systemu,
/// - `--hash-style=both`: GNU hash (rychle hledani) i SysV,
/// - `-z pack-relative-relocs`: relativni relokace jako RELR (mensi soubor),
/// - `--gc-sections`, `--build-id=none`: bez nepouzitych sekci, stabilni
///   vystup.
pub fn link_flags(target: &str, link: &Link) -> Result<Vec<String>, String> {
    let emulation = match target {
        "x86_64-unknown-magnumopus" => "elf_x86_64",
        "aarch64-unknown-magnumopus" => "aarch64elf",
        _ => return Err(format!("cbuild: sdilene knihovny jen pro programy (*-unknown-magnumopus), ne {target}")),
    };
    let mut f: Vec<String> = [
        "-m",
        emulation,
        "-z",
        "now",
        "-z",
        "relro",
        "-z",
        "text",
        "-z",
        "max-page-size=65536",
        "-z",
        "noexecstack",
        "--hash-style=both",
        "-z",
        "pack-relative-relocs",
        "--gc-sections",
        "--build-id=none",
        "--eh-frame-hdr",
    ]
    .map(String::from)
    .to_vec();
    match &link.kind {
        LinkKind::Shared { soname } => {
            f.push("-shared".into());
            f.push(format!("-soname={soname}"));
        }
        LinkKind::Pie => {
            f.push("-pie".into());
            f.push(format!("--dynamic-linker={INTERP}"));
            // Knihovny odkazuji na funkce zavadece (`__tls_get_addr`), ktere
            // pri linku nejsou v zadne knihovne; chybejici symbol ohlasi
            // zavadec pri startu (`ld.so: nedefinovany symbol ...`).
            f.push("--allow-shlib-undefined".into());
        }
    }
    if let Some(r) = &link.runpath {
        f.push("--enable-new-dtags".into());
        f.push(format!("-rpath={r}"));
    }
    Ok(f)
}

/// `ld.lld` vedle clangu (instalace LLVM ho ma), jinak z PATH.
pub fn find_lld(clang: &Path) -> PathBuf {
    let exe = if cfg!(windows) { "ld.lld.exe" } else { "ld.lld" };
    if let Some(dir) = clang.parent() {
        let p = dir.join(exe);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(exe)
}

impl Build {
    /// Prelozi zdroje s `-fPIC` (objekty v `out_dir`) a slinkuje modul
    /// `out` pro cil Rustu `target` (`*-unknown-magnumopus`). Vola ho xtask
    /// (sdilene knihovny a programy PIE do obrazu), ne build.rs.
    pub fn link(&self, target: &str, out_dir: &Path, out: &Path, link: &Link) -> Result<(), String> {
        let clang = find_clang()?;
        let flags = link_flags(target, link)?;
        let code = match link.kind {
            LinkKind::Shared { .. } => crate::Code::Shared,
            LinkKind::Pie => crate::Code::Pie,
        };
        let (obj_dir, mut objects) = self.compile_objects(&clang, target, out_dir, code)?;
        let mut all = link.extra_objects.clone();
        all.append(&mut objects);
        let rsp = write_rsp(&obj_dir, &all)?;
        let lld = find_lld(&clang);
        let o = Command::new(&lld)
            .args(&flags)
            .arg(format!("@{}", rsp.display()))
            .args(&link.needed)
            .arg("-o")
            .arg(out)
            .output()
            .map_err(|e| format!("{} nejde spustit: {e}", lld.display()))?;
        if !o.status.success() {
            return Err(format!("ld.lld ({}) selhal:\n{}{}", self.name, String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)));
        }
        Ok(())
    }
}
