//! Porty programu a knihoven v C prelozene proti sysrootu C
//! (`csysroot.rs`): obecny mechanismus (docs/cs/porty-c.md).
//!
//! Port = archiv z oficialniho mista (HTTPS + SHA-256), zaplaty a vlastni
//! soubory z `third_party/c-ports/<port>/` (`fetch.rs`) a jeden ze zpusobu
//! sestaveni:
//!
//! - [`Build::Sources`] - seznam zdroju primo (`sources.rs`), bez nastroju
//!   navic (zlib, sqlite),
//! - [`Build::CMake`] - CMake + Ninja se souborem nastroju sysrootu
//!   (`cmake.rs`, SDL),
//! - [`Build::Configure`] - skript `configure` (autotools, FFmpeg) + make
//!   (`configure.rs`).
//!
//! Knihovny se instaluji do predpony `target/c-ports/<arch>/prefix`
//! (`include/`, `lib/`), kde je najdou dalsi porty. Programy jdou do
//! initfs. Chybejici nastroj (CMake, Ninja, make) = chyba s radou
//! k instalaci (`tools.rs`).

mod cmake;
mod configure;
mod fetch;
mod sources;
mod tools;

use std::path::{Path, PathBuf};

use crate::arch::Arch;
use crate::{Result, target_dir};

/// Program portu (zpusob `Sources`).
pub struct Prog {
    /// Jmeno v initfs.
    pub name: &'static str,
    /// Zdroje programu navic ke knihovnim (`ws:` = cesta v repozitari).
    pub sources: &'static [&'static str],
}

/// Zpusob sestaveni portu.
pub enum Build {
    Sources {
        lib: &'static [&'static str],
        defines: &'static [&'static str],
        programs: &'static [Prog],
    },
    CMake {
        /// `-D...` volby konfigurace.
        options: &'static [&'static str],
        /// Programy do initfs: (jmeno v initfs, cesta v adresari sestaveni).
        programs: &'static [(&'static str, &'static str)],
    },
    Configure {
        /// Argumenty `configure`; zastupne `{prefix}`, `{host}`, `{arch}`,
        /// `{cc}`, `{ar}`, `{cflags}`, `{ldflags}`.
        args: &'static [&'static str],
        programs: &'static [(&'static str, &'static str)],
    },
}

pub struct CPort {
    pub name: &'static str,
    pub url: &'static str,
    pub file: &'static str,
    pub sha256: &'static str,
    /// Adresar v archivu.
    pub top: &'static str,
    /// Zaplaty v `third_party/c-ports/<name>/` (poradi pouziti).
    pub patches: &'static [&'static str],
    pub build: Build,
}

const PORTS: &[CPort] = &[
    CPort {
        name: "zlib",
        url: "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz",
        file: "zlib-1.3.1.tar.gz",
        sha256: "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23",
        top: "zlib-1.3.1",
        patches: &[],
        build: Build::Sources {
            lib: &[
                "adler32.c", "compress.c", "crc32.c", "deflate.c", "gzclose.c", "gzlib.c", "gzread.c", "gzwrite.c",
                "infback.c", "inffast.c", "inflate.c", "inftrees.c", "trees.c", "uncompr.c", "zutil.c",
            ],
            defines: &["-DHAVE_UNISTD_H", "-DHAVE_STDARG_H", "-D_LARGEFILE64_SOURCE=0"],
            programs: &[
                Prog { name: "zlib-example", sources: &["test/example.c"] },
                Prog { name: "minigzip", sources: &["test/minigzip.c"] },
            ],
        },
    },
    CPort {
        name: "sqlite",
        url: "https://www.sqlite.org/2024/sqlite-amalgamation-3460100.zip",
        file: "sqlite-amalgamation-3460100.zip",
        sha256: "77823cb110929c2bcb0f5d48e4833b5c59a8a6e40cdea3936b99e199dbbe5784",
        top: "sqlite-amalgamation-3460100",
        patches: &[],
        build: Build::Sources {
            lib: &["sqlite3.c"],
            defines: &[
                "-DSQLITE_THREADSAFE=1",
                "-DSQLITE_OMIT_LOAD_EXTENSION",
                // Sdilena pamet souboru (MAP_SHARED se zapisem) zatim neni:
                // bez mmap I/O a bez WAL (vychozi rollback journal).
                "-DSQLITE_MAX_MMAP_SIZE=0",
                "-DSQLITE_OMIT_WAL",
                "-DSQLITE_OMIT_POPEN",
                "-DHAVE_USLEEP=1",
                "-DHAVE_READLINE=0",
            ],
            programs: &[
                Prog { name: "sqlite3", sources: &["shell.c"] },
                Prog { name: "sqltest", sources: &["ws:user/ctest/c/sqltest.c"] },
            ],
        },
    },
];

/// Predpona instalace knihoven portu pro architekturu.
pub fn prefix(arch: &Arch) -> PathBuf {
    target_dir().join("c-ports").join(arch.name).join("prefix")
}

/// Pracovni adresar portu (objekty, sestaveni).
fn work_dir(arch: &Arch, p: &CPort) -> PathBuf {
    target_dir().join("c-ports").join(arch.name).join(p.name)
}

/// Prelozi porty v C pro architekturu proti sysrootu; vraci (jmeno v
/// initfs, binarka).
pub fn build(arch: &Arch, sysroot: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    for p in PORTS {
        let src = fetch::prepare(p)?;
        let work = work_dir(arch, p);
        std::fs::create_dir_all(&work)?;
        println!("xtask: port C {} ({})", p.name, arch.name);
        let progs = match &p.build {
            Build::Sources { lib, defines, programs } => sources::build(arch, sysroot, &src, &work, lib, defines, programs)?,
            Build::CMake { options, programs } => cmake::build(arch, sysroot, p, &src, &work, options, programs)?,
            Build::Configure { args, programs } => configure::build(arch, sysroot, p, &src, &work, args, programs)?,
        };
        out.extend(progs);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
