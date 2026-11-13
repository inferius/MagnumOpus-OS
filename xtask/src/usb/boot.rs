//! Start obrazu USB v QEMU jako `usb-storage` na radici xHCI: stroj co
//! nejbliz skutecnemu PC (zadne virtio, framebuffer z firmwaru, klavesnice
//! a mys USB, sitova karta Intel). Firmware bootuje z flash disku, system
//! najde datovy oddil pres ovladac `usb-storage`.
//!
//! Testy (`cargo xtask test`, `cargo xtask usb --test`):
//! - [`test`]: start do plochy s terminalem ([`MARKERS`], vyvojovy profil
//!   zive USB s automatickym prihlasenim) a kopie logu na FAT
//!   oddil MO-LOGY (overeno na hostiteli crate `fatfs`),
//! - [`test_rescue`]: rezim "vytahni logy" (klavesa L pres QMP), logy
//!   "nainstalovaneho systemu" z disku NVMe na MO-LOGY, disk NVMe beze zmeny,
//! - [`test_user_image`]: obraz bez vyvojoveho profilu (jako obraz pro
//!   uzivatele) se automaticky neprihlasi ([`USER_MARKERS`]).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::arch::Arch;
use crate::{Result, target_dir};

/// Znacky uspesneho startu z USB (ovladac disku USB, FS z datoveho oddilu
/// flash disku, vyvojove prihlaseni, kompozitor a terminal v plose).
pub const MARKERS: &[&str] = &[
    "usb-storage: LUN 0:",
    "usb-storage: disk 0: nas oddil",
    "fsd: pripojeno 'MagnumOpus'",
    "accountd: vyvojove prihlaseni vyvojar",
    "init: kompozitor bezi",
    "init: termd (",
];

/// Znacky startu obrazu bez vyvojoveho profilu: sluzba uctu bez
/// vyvojoveho prihlaseni a prihlasovaci obrazovka.
pub const USER_MARKERS: &[&str] = &["fsd: pripojeno 'MagnumOpus'", "accountd: bezim", "init: kompozitor bezi", "compd: prihlasovaci obrazovka:"];

/// Jak dlouho smi start trvat (aarch64 v emulaci TCG).
const TIMEOUT: Duration = Duration::from_secs(420);

/// Argumenty QEMU pro stroj s flash diskem `image` (`extra` = dalsi
/// argumenty, napr. interni disk).
fn command(arch: &Arch, image: &Path, vars: &Path, serial: Option<&Path>, headless: bool, extra: &[String]) -> Result<Command> {
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
        .args(["-device", "qemu-xhci,id=xhci"]);
    // Prazdna cesta = bez flashky (start z nainstalovaneho disku).
    if !image.as_os_str().is_empty() {
        cmd.arg("-drive")
            .arg(format!("if=none,id=usbstick,format=raw,file={}", image.display()))
            .args(["-device", "usb-storage,bus=xhci.0,drive=usbstick,removable=on,bootindex=0"]);
    }
    cmd.args(["-device", "usb-kbd,bus=xhci.0"])
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
    cmd.args(extra);
    Ok(cmd)
}

/// Interaktivni start (okno QEMU, seriova linka v terminalu).
pub fn run(arch: &Arch, image: &Path) -> Result<()> {
    let vars = target_dir().join(format!("fw-{}-vars-usb.fd", arch.name));
    let status = command(arch, image, &vars, None, false, &[])?.status()?;
    if !status.success() {
        return Err(format!("QEMU skoncilo s {status}").into());
    }
    Ok(())
}

/// Test: start z flash disku az do plochy s terminalem (vyvojovy profil);
/// logd zkopiruje log na FAT oddil MO-LOGY.
pub fn test(arch: &Arch, image: &Path) -> Result<()> {
    let log = boot_until(arch, image, &[], false, "start", |t| {
        let mut m = missing_markers(t);
        if !t.contains("logd: kopie logu na oddil MO-LOGY") {
            m.push("logd: kopie logu na oddil MO-LOGY");
        }
        m
    })?;
    let fat = read_logy(image)?;
    let ok = fat_read(&fat, "MAGNUMOS/LOG01.TXT").is_some_and(|t| t.starts_with("MagnumOpus: log startu 1"));
    if !ok {
        return Err(format!("na oddilu MO-LOGY chybi MAGNUMOS/LOG01.TXT (log {})", log.display()).into());
    }
    println!("xtask: kopie logu na FAT oddilu flashky je");
    Ok(())
}

/// Test obrazu bez vyvojoveho profilu: start do prihlasovaci obrazovky,
/// v logu zadne vyvojove prihlaseni ani terminal.
pub fn test_user_image(arch: &Arch, image: &Path) -> Result<()> {
    let log = boot_until(arch, image, &[], false, "obraz bez vyvojoveho profilu", |t| {
        USER_MARKERS.iter().copied().filter(|m| !t.contains(m)).collect()
    })?;
    let text = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).into_owned())?;
    for bad in ["vyvojove prihlaseni", "init: termd ("] {
        if text.contains(bad) {
            return Err(format!("obraz bez vyvojoveho profilu: v logu je '{bad}' (log {})", log.display()).into());
        }
    }
    println!("xtask: obraz bez vyvojoveho profilu se automaticky neprihlasi (prihlasovaci obrazovka)");
    Ok(())
}

/// Test rezimu "vytahni logy": klavesa L v zavadeci (QMP), "nainstalovany"
/// system na disku NVMe s logy; po behu jsou jeho logy na oddilu MO-LOGY
/// a disk NVMe je bajt po bajtu stejny (jen cteni).
pub fn test_rescue(arch: &Arch, image: &Path) -> Result<()> {
    let inst = target_dir().join(format!("usb-installed-{}.img", std::process::id()));
    super::installed_disk(&inst)?;
    let before = std::fs::read(&inst)?;
    let extra = [
        String::from("-drive"),
        format!("if=none,id=inst,format=raw,file={}", inst.display()),
        String::from("-device"),
        String::from("nvme,drive=inst,serial=MO-INSTALLED"),
    ];
    let r = boot_until(arch, image, &extra, true, "vytazeni logu", |t| {
        ["rezim: vytahnout logy", "init: HOTOVO: logy a zaznamy padu"]
            .into_iter()
            .filter(|m| !t.contains(m))
            .collect()
    });
    let after = std::fs::read(&inst)?;
    let _ = std::fs::remove_file(&inst);
    r?;
    if before != after {
        return Err("rezim vytazeni logu ZMENIL disk nainstalovaneho systemu".into());
    }
    let fat = read_logy(image)?;
    let list = fat_read(&fat, "ZACHRANA/SEZNAM.TXT").ok_or("na oddilu MO-LOGY chybi ZACHRANA/SEZNAM.TXT")?;
    for want in ["/system/log/boot-000007.log", "/system/pady/000006-jadro-MO-1234-5678.crash"] {
        if !list.contains(want) {
            return Err(format!("SEZNAM.TXT neobsahuje {want}:\n{list}").into());
        }
    }
    println!("xtask: vytazeni logu prosel (disk nainstalovaneho systemu beze zmeny)");
    Ok(())
}

/// QMP prikaz: klavesa L.
const KEY_L: &str = r#"{"execute":"send-key","arguments":{"keys":[{"type":"qcode","data":"l"}]}}"#;

/// Spusti stroj a ceka, az `missing(log)` bude prazdne. `keys` = posilat
/// klavesu L pres QMP (nabidka zavadece). Vraci cestu logu.
pub(super) fn boot_until(
    arch: &Arch,
    image: &Path,
    extra: &[String],
    keys: bool,
    what: &str,
    missing: impl Fn(&str) -> Vec<&'static str>,
) -> Result<PathBuf> {
    let pid = std::process::id();
    let vars = target_dir().join(format!("fw-{}-vars-usb-{pid}.fd", arch.name));
    let log = target_dir().join(format!("serial-usb-{}-{pid}.log", arch.name));
    let _ = std::fs::remove_file(&log);
    let mut extra = extra.to_vec();
    let port = crate::qmp::free_port()?;
    if keys {
        extra.push(String::from("-qmp"));
        extra.push(crate::qmp::qemu_arg(port));
    }
    let mut cmd = command(arch, image, &vars, Some(&log), true, &extra)?;
    println!("xtask: {what} z USB v QEMU ({}), log {}", arch.name, log.display());
    let mut child = cmd.stdin(Stdio::null()).spawn()?;
    let start = Instant::now();
    let result = loop {
        let text = std::fs::read(&log).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        // Klavesa L, dokud zavadec volbu nepotvrdi.
        if keys && !text.contains("rezim: vytahnout logy") && start.elapsed() > Duration::from_secs(1) {
            let _ = crate::qmp::command(port, KEY_L);
        }
        let m = missing(&text);
        if m.is_empty() {
            // Chvile na posledni zapisy.
            std::thread::sleep(Duration::from_secs(3));
            break Ok(());
        }
        // Znacka 'CHYBA: ...' = beh uz neuspeje, necekat na limit.
        if let Some(f) = m.iter().find(|s| s.starts_with("CHYBA: ")) {
            std::thread::sleep(Duration::from_secs(1));
            break Err(String::from(*f));
        }
        if let Some(status) = child.try_wait()? {
            break Err(format!("QEMU skoncilo ({status}); v logu chybi: {}", m.join("; ")));
        }
        if text.contains("KERNEL PANIC") {
            break Err(String::from("jadro spadlo (KERNEL PANIC)"));
        }
        if start.elapsed() > TIMEOUT {
            break Err(format!("{what} nedobehl do {} s; v logu chybi: {}", TIMEOUT.as_secs(), m.join("; ")));
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&vars);
    match result {
        Ok(()) => {
            println!("xtask: {what} z USB prosel za {} s (log {})", start.elapsed().as_secs(), log.display());
            Ok(log)
        }
        Err(e) => Err(format!("{e} (log: {})", log.display()).into()),
    }
}

/// Oddil MO-LOGY z obrazu (do pameti).
fn read_logy(image: &Path) -> Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let (at, len) = super::image::logy_range(image)?;
    let mut f = std::fs::File::open(image)?;
    f.seek(SeekFrom::Start(at))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Soubor z FAT obrazu (nezavisla implementace `fatfs`).
fn fat_read(fat: &[u8], path: &str) -> Option<String> {
    use std::io::Read;
    let mut copy = fat.to_vec();
    let fs = fatfs::FileSystem::new(std::io::Cursor::new(&mut copy[..]), fatfs::FsOptions::new()).ok()?;
    let mut f = fs.root_dir().open_file(path).ok()?;
    let mut v = Vec::new();
    f.read_to_end(&mut v).ok()?;
    Some(String::from_utf8_lossy(&v).into_owned())
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
