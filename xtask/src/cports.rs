//! Porty programu v C (zlib, sqlite) prelozene proti sysrootu C
//! (`csysroot.rs`) - overeni `libc.a` a sysrootu v praxi (docs/cs/cstd.md,
//! kapitola Porty v C).
//!
//! Zdroje se stahnou z oficialnich mist (HTTPS, `curl`) do
//! `target/c-ports/dl`, overi se SHA-256 a rozbali (`tar`, umi i zip).
//! Kazdy port ma knihovni zdroje (prelozene jednou) a programy; testovaci
//! programy z repozitare (`ws:` cesta) se prelozi stejne. Objekty se
//! prekladaji znovu jen pri zmene zdroje nebo priznaku; link je vzdy.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::arch::Arch;
use crate::{Result, csysroot, target_dir, workspace_root};

struct Prog {
    /// Jmeno v initfs.
    name: &'static str,
    /// Zdroje programu navic ke knihovnim (`ws:` = cesta v repozitari).
    sources: &'static [&'static str],
}

struct CPort {
    name: &'static str,
    url: &'static str,
    file: &'static str,
    sha256: &'static str,
    /// Adresar v archivu.
    top: &'static str,
    lib: &'static [&'static str],
    defines: &'static [&'static str],
    programs: &'static [Prog],
}

const PORTS: &[CPort] = &[
    CPort {
        name: "zlib",
        url: "https://github.com/madler/zlib/releases/download/v1.3.1/zlib-1.3.1.tar.gz",
        file: "zlib-1.3.1.tar.gz",
        sha256: "9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23",
        top: "zlib-1.3.1",
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
    CPort {
        name: "sqlite",
        url: "https://www.sqlite.org/2024/sqlite-amalgamation-3460100.zip",
        file: "sqlite-amalgamation-3460100.zip",
        sha256: "77823cb110929c2bcb0f5d48e4833b5c59a8a6e40cdea3936b99e199dbbe5784",
        top: "sqlite-amalgamation-3460100",
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
];

fn sha256(data: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, data);
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Stazeny a overeny archiv portu.
fn fetch(p: &CPort) -> Result<PathBuf> {
    let dl = target_dir().join("c-ports").join("dl");
    std::fs::create_dir_all(&dl)?;
    let path = dl.join(p.file);
    if let Ok(data) = std::fs::read(&path)
        && sha256(&data) == p.sha256
    {
        return Ok(path);
    }
    println!("xtask: stahuji {}", p.url);
    let tmp = dl.join(format!("{}.part", p.file));
    let ok = Command::new("curl").args(["-sSfL", "-o"]).arg(&tmp).arg(p.url).status()?.success();
    if !ok {
        return Err(format!("stazeni {} selhalo (curl)", p.url).into());
    }
    let data = std::fs::read(&tmp)?;
    let got = sha256(&data);
    if got != p.sha256 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("{}: SHA-256 {got} nesedi (ocekavano {})", p.file, p.sha256).into());
    }
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// Rozbaleny zdroj (jednou na otisk archivu).
fn extract(p: &CPort, archive: &Path) -> Result<PathBuf> {
    let base = target_dir().join("c-ports").join("src").join(p.sha256);
    let stamp = base.join(".hotovo");
    if !stamp.exists() {
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base)?;
        let ok = Command::new("tar").arg("-xf").arg(archive).arg("-C").arg(&base).status()?.success();
        if !ok {
            return Err(format!("rozbaleni {} selhalo (tar)", p.file).into());
        }
        std::fs::write(&stamp, b"")?;
    }
    Ok(base.join(p.top))
}

fn source_path(src_dir: &Path, s: &str) -> PathBuf {
    match s.strip_prefix("ws:") {
        Some(rel) => workspace_root().join(rel),
        None => src_dir.join(s),
    }
}

fn newer(a: &Path, b: &Path) -> bool {
    let m = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (m(a), m(b)) {
        (Some(x), Some(y)) => x > y,
        _ => true,
    }
}

/// Prelozi zdroj na objekt (znovu jen pri zmene zdroje nebo priznaku).
fn compile(clang: &Path, flags: &[String], src: &Path, obj_dir: &Path) -> Result<PathBuf> {
    let mut h = DefaultHasher::new();
    src.hash(&mut h);
    flags.hash(&mut h);
    let obj = obj_dir.join(format!(
        "{}-{:016x}.o",
        src.file_stem().unwrap_or_default().to_string_lossy(),
        h.finish()
    ));
    if obj.exists() && !newer(src, &obj) {
        return Ok(obj);
    }
    let out = Command::new(clang).args(flags).arg("-c").arg(src).arg("-o").arg(&obj).output()?;
    if !out.status.success() {
        return Err(format!("preklad {} selhal:\n{}", src.display(), String::from_utf8_lossy(&out.stderr)).into());
    }
    Ok(obj)
}

/// Prelozi porty v C pro architekturu proti sysrootu; vraci (jmeno v
/// initfs, binarka).
pub fn build(arch: &Arch, sysroot: &Path) -> Result<Vec<(String, PathBuf)>> {
    let clang = cbuild::find_clang()?;
    let base_flags = csysroot::cflags(arch, sysroot)?;
    let ldflags = csysroot::ldflags(sysroot);
    let mut out = Vec::new();
    for p in PORTS {
        let src = extract(p, &fetch(p)?)?;
        let obj_dir = target_dir().join("c-ports").join(arch.name).join(p.name);
        std::fs::create_dir_all(&obj_dir)?;
        let mut flags = base_flags.clone();
        flags.extend(["-O2", "-w"].map(String::from));
        flags.extend(p.defines.iter().map(|d| d.to_string()));
        flags.push(format!("-I{}", src.display()));
        println!("xtask: port C {} ({})", p.name, arch.name);
        let lib: Vec<PathBuf> = p.lib.iter().map(|s| compile(&clang, &flags, &source_path(&src, s), &obj_dir)).collect::<Result<_>>()?;
        for prog in p.programs {
            let mut objs = lib.clone();
            for s in prog.sources {
                objs.push(compile(&clang, &flags, &source_path(&src, s), &obj_dir)?);
            }
            let exe = obj_dir.join(prog.name);
            let link = Command::new(&clang)
                .args(cbuild::target_flags(arch.user_target)?)
                .args(&objs)
                .args(&ldflags)
                .arg("-o")
                .arg(&exe)
                .output()?;
            if !link.status.success() {
                return Err(format!("link {} selhal:\n{}", prog.name, String::from_utf8_lossy(&link.stderr)).into());
            }
            out.push((String::from(prog.name), exe));
        }
    }
    Ok(out)
}
