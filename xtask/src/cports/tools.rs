//! Nastroje hostitele pro porty v C (CMake, Ninja, sh, make): hledani
//! a srozumitelna rada k instalaci, kdyz chybi.

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
            Tool::Sh | Tool::Make => None,
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
            (Tool::Make, true) => "MSYS2 (winget install MSYS2.MSYS2, pak pacman -S make) nebo spustte xtask ve WSL",
            (Tool::Sh | Tool::Make, false) => "apt install make / dnf install make (sh je soucasti systemu)",
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
        Tool::Make => vec![PathBuf::from("C:\\msys64\\usr\\bin")],
    }
}

/// Najde nastroj: promenna prostredi, PATH, bezna mista instalace.
pub fn find(t: Tool) -> Option<PathBuf> {
    let (name, var) = t.names();
    if let Some(p) = env::var_os(var).map(PathBuf::from)
        && p.is_file()
    {
        return Some(p);
    }
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
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
