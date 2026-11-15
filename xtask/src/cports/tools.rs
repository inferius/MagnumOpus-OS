//! Nastroje hostitele pro porty v C (CMake, Ninja, sh, make, nasm, Meson,
//! pkgconf): hledani a srozumitelna rada k instalaci, kdyz chybi.
//!
//! Na Windows maji nastroje unixovych build systemu (sh, make, nasm, Meson,
//! pkgconf) prednost z MSYS2 (`C:\msys64\usr\bin`, promenna `MSYS2_ROOT`):
//! jedno prostredi misto mixu Git a MSYS2. Meson je skript Pythonu MSYS2
//! (bez `.exe`), spousti se pres `sh` ([`unix_env`]).

use std::env;
use std::path::PathBuf;

use crate::Result;

/// Nastroj, ktery port potrebuje.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    CMake,
    Ninja,
    Sh,
    Make,
    Nasm,
    Meson,
    PkgConf,
}

impl Tool {
    /// Jmeno spustitelneho souboru (bez pripony) a promenna prostredi,
    /// ktera ho muze urcit primo.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Tool::CMake => ("cmake", "CMAKE"),
            Tool::Ninja => ("ninja", "NINJA"),
            Tool::Sh => ("sh", "SH"),
            Tool::Make => ("make", "MAKE"),
            Tool::Nasm => ("nasm", "NASM"),
            Tool::Meson => ("meson", "MESON"),
            Tool::PkgConf => ("pkgconf", "PKG_CONFIG"),
        }
    }

    /// Nejnizsi podporovana verze (docs/cs/sestaveni.md, pozadavky na
    /// hostitele); `None` = verze se nekontroluje.
    pub fn min_version(self) -> Option<(u32, u32)> {
        match self {
            // SDL 3.4 chce 3.16; 3.24 kvuli souboru nastroju (hledani
            // v CMAKE_FIND_ROOT_PATH, CMAKE_TRY_COMPILE_TARGET_TYPE).
            Tool::CMake => Some((3, 24)),
            Tool::Ninja => Some((1, 10)),
            // dav1d chce nasm 2.14 (AVX-512), FFmpeg 2.13.
            Tool::Nasm => Some((2, 14)),
            // Meson a pkgconf z MSYS2 / systemu (verzi pro projekt hlida Meson sam).
            Tool::Sh | Tool::Make | Tool::Meson | Tool::PkgConf => None,
        }
    }

    /// Rada k instalaci podle hostitele.
    pub fn advice(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Tool::CMake, true) => "winget install Kitware.CMake",
            (Tool::Ninja, true) => "winget install Ninja-build.Ninja",
            (Tool::CMake, false) => "apt install cmake (Debian, Ubuntu) / dnf install cmake / brew install cmake",
            (Tool::Ninja, false) => "apt install ninja-build / dnf install ninja-build / brew install ninja",
            (Tool::Sh, true) => "Git for Windows (winget install Git.Git) obsahuje sh; nebo spustte xtask ve WSL",
            (Tool::Make, true) => "MSYS2 (winget install MSYS2.MSYS2, pak v MSYS2: pacman -S make) nebo spustte xtask ve WSL",
            (Tool::Nasm, true) => "MSYS2: pacman -S nasm (winget install MSYS2.MSYS2)",
            (Tool::Meson, true) => "MSYS2: pacman -S meson (pritahne python a ninja)",
            (Tool::PkgConf, true) => "MSYS2: pacman -S pkgconf (s mesonem se nainstaluje samo)",
            (Tool::Sh | Tool::Make, false) => "apt install make / dnf install make (sh je soucasti systemu)",
            (Tool::Nasm, false) => "apt install nasm / dnf install nasm / brew install nasm",
            (Tool::Meson, false) => "apt install meson / dnf install meson / brew install meson",
            (Tool::PkgConf, false) => "apt install pkgconf / dnf install pkgconf / brew install pkgconf",
        }
    }
}

/// Bezna mista instalace mimo PATH (instalator nemusel PATH obnovit
/// v otevrenem terminalu).
fn known_dirs(t: Tool) -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let pf = env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Program Files"));
    let local = env::var_os("LOCALAPPDATA").map(PathBuf::from);
    match t {
        // winget dava CMake i Ninja jako balik s odkazem ve WinGet\Links.
        Tool::CMake => {
            let mut v = vec![pf.join("CMake").join("bin")];
            v.extend(local.map(|l| l.join("Microsoft").join("WinGet").join("Links")));
            v
        }
        Tool::Ninja => local.map(|l| vec![l.join("Microsoft").join("WinGet").join("Links")]).unwrap_or_default(),
        Tool::Sh => vec![pf.join("Git").join("bin"), pf.join("Git").join("usr").join("bin")],
        // MSYS2 hleda `msys2_bin` pred PATH.
        Tool::Make | Tool::Nasm | Tool::Meson | Tool::PkgConf => Vec::new(),
    }
}

/// `usr\bin` instalace MSYS2 (`MSYS2_ROOT`, jinak `C:\msys64`), kdyz existuje.
pub fn msys2_bin() -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let root = env::var_os("MSYS2_ROOT").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\msys64"));
    let bin = root.join("usr").join("bin");
    bin.join("sh.exe").is_file().then_some(bin)
}

/// Nastroj unixoveho build systemu: na Windows ma prednost MSYS2.
fn unix_tool(t: Tool) -> bool {
    matches!(t, Tool::Sh | Tool::Make | Tool::Nasm | Tool::Meson | Tool::PkgConf)
}

/// Prostredi pro `sh` / `make` / `meson` portu: na Windows MSYS2 v rezimu
/// MSYS (Meson z MSYS2 odmita bezet pod MINGW64 z Git Bash), na zacatku
/// PATH nastroje MSYS2 a adresar clangu (configure FFmpeg nezvlada cesty
/// s mezerou, nastroje LLVM se proto predavaji jen jmenem).
pub fn unix_env(cmd: &mut std::process::Command, clang: &std::path::Path) {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(m) = msys2_bin() {
        dirs.push(m);
        cmd.env("MSYSTEM", "MSYS");
    }
    if let Some(d) = clang.parent() {
        dirs.push(d.to_path_buf());
    }
    dirs.extend(env::var_os("PATH").map(|p| env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default());
    if let Ok(p) = env::join_paths(dirs) {
        cmd.env("PATH", p);
    }
}

/// Cesta ve tvaru pro seznamy cest v `sh` z MSYS2 (`J:\a\b` -> `/j/a/b`,
/// dvojtecka jako oddelovac); na unixu beze zmeny.
pub fn sh_path(p: &std::path::Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    let b = s.as_bytes();
    if cfg!(windows) && b.len() >= 2 && b[1] == b':' {
        return format!("/{}{}", (b[0] as char).to_ascii_lowercase(), &s[2..]);
    }
    s
}

/// Najde nastroj: promenna prostredi, (MSYS2,) PATH, bezna mista instalace.
pub fn find(t: Tool) -> Option<PathBuf> {
    let (name, var) = t.names();
    if let Some(p) = env::var_os(var).map(PathBuf::from)
        && p.is_file()
    {
        return Some(p);
    }
    // Meson je skript (bez `.exe` i na Windows).
    let exe = if cfg!(windows) && t != Tool::Meson { format!("{name}.exe") } else { name.to_string() };
    if unix_tool(t)
        && let Some(m) = msys2_bin()
        && m.join(&exe).is_file()
    {
        return Some(m.join(&exe));
    }
    let path_dirs = env::var_os("PATH").map(|p| env::split_paths(&p).collect::<Vec<_>>()).unwrap_or_default();
    path_dirs.into_iter().chain(known_dirs(t)).map(|d| d.join(&exe)).find(|p| p.is_file())
}

/// Prvni "X.Y" ve vystupu `--version` (`cmake version 4.4.4`, `1.13.2`).
pub fn parse_version(out: &str) -> Option<(u32, u32)> {
    out.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|w| {
        let mut it = w.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        Some((major, minor))
    })
}

/// Verze nastroje (`--version`), nebo `None`, kdyz nejde zjistit.
fn version(path: &std::path::Path) -> Option<(u32, u32)> {
    let out = std::process::Command::new(path).arg("--version").output().ok()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// Najde vsechny nastroje portu a overi jejich verzi, nebo vrati jednu
/// chybu se vsemi, ktere chybi nebo jsou stare, a radou k instalaci.
pub fn require(port: &str, tools: &[Tool]) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    let mut old = Vec::new();
    for &t in tools {
        match find(t) {
            Some(p) => {
                if let Some(min) = t.min_version() {
                    match version(&p) {
                        Some(v) if v >= min => {}
                        v => old.push((t, v, min)),
                    }
                }
                found.push(p);
            }
            None => missing.push(t),
        }
    }
    if missing.is_empty() && old.is_empty() {
        return Ok(found);
    }
    let mut msg = format!("port C {port} potrebuje nastroje, ktere na hostiteli chybi nebo jsou stare:\n");
    for t in &missing {
        let (name, var) = t.names();
        msg.push_str(&format!("  - {name}: chybi; {} (nebo promenna {var} s cestou)\n", t.advice()));
    }
    for (t, v, (a, b)) in &old {
        let have = v.map_or(String::from("neznama verze"), |(x, y)| format!("{x}.{y}"));
        msg.push_str(&format!("  - {}: {have}, potreba aspon {a}.{b}; {}\n", t.names().0, t.advice()));
    }
    msg.push_str("Po instalaci otevrete novy terminal (PATH) a spustte xtask znovu.");
    Err(msg.into())
}
