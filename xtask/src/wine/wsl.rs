//! Beh ve WSL pro sestaveni Wine: kontrola nastroju s radou k instalaci
//! (docs/cs/sestaveni.md) a spusteni skriptu s promennymi.

use std::path::Path;
use std::process::Command;

use crate::Result;

/// Distribuce WSL (`WINE_WSL_DISTRO`, vychozi Ubuntu jako QEMU s Venusem).
pub fn distro() -> String {
    std::env::var("WINE_WSL_DISTRO").unwrap_or_else(|_| String::from("Ubuntu"))
}

/// Nastroj ve WSL: prikaz verze a nejnizsi verze (hlavni, vedlejsi).
pub struct WslTool {
    pub name: &'static str,
    pub version_cmd: &'static str,
    pub min: Option<(u32, u32)>,
}

/// Co Wine ve WSL potrebuje. LLVM 23 = verze clangu hostitele (ABI
/// hlavicek a sysrootu C).
pub const TOOLS: &[WslTool] = &[
    WslTool { name: "clang-23", version_cmd: "clang-23 --version", min: Some((23, 1)) },
    WslTool { name: "ld.lld-23", version_cmd: "ld.lld-23 --version", min: Some((23, 1)) },
    WslTool { name: "llvm-ar-23", version_cmd: "llvm-ar-23 --version", min: Some((23, 1)) },
    WslTool { name: "flex", version_cmd: "flex --version", min: None },
    WslTool { name: "bison", version_cmd: "bison --version", min: None },
    WslTool { name: "make", version_cmd: "make --version", min: None },
    WslTool { name: "gcc", version_cmd: "gcc --version", min: None },
    // Nastroj sfnt2fon (bitmapova pisma .fon z .ttf) potrebuje FreeType
    // hostitele.
    WslTool { name: "libfreetype-dev", version_cmd: "pkg-config --modversion freetype2", min: None },
];

/// Rada k instalaci (docs/cs/sestaveni.md, Wine).
pub const ADVICE: &str = "ve WSL Ubuntu 24.04: oficialni repozitar LLVM (apt.llvm.org, llvm-toolchain-noble-23 s jeho klicem) \
a `sudo apt install clang-23 lld-23 llvm-23 flex bison make gcc pkg-config libfreetype-dev` (docs/cs/sestaveni.md, kapitola Wine)";

/// Prvni cislo verze `X.Y` v textu.
pub fn parse_version(out: &str) -> Option<(u32, u32)> {
    out.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|w| {
        let mut it = w.split('.');
        let a = it.next()?.parse().ok()?;
        let b = it.next()?.parse().ok()?;
        Some((a, b))
    })
}

fn wsl(distro: &str) -> Command {
    let mut c = Command::new("wsl.exe");
    c.args(["-d", distro, "--"]);
    c
}

/// Overi nastroje ve WSL; chyba vyjmenuje chybejici a stare s radou.
pub fn require_tools(distro: &str) -> Result<()> {
    let mut bad = Vec::new();
    for t in TOOLS {
        let out = wsl(distro).args(["bash", "-lc", t.version_cmd]).output();
        let text = match out {
            Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
            Ok(_) => {
                bad.push(format!("{} chybi", t.name));
                continue;
            }
            Err(e) => return Err(format!("WSL ({distro}) nejde spustit: {e}; {ADVICE}").into()),
        };
        if let Some(min) = t.min {
            match parse_version(&text) {
                Some(v) if v >= min => {}
                v => bad.push(format!("{} verze {v:?}, potreba aspon {}.{}", t.name, min.0, min.1)),
            }
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("nastroje Wine ve WSL ({distro}): {}; {ADVICE}", bad.join(", ")).into())
    }
}

/// Spusti skript ve WSL s promennymi (hodnoty bez apostrofu).
pub fn run_script(distro: &str, script: &Path, env: &[(&str, String)]) -> Result<()> {
    let mut line = String::new();
    for (k, v) in env {
        if v.contains('\'') {
            return Err(format!("promenna {k} obsahuje apostrof").into());
        }
        line.push_str(&format!("export {k}='{v}'; "));
    }
    line.push_str(&format!("tr -d '\\r' < '{}' | bash -s", crate::qemu::to_wsl_paths(&script.display().to_string())));
    let st = wsl(distro).args(["bash", "-lc", &line]).status()?;
    if !st.success() {
        return Err(format!("sestaveni Wine ve WSL selhalo ({st})").into());
    }
    Ok(())
}
