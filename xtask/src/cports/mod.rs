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
//!   (`configure.rs`), i se sdilenymi knihovnami nad `libc.so`,
//! - [`Build::Meson`] - Meson + Ninja (`meson.rs`, dav1d): staticka knihovna
//!   s `-fPIC` pro sdilenou knihovnu jineho portu.
//!
//! Knihovny se instaluji do predpony `target/c-ports/<arch>/prefix`
//! (`include/`, `lib/`), kde je najdou dalsi porty. Programy jdou do
//! initfs, sdilene knihovny do `lib/` (jmeno = soname). Porty se sdilenymi
//! knihovnami ([`CPort::dynamic`]) se stavi az po `libc.so` ([`build`]
//! s `dynamic = true`). Chybejici nastroj (CMake, Ninja, make, nasm, Meson,
//! pkgconf) = chyba s radou k instalaci (`tools.rs`).

mod cmake;
mod configure;
mod fetch;
mod meson;
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
        /// Argumenty `configure`; zastupne hodnoty viz `configure.rs`.
        args: &'static [&'static str],
        programs: &'static [(&'static str, &'static str)],
        /// Sdilene knihovny do initfs `lib/`: (soname, cesta v adresari
        /// sestaveni). Neprazdne = port nad `libc.so`.
        libraries: &'static [(&'static str, &'static str)],
    },
    Meson {
        /// `-D...` volby projektu.
        options: &'static [&'static str],
        /// Soubory `.pc`, kterym se po instalaci prepise predpona.
        pkgconfig: &'static [&'static str],
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
    /// Port stoji na dynamicke casti sysrootu (`libc.so`, `crt1.o`): stavi
    /// se az po ni.
    pub dynamic: bool,
}

/// Vystup portu do initfs.
pub enum Output {
    /// Program (ET_EXEC; do initfs jen nahravane segmenty).
    Program(String, PathBuf),
    /// Soubor beze zmeny (manifest, data testu).
    Data(String, PathBuf),
    /// Sdilena knihovna (ET_DYN cela) do `lib/<soname>`.
    Library(String, PathBuf),
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
        dynamic: false,
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
        dynamic: false,
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
        dynamic: false,
        // Ovladac videa `magnumopus` (overlay src/video/magnumopus) nad
        // libs/mosdl; docs/cs/sdl.md.
        patches: &["0001-video-magnumopus.patch"],
        data: &[
            ("testsprite.manifest.toml", "port:testsprite.manifest.toml"),
            ("checkkeys.manifest.toml", "port:checkkeys.manifest.toml"),
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
            programs: &[("testsprite", "test/testsprite"), ("checkkeys", "test/checkkeys")],
        },
    },
    // AV1 (BSD-2-Clause): staticka knihovna s -fPIC, vlozi se do
    // libavcodec.so (docs/cs/multimedia.md).
    CPort {
        name: "dav1d",
        url: "https://downloads.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz",
        file: "dav1d-1.5.4.tar.xz",
        sha256: "686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5",
        top: "dav1d-1.5.4",
        patches: &[],
        data: &[],
        dynamic: true,
        build: Build::Meson {
            options: &["-Denable_tools=false", "-Denable_tests=false", "-Denable_examples=false", "-Denable_docs=false"],
            pkgconfig: &["dav1d.pc"],
        },
    },
    // FFmpeg (LGPL-2.1-or-later): sdilene knihovny nad libc.so, nikdy GPL
    // ani nonfree (overeni v config.h: `ffmpeg_license`). Proces dekoderu
    // je user/ffdec. Archiv podepsany klicem FFmpeg (FCF9 86EA 15E6 E293
    // A564 4F10 B432 2F04 D676 58D8), otisk z overeneho archivu.
    CPort {
        name: "ffmpeg",
        url: "https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz",
        file: "ffmpeg-9.0.2.tar.xz",
        sha256: "8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e",
        top: "ffmpeg-9.0.2",
        patches: &[],
        data: &[],
        dynamic: true,
        build: Build::Configure {
            args: FFMPEG_ARGS,
            programs: &[],
            libraries: &[
                ("libavutil.so.61", "libavutil/libavutil.so.61"),
                ("libswresample.so.7", "libswresample/libswresample.so.7"),
                ("libswscale.so.10", "libswscale/libswscale.so.10"),
                ("libavcodec.so.63", "libavcodec/libavcodec.so.63"),
                ("libavformat.so.63", "libavformat/libavformat.so.63"),
            ],
        },
    },
];

/// Konfigurace FFmpeg: jen dekodovani (demuxery, parsery, dekodery, bsf),
/// zadne kodery, muxery, filtry, zarizeni ani sit (vstup dava `mediad`
/// jako bajty). Patentove zatizene dekodery (H.264, HEVC, AAC) jsou
/// zapnute (rozhodnuti 5 = B); bez nich viz [`ffmpeg_args`].
const FFMPEG_ARGS: &[&str] = &[
    "--prefix={prefix}",
    "--enable-cross-compile",
    "--target-os=none",
    "--arch={arch}",
    "--cc={cc}",
    "--ar={ar}",
    "--nm={nm} -g",
    "--ranlib={ranlib}",
    "--strip={strip}",
    "--host-cc={cc}",
    "--x86asmexe=nasm",
    "--extra-cflags={cflags}",
    "--extra-ldflags={ldflags}",
    "--extra-ldexeflags={ldexeflags}",
    "--extra-ldsoflags={ldsoflags}",
    "--extra-libs={libs}",
    "--pkg-config=pkgconf",
    "--disable-static",
    "--enable-shared",
    "--enable-pic",
    // Licence: LGPL 2.1+ (nikdy GPL, verze 3 ani nonfree).
    "--disable-gpl",
    "--disable-version3",
    "--disable-nonfree",
    "--disable-programs",
    "--disable-doc",
    "--disable-debug",
    "--disable-network",
    "--disable-autodetect",
    "--disable-avdevice",
    "--disable-avfilter",
    "--disable-encoders",
    "--disable-muxers",
    "--disable-devices",
    "--disable-hwaccels",
    "--disable-filters",
    "--disable-protocols",
    "--enable-pthreads",
    "--enable-libdav1d",
];

/// Patentove zatizene dekodery a jejich parsery (vypne je
/// `XTASK_BEZ_PATENTOVYCH_KODEKU=1`, docs/cs/multimedia.md).
pub const PATENT_DECODERS: &[&str] = &["h264", "hevc", "aac", "aac_fixed", "aac_latm", "eac3", "vvc"];
pub const PATENT_PARSERS: &[&str] = &["h264", "hevc", "aac", "aac_latm", "vvc"];

/// Obraz bez patentove zatizenych dekoderu?
pub fn without_patent_codecs() -> bool {
    std::env::var_os("XTASK_BEZ_PATENTOVYCH_KODEKU").is_some_and(|v| v == "1")
}

/// Argumenty configure FFmpeg podle volby patentovych kodeku.
pub fn ffmpeg_args(base: &[&str], without_patents: bool) -> Vec<String> {
    let mut v: Vec<String> = base.iter().map(|s| s.to_string()).collect();
    if without_patents {
        v.extend(PATENT_DECODERS.iter().map(|d| format!("--disable-decoder={d}")));
        v.extend(PATENT_PARSERS.iter().map(|d| format!("--disable-parser={d}")));
    }
    v
}

/// Overi licenci a volbu patentovych kodeku v `config.h` a
/// `config_components.h` sestaveni FFmpeg: GPL / nonfree / verze 3 =
/// chyba (dekoder by sel do obrazu pod spatnou licenci).
pub fn check_ffmpeg_config(build: &Path, without_patents: bool) -> Result<()> {
    let config = std::fs::read_to_string(build.join("config.h"))?;
    let components = std::fs::read_to_string(build.join("config_components.h"))?;
    for (key, want) in [("CONFIG_GPL", "0"), ("CONFIG_NONFREE", "0"), ("CONFIG_VERSION3", "0"), ("CONFIG_SHARED", "1")] {
        let line = format!("#define {key} {want}");
        if !config.lines().any(|l| l.trim() == line) {
            return Err(format!("FFmpeg: v config.h chybi `{line}` (licence nebo sdilene knihovny)").into());
        }
    }
    for d in PATENT_DECODERS {
        let line = format!("#define CONFIG_{}_DECODER {}", d.to_uppercase(), if without_patents { 0 } else { 1 });
        if !components.lines().any(|l| l.trim() == line) {
            return Err(format!("FFmpeg: v config_components.h chybi `{line}`").into());
        }
    }
    Ok(())
}

/// Testovaci media do initfs (`media/<jmeno>`): oficialni testovaci
/// vektory s MD5 kazdeho dekodovaneho snimku (I420), sada `media ffmpeg`.
/// AV1: AOM (BSD-2-Clause), VP9: libvpx (BSD-3-Clause); THIRD-PARTY.md.
pub const TEST_MEDIA: &[(&str, &str, &str)] = &[
    (
        "av1-64x64.ivf",
        "https://storage.googleapis.com/aom-test-data/av1-1-b8-01-size-64x64.ivf",
        "3a92ce417514905ca9c63192d6a73114d46d333f71ffde7baf524b76870d96bb",
    ),
    (
        "av1-64x64.ivf.md5",
        "https://storage.googleapis.com/aom-test-data/av1-1-b8-01-size-64x64.ivf.md5",
        "81cc609dfc77bc3af5cb7d451f228cbdccef424235dc98e7fcbdc88851dac958",
    ),
    (
        "vp9-64x64.webm",
        "https://storage.googleapis.com/downloads.webmproject.org/test_data/libvpx/vp90-2-02-size-64x64.webm",
        "108c5f1c4bcceb0318ee2887678ca2e92fea752db38ad9099eb236ce63ba60b3",
    ),
    (
        "vp9-64x64.webm.md5",
        "https://storage.googleapis.com/downloads.webmproject.org/test_data/libvpx/vp90-2-02-size-64x64.webm.md5",
        "a3f6e09e19a69f32129b6f1b12deaa10a702dbb36d598dafc379df1fbf9c3aad",
    ),
];

/// Testovaci media pro initfs (`media/<jmeno>`, soubor beze zmeny).
pub fn test_media() -> Result<Vec<Output>> {
    TEST_MEDIA
        .iter()
        .map(|(name, url, sha)| {
            let file = url.rsplit('/').next().unwrap_or(name);
            Ok(Output::Data(format!("media/{name}"), fetch::fetch_file(url, file, sha)?))
        })
        .collect()
}

/// Predpona instalace knihoven portu pro architekturu.
pub fn prefix(arch: &Arch) -> PathBuf {
    target_dir().join("c-ports").join(arch.name).join("prefix")
}

/// Pracovni adresar portu (objekty, sestaveni).
fn work_dir(arch: &Arch, p: &CPort) -> PathBuf {
    target_dir().join("c-ports").join(arch.name).join(p.name)
}

/// Prelozi porty v C pro architekturu proti sysrootu; vraci programy
/// a soubory do initfs. `dynamic` = jen porty nad `libc.so` (volat po
/// `csysroot::add_dynamic`), jinak jen staticke.
pub fn build(arch: &Arch, sysroot: &Path, dynamic: bool) -> Result<Vec<Output>> {
    let mut out = Vec::new();
    for p in PORTS.iter().filter(|p| p.dynamic == dynamic) {
        let src = fetch::prepare(p)?;
        let work = work_dir(arch, p);
        std::fs::create_dir_all(&work)?;
        println!("xtask: port C {} ({})", p.name, arch.name);
        let (progs, libs) = match &p.build {
            Build::Sources { lib, defines, programs } => (sources::build(arch, sysroot, &src, &work, lib, defines, programs)?, Vec::new()),
            Build::CMake { options, programs } => (cmake::build(arch, sysroot, p, &src, &work, options, programs)?, Vec::new()),
            Build::Configure { args, programs, libraries } => {
                let ffmpeg = p.name == "ffmpeg";
                let without = ffmpeg && without_patent_codecs();
                let args = if ffmpeg { ffmpeg_args(args, without) } else { args.iter().map(|s| s.to_string()).collect() };
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                let built = configure::build(arch, sysroot, p, &src, &work, &refs, programs, libraries)?;
                if ffmpeg {
                    check_ffmpeg_config(&work.join("build"), without)?;
                }
                built
            }
            Build::Meson { options, pkgconfig } => {
                meson::build(arch, sysroot, p, &src, &work, options, pkgconfig)?;
                (Vec::new(), Vec::new())
            }
        };
        out.extend(progs.into_iter().map(|(n, p)| Output::Program(n, p)));
        out.extend(libs.into_iter().map(|(n, p)| Output::Library(n, p)));
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
