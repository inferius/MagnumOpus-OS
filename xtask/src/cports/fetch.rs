//! Zdroj portu: stazeni (HTTPS, `curl`), kontrola SHA-256, rozbaleni
//! (`tar`, umi i zip), zaplaty (`git apply`) a vlastni soubory (overlay)
//! z `third_party/c-ports/<port>/`.
//!
//! Rozbaleny a zaplatovany strom se dela znovu, kdyz se zmeni archiv,
//! zaplata nebo soubor overlay (otisk v nazvu adresare); jinak zustava
//! (preklad je pak prirustkovy).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::CPort;
use crate::{Result, target_dir, workspace_root};

pub fn sha256(data: &[u8]) -> String {
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

/// Adresar vlastnich souboru portu v repozitari.
pub fn port_dir(p: &CPort) -> PathBuf {
    workspace_root().join("third_party").join("c-ports").join(p.name)
}

/// Soubory overlay (relativni cesta, obsah), serazene.
fn overlay_files(p: &CPort) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
        for e in std::fs::read_dir(dir)? {
            let e = e?;
            if e.file_type()?.is_dir() {
                walk(base, &e.path(), out)?;
            } else {
                let rel = e.path().strip_prefix(base).map_err(|e| e.to_string())?.to_path_buf();
                out.push((rel, std::fs::read(e.path())?));
            }
        }
        Ok(())
    }
    let base = port_dir(p).join("overlay");
    let mut out = Vec::new();
    if base.is_dir() {
        walk(&base, &base, &mut out)?;
    }
    out.sort();
    Ok(out)
}

/// Pripraveny zdroj: rozbaleny archiv se zaplatami a overlay.
pub fn prepare(p: &CPort) -> Result<PathBuf> {
    let archive = fetch(p)?;
    let patches: Vec<(String, Vec<u8>)> = p
        .patches
        .iter()
        .map(|f| Ok((f.to_string(), std::fs::read(port_dir(p).join(f)).map_err(|e| format!("zaplata {f}: {e}"))?)))
        .collect::<Result<_>>()?;
    let overlay = overlay_files(p)?;
    let mut h = DefaultHasher::new();
    p.sha256.hash(&mut h);
    patches.hash(&mut h);
    overlay.hash(&mut h);
    let base = target_dir().join("c-ports").join("src").join(format!("{}-{:016x}", p.name, h.finish()));
    let src = base.join(p.top);
    let stamp = base.join(".hotovo");
    if stamp.exists() {
        return Ok(src);
    }
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base)?;
    let ok = Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(&base).status()?.success();
    if !ok {
        return Err(format!("rozbaleni {} selhalo (tar)", p.file).into());
    }
    for (name, _) in &patches {
        // `git apply` funguje i mimo repozitar; cesty v zaplate vuci `top`.
        // `target/` lezi uvnitr repozitare MagnumOpus: bez stropu by git
        // nasel nas repozitar a zaplatu mimo jeho index tise preskocil.
        let out = Command::new("git")
            .arg("apply")
            .arg("--whitespace=nowarn")
            .arg("-p1")
            .arg(port_dir(p).join(name))
            .current_dir(&src)
            .env("GIT_CEILING_DIRECTORIES", &base)
            .output()?;
        if !out.status.success() {
            return Err(format!("zaplata {name} portu {} nejde pouzit:\n{}", p.name, String::from_utf8_lossy(&out.stderr)).into());
        }
    }
    for (rel, data) in &overlay {
        let to = src.join(rel);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&to, data)?;
    }
    std::fs::write(&stamp, b"")?;
    Ok(src)
}
