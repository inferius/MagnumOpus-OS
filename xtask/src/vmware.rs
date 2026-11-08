//! `cargo xtask vmware`: obraz ziveho USB jako disk VMware Workstation.
//!
//! Sestavi obraz (jako `cargo xtask usb`), prevede ho nastrojem
//! `qemu-img` na VMDK a nahradi jim disk virtualniho pocitace. Puvodni
//! disk se pred prepsanim zazalohuje (nechavaji se posledni
//! [`BACKUPS`] zalohy).
//!
//! Slozka virtualniho pocitace: `--dir <slozka>` nebo promenna prostredi
//! `MO_VMWARE_DIR`. Jmeno disku: `magnumopus.vmdk` (stejne jako v navodu
//! v `docs/cs/zive-usb.md`).
//!
//! Bezici VM se neprepisuje: VMware drzi zamky `*.lck` a prepis disku
//! za behu by ho poskodil.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{Options, Result, target_dir};

/// Jmeno disku ve slozce VM.
const DISK: &str = "magnumopus.vmdk";
/// Kolik zaloh puvodniho disku nechat.
const BACKUPS: usize = 3;

pub fn command(opts: &Options, dir: Option<PathBuf>) -> Result<()> {
    let dir = dir
        .or_else(|| std::env::var_os("MO_VMWARE_DIR").map(PathBuf::from))
        .ok_or("vmware: chybi slozka VM (--dir <slozka> nebo promenna MO_VMWARE_DIR)")?;
    if !dir.is_dir() {
        return Err(format!("vmware: slozka {} neexistuje", dir.display()).into());
    }
    check_not_running(&dir)?;
    let qemu_img = qemu_img(opts)?;

    let artifacts = crate::build(opts)?;
    let profile = if opts.release { "release" } else { "debug" };
    let img = target_dir().join(format!("usb-{}-{profile}.img", opts.arch.name));
    crate::usb::create_image(opts, &artifacts, &img, crate::usb::DEFAULT_MIB)?;

    // Nejdriv do docasneho souboru: kdyz prevod selze, puvodni disk zustane.
    let tmp = dir.join(format!("{DISK}.novy"));
    let _ = std::fs::remove_file(&tmp);
    let status = Command::new(&qemu_img)
        .args(["convert", "-O", "vmdk"])
        .arg(&img)
        .arg(&tmp)
        .status()?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err("vmware: qemu-img convert selhal".into());
    }
    let disk = dir.join(DISK);
    if disk.exists() {
        let backup = dir.join(format!("magnumopus-{}.vmdk", stamp()));
        std::fs::rename(&disk, &backup)?;
        println!("xtask: zaloha puvodniho disku: {}", backup.display());
        prune_backups(&dir)?;
    }
    std::fs::rename(&tmp, &disk)?;
    println!("xtask: disk VM nahrazen: {} ({})", disk.display(), crate::build_info().1);
    Ok(())
}

/// VMware za behu drzi zamky `<jmeno>.lck` (slozky nebo soubory).
fn check_not_running(dir: &Path) -> Result<()> {
    for e in std::fs::read_dir(dir)? {
        let name = e?.file_name();
        if name.to_string_lossy().ends_with(".lck") {
            return Err(format!(
                "vmware: VM ve slozce {} bezi nebo nebyl radne vypnut (zamek {}); vypni ho ve VMware",
                dir.display(),
                name.to_string_lossy()
            )
            .into());
        }
    }
    Ok(())
}

/// `qemu-img` vedle nalezeneho QEMU (na Windows neni v PATH).
fn qemu_img(opts: &Options) -> Result<PathBuf> {
    let qemu = opts.arch.find_qemu()?;
    let exe = if cfg!(windows) { "qemu-img.exe" } else { "qemu-img" };
    let path = qemu.parent().map(|p| p.join(exe)).unwrap_or_else(|| PathBuf::from(exe));
    if path.exists() { Ok(path) } else { Ok(PathBuf::from(exe)) }
}

/// Casova znacka pro jmeno zalohy (sekundy od epochy hostitele; radi
/// se spravne i bez knihovny na datum).
fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Smaze nejstarsi zalohy nad [`BACKUPS`].
fn prune_backups(dir: &Path) -> Result<()> {
    let mut backups: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("magnumopus-") && n.ends_with(".vmdk"))
        })
        .collect();
    backups.sort();
    while backups.len() > BACKUPS {
        let old = backups.remove(0);
        std::fs::remove_file(&old)?;
        println!("xtask: smazana stara zaloha {}", old.display());
    }
    Ok(())
}
