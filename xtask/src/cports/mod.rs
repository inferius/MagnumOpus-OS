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
    // Prvni port tohoto druhu bude FFmpeg (krok 7).
    #[allow(dead_code)]
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
    /// Dalsi soubory do initfs: (jmeno v initfs, cesta). `port:` = cesta
    /// v `third_party/c-ports/<name>/` (manifesty), jinak ve zdroji portu.
    pub data: &'static [(&'static str, &'static str)],
}

/// Vystup portu do initfs.
pub enum Output {
    /// Program (ET_EXEC; do initfs jen nahravane segmenty).
    Program(String, PathBuf),
    /// Soubor beze zmeny (manifest, data testu).
    Data(String, PathBuf),
}

const PORTS: &[CPort] = &[
    CPort {
        name: "zlib",
        url: "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz",
        file: "zlib-1.3.1.tar.gz",
        sha256: "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23",
        top: "zlib-1.3.1",
        patches: &[],
        data: &[],
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
        data: &[],
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
    CPort {
        name: "sdl3",
        url: "https://github.com/libsdl-org/SDL/releases/download/release-3.4.20/SDL3-3.4.20.tar.gz",
        file: "SDL3-3.4.20.tar.gz",
        sha256: "f3bb06f55bed9557227a700af2ea62e49497ebdf58037732a731a5fb3ef8581f",
        top: "SDL3-3.4.20",
        // Ovladac videa `magnumopus` (overlay src/video/magnumopus) nad
        // libs/mosdl; docs/cs/sdl.md.
        patches: &["0001-video-magnumopus.patch"],
        data: &[
            ("testsprite.manifest.toml", "port:testsprite.manifest.toml"),
            ("checkkeys.manifest.toml", "port:checkkeys.manifest.toml"),
            ("testmagnumopus.manifest.toml", "port:testmagnumopus.manifest.toml"),
            ("sdl-icon.png", "test/icon.png"),
        ],
        build: Build::CMake {
            options: &[
                // Staticka knihovna (Q8 A); sdilena az s libc.so.
                "-DSDL_SHARED=OFF",
                "-DSDL_STATIC=ON",
                "-DSDL_TESTS=ON",
                "-DSDL_INSTALL_TESTS=OFF",
                "-DSDL_EXAMPLES=ON",
                // Zadne ovladace Linuxu / X11 / Waylandu ani dlopen knihoven.
                "-DSDL_X11=OFF",
                "-DSDL_WAYLAND=OFF",
                "-DSDL_KMSDRM=OFF",
                "-DSDL_OFFSCREEN=OFF",
                "-DSDL_OPENGL=OFF",
                "-DSDL_OPENGLES=OFF",
                "-DSDL_VULKAN=OFF",
                "-DSDL_GPU=OFF",
                "-DSDL_RENDER_GPU=OFF",
                "-DSDL_ALSA=OFF",
                "-DSDL_PULSEAUDIO=OFF",
                "-DSDL_PIPEWIRE=OFF",
                "-DSDL_JACK=OFF",
                "-DSDL_SNDIO=OFF",
                "-DSDL_OSS=OFF",
                "-DSDL_DBUS=OFF",
                "-DSDL_IBUS=OFF",
                "-DSDL_LIBUDEV=OFF",
                "-DSDL_LIBURING=OFF",
                "-DSDL_SYSTEM_ICONV=OFF",
                "-DSDL_LIBICONV=OFF",
                "-DSDL_RPATH=OFF",
                "-DSDL_DEPS_SHARED=OFF",
                "-DSDL_DLOPEN_NOTES=OFF",
                // Okna dava ovladac magnumopus, ne X11 / Wayland (jinak by
                // konfigurace na "unixu" bez nich skoncila chybou).
                "-DSDL_UNIX_CONSOLE_BUILD=ON",
                // Gamepady samostatny krok (Q6 B), kamera a dialogy nejsou (Q7 A).
                "-DSDL_HIDAPI=OFF",
                "-DSDL_JOYSTICK=OFF",
                "-DSDL_HAPTIC=OFF",
                "-DSDL_CAMERA=OFF",
                "-DSDL_DIALOG=OFF",
                "-DSDL_TRAY=OFF",
            ],
            programs: &[("testsprite", "test/testsprite"), ("checkkeys", "test/checkkeys"), ("testmagnumopus", "test/testmagnumopus")],
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

/// Prelozi porty v C pro architekturu proti sysrootu; vraci programy
/// a soubory do initfs.
pub fn build(arch: &Arch, sysroot: &Path) -> Result<Vec<Output>> {
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
        out.extend(progs.into_iter().map(|(n, p)| Output::Program(n, p)));
        for (name, path) in p.data {
            let file = match path.strip_prefix("port:") {
                Some(rel) => fetch::port_dir(p).join(rel),
                None => src.join(path),
            };
            out.push(Output::Data(name.to_string(), file));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
