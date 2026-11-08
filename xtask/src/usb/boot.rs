//! Start obrazu USB v QEMU jako `usb-storage` na radici xHCI: stroj co
//! nejbliz skutecnemu PC (zadne virtio, framebuffer z firmwaru, klavesnice
//! a mys USB, sitova karta Intel). Firmware bootuje z flash disku, system
//! najde datovy oddil pres ovladac `usb-storage`.
//!
//! Test (`cargo xtask test`, `cargo xtask usb --test`) ceka v logu seriove
//! linky na [`MARKERS`] a QEMU pak ukonci.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::arch::Arch;
use crate::{Result, target_dir};

/// Znacky uspesneho startu z USB (ovladac disku USB, FS z datoveho oddilu
/// flash disku, kompozitor a terminal v GUI).
pub const MARKERS: &[&str] = &[
    "usb-storage: LUN 0:",
    "usb-storage: disk 0: nas oddil",
    "fsd: pripojeno 'MagnumOpus'",
    "init: kompozitor bezi",
    "init: termd (",
];

/// Jak dlouho smi start trvat (aarch64 v emulaci TCG).
const TIMEOUT: Duration = Duration::from_secs(420);

/// Argumenty QEMU pro stroj s flash diskem `image`.
fn command(arch: &Arch, image: &Path, vars: &Path, serial: Option<&Path>, headless: bool) -> Result<Command> {
    let qemu = arch.find_qemu()?;
    let (code, template) = arch.find_firmware(&qemu)?;
    // Cerstve promenne firmwaru: bez starych bootovacich polozek bootuje
    // z vyjimatelneho media jako skutecne PC s novou flashkou.
    std::fs::copy(&template, vars)?;
    let mut cmd = Command::new(&qemu);
    if arch.name == "x86_64" {
        // Skutecne PC casto nema IOMMU zapnutou: system musi fungovat i bez.
        cmd.args(["-machine", "q35", "-cpu", "max", "-smp", "4", "-m", "1G"])
            .args(["-vga", "std"]);
    } else {
        cmd.args(["-machine", "virt,gic-version=3,its=on,virtualization=on,secure=off"])
            .args(["-cpu", "cortex-a76", "-smp", "4", "-m", "1G"])
            .args(["-device", "ramfb"]);
    }
    cmd.arg("-drive")
        .arg(crate::qemu::pflash_arg(&code, true))
        .arg("-drive")
        .arg(crate::qemu::pflash_arg(vars, false))
        .args(["-device", "qemu-xhci,id=xhci"])
        .arg("-drive")
        .arg(format!("if=none,id=usbstick,format=raw,file={}", image.display()))
        .args(["-device", "usb-storage,bus=xhci.0,drive=usbstick,removable=on,bootindex=0"])
        .args(["-device", "usb-kbd,bus=xhci.0"])
        .args(["-device", "usb-tablet,bus=xhci.0"])
        .args(["-netdev", "user,id=net0"])
        .args(["-device", "e1000e,netdev=net0,romfile="])
        .arg("-no-reboot");
    match serial {
        Some(log) => cmd.arg("-serial").arg(format!("file:{}", log.display())),
        None => cmd.args(["-serial", "stdio"]),
    };
    if headless {
        cmd.args(["-display", "none"]);
    }
    Ok(cmd)
}

/// Interaktivni start (okno QEMU, seriova linka v terminalu).
pub fn run(arch: &Arch, image: &Path) -> Result<()> {
    let vars = target_dir().join(format!("fw-{}-vars-usb.fd", arch.name));
    let status = command(arch, image, &vars, None, false)?.status()?;
    if !status.success() {
        return Err(format!("QEMU skoncilo s {status}").into());
    }
    Ok(())
}

/// Test: start z flash disku az do GUI s terminalem.
pub fn test(arch: &Arch, image: &Path) -> Result<()> {
    let pid = std::process::id();
    let vars = target_dir().join(format!("fw-{}-vars-usb-{pid}.fd", arch.name));
    let log = target_dir().join(format!("serial-usb-{}-{pid}.log", arch.name));
    let _ = std::fs::remove_file(&log);
    let mut cmd = command(arch, image, &vars, Some(&log), true)?;
    println!("xtask: start z USB v QEMU ({}), log {}", arch.name, log.display());
    let mut child = cmd.stdin(Stdio::null()).spawn()?;
    let start = Instant::now();
    let result = loop {
        let text = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let missing = missing_markers(&text);
        if missing.is_empty() {
            break Ok(());
        }
        if let Some(status) = child.try_wait()? {
            break Err(format!("QEMU skoncilo ({status}) pred startem GUI; v logu chybi: {}", missing.join("; ")));
        }
        if text.contains("KERNEL PANIC") {
            break Err(String::from("jadro spadlo (KERNEL PANIC)"));
        }
        if start.elapsed() > TIMEOUT {
            break Err(format!("start z USB nedobehl do {} s; v logu chybi: {}", TIMEOUT.as_secs(), missing.join("; ")));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&vars);
    match result {
        Ok(()) => {
            println!("xtask: start z USB prosel za {} s (log {})", start.elapsed().as_secs(), log.display());
            Ok(())
        }
        Err(e) => Err(format!("{e} (log: {})", log.display()).into()),
    }
}

/// Znacky, ktere v logu chybi (terminal: radek s `termd (` a `spusten`).
pub fn missing_markers(text: &str) -> Vec<&'static str> {
    MARKERS
        .iter()
        .copied()
        .filter(|m| {
            if *m == "init: termd (" {
                !text.lines().any(|l| l.contains(m) && l.contains("spusten"))
            } else {
                !text.contains(m)
            }
        })
        .collect()
}

/// Obraz USB pro test (mensi nez vychozi: rychlejsi tvorba).
pub fn test_image_path(arch: &Arch) -> PathBuf {
    target_dir().join(format!("usb-{}-test-{}.img", arch.name, std::process::id()))
}
