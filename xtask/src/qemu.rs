//! Spusteni systemu v QEMU (interaktivne nebo jako test).
//!
//! - x86_64: stroj q35, firmware OVMF, IOMMU Intel VT-d nebo AMD-Vi, konec
//!   testu pres zarizeni isa-debug-exit.
//! - aarch64: stroj virt (GICv3 s ITS, volitelne SMMUv3), firmware edk2,
//!   konec testu pres semihosting (SYS_EXIT s kodem od jadra, 33 = uspech).
//!
//! Disky jsou na obou stejne: systemovy (ESP s loaderem, jadrem a initfs),
//! maly prazdny pro testy ovladace a datovy s nasim souborovym systemem, vse
//! jako virtio-blk na PCI Express.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Artifacts, Options, Result, datadisk, qmp, symbolize, target_dir};

/// Zvysit pri kazde zmene zarizeni PCI v QEMU (posune sloty disku).
const DEVICE_LAYOUT: u32 = 6;

/// Pokusy o start QEMU, kdyz mu port (QMP, hostfwd) mezitim vzal jiny
/// proces (soubezne behy testu).
const START_ATTEMPTS: u32 = 3;

/// Jak dlouho po startu se konec QEMU s chybou portu povazuje za nepovedeny
/// start (pozdeji uz bezi system).
const START_WINDOW: Duration = Duration::from_secs(15);

fn default_accel() -> &'static str {
    if cfg!(target_os = "linux") && Path::new("/dev/kvm").exists() {
        "kvm"
    } else {
        "tcg"
    }
}

pub fn run(image: &Path, artifacts: &Artifacts, opts: &Options) -> Result<()> {
    let arch = opts.arch;
    let qemu = arch.find_qemu()?;
    let (fw_code, fw_vars_template) = arch.find_firmware(&qemu)?;

    // Firmware do vars zapisuje (boot polozky), proto pracujeme s kopii.
    // Vlastni kopie pro kazdy rezim IOMMU: IOMMU muze zabrat slot PCI a disky
    // se posunou, ulozene bootovaci polozky by pak nesedely.
    // Verze rozlozeni zarizeni je v nazvu: po zmene zarizeni PCI by stare
    // bootovaci polozky nesedely a firmware by skoncil ve svem shellu.
    let fw_vars = target_dir().join(format!("fw-{}-vars-{}-l{DEVICE_LAYOUT}.fd", arch.name, opts.iommu));
    if !fw_vars.exists() {
        std::fs::copy(&fw_vars_template, &fw_vars)?;
    }

    let serial_log = target_dir().join(if opts.test {
        format!("serial-test-{}.log", arch.name)
    } else {
        format!("serial-{}.log", arch.name)
    });
    let qmp_port = Arc::new(AtomicU16::new(0));
    qmp::spawn_injector(serial_log.clone(), qmp_port.clone(), arch.ps2);
    let mut hostfwd = opts.hostfwd.clone();
    let mut attempt = 0;
    let result = loop {
        attempt += 1;
        // Stary log nesmi obsahovat znacku testu vstupu z minuleho behu.
        let _ = std::fs::remove_file(&serial_log);
        let port = qmp::free_port()?;
        qmp_port.store(port, Ordering::Relaxed);
        let cmd = command(&qemu, image, artifacts, opts, (&fw_code, &fw_vars), &serial_log, port, hostfwd.as_deref())?;
        // Port pro hostfwd uvolnit az ted (QEMU ho hned obsadi).
        if let Some(h) = opts.net_helper.borrow_mut().as_mut() {
            h.release_forwarded_port();
        }
        println!("xtask: spoustim {}", qemu.display());
        if !opts.test {
            break run_interactive(cmd);
        }
        match run_test(cmd, opts) {
            Err(TestError::PortTaken(e)) if attempt < START_ATTEMPTS => {
                println!("xtask: QEMU nenastartovalo ({e}), novy pokus s jinymi porty ({attempt}/{START_ATTEMPTS})");
                if let Some(h) = opts.net_helper.borrow_mut().as_mut() {
                    hostfwd = Some(h.rearm_forwarded_port()?);
                }
            }
            Err(TestError::PortTaken(e)) => break Err(format!("QEMU nenastartovalo: {e}").into()),
            Err(TestError::Other(e)) => break Err(e),
            Ok(()) => break Ok(()),
        }
    };
    // Pri panice nebo fatalni vyjimce prelozit adresy na jmena funkci.
    if let Err(e) = symbolize::report(&serial_log, &artifacts.kernel) {
        eprintln!("xtask: preklad adres selhal: {e}");
    }
    println!("xtask: vystup seriove linky je v {}", serial_log.display());
    result
}

/// Prikaz QEMU s porty `qmp_port` a `hostfwd` (pri opakovanem startu nove).
#[allow(clippy::too_many_arguments)]
fn command(
    qemu: &Path,
    image: &Path,
    artifacts: &Artifacts,
    opts: &Options,
    fw: (&Path, &Path),
    serial_log: &Path,
    qmp_port: u16,
    hostfwd: Option<&str>,
) -> Result<Command> {
    let (fw_code, fw_vars) = fw;
    let accel = opts.accel.as_deref().unwrap_or(default_accel());
    let mut cmd = Command::new(qemu);
    let translate_os_disk = match opts.arch.name {
        "x86_64" => machine_x86_64(&mut cmd, accel, opts),
        _ => machine_aarch64(&mut cmd, accel, opts),
    };
    let iommu_on = opts.iommu != "none";
    // Disky jen v modernim rezimu virtio a s prekladem DMA pres IOMMU.
    let blk = |drive: &str| {
        let translate = iommu_on && (drive != "osdisk" || translate_os_disk);
        format!(
            "virtio-blk-pci,drive={drive},disable-legacy=on,iommu_platform={}",
            if translate { "on" } else { "off" }
        )
    };
    cmd.arg("-drive")
        .arg(pflash_arg(fw_code, true))
        .arg("-drive")
        .arg(pflash_arg(fw_vars, false))
        .arg("-drive")
        .arg(format!(
            "if=none,id=osdisk,format=raw,file={}",
            image.display()
        ))
        .args(["-device", &blk("osdisk")])
        // Maly prazdny disk jen pro test ovladace (testy do nej zapisuji).
        .arg("-drive")
        .arg(format!(
            "if=none,id=scratch,format=raw,file={}",
            scratch_disk()?.display()
        ))
        .args(["-device", &blk("scratch")])
        // Datovy disk s nasim souborovym systemem (sluzba fsd); TRIM povolen.
        .arg("-drive")
        .arg(format!(
            "if=none,id=data,format=raw,discard=unmap,file={}",
            datadisk::create(&target_dir())?.display()
        ))
        .args(["-device", &blk("data")])
        .arg("-qmp")
        .arg(qmp::qemu_arg(qmp_port))
        // Sitova karta virtio-net v user-mode siti (slirp: host 10.0.2.15,
        // brana a hostitel 10.0.2.2, DNS 10.0.2.3). Bez option ROM
        // (`romfile=`), aby firmware nezkousel bootovat ze site.
        .arg("-netdev")
        .arg(match hostfwd {
            Some(f) => format!("user,id=net0,hostfwd={f}"),
            None => String::from("user,id=net0"),
        })
        .args([
            "-device",
            "virtio-net-pci,netdev=net0,disable-legacy=on,iommu_platform=on,romfile=",
        ])
        // Zdroj nahody z hostitele (doplnuje entropii jadra, viz
        // docs/cs/syscally.md RANDOM_ADD).
        .args(["-device", "virtio-rng-pci,disable-legacy=on,iommu_platform=on"])
        // Seriova linka do terminalu a zaroven do souboru, aby se dal vystup
        // dohledat i po skonceni behu.
        .arg("-chardev")
        .arg(format!("stdio,id=serial0,logfile={}", serial_log.display()))
        .args(["-serial", "chardev:serial0"])
        .arg("-no-reboot");
    if opts.headless {
        // Venus potrebuje OpenGL i bez okna.
        let display = if opts.gpu == "2d" { "none" } else { "egl-headless" };
        cmd.args(["-display", display]);
    }
    if opts.gdb {
        cmd.args(["-s", "-S"]);
        println!("xtask: QEMU ceka na GDB na localhost:1234, napr.:");
        println!(
            "  gdb {} -ex \"target remote localhost:1234\"",
            artifacts.kernel.display()
        );
    }
    cmd.args(&opts.qemu_args);
    Ok(cmd)
}

/// Stroj q35 s OVMF. Vraci, zda smi systemovy disk prochazet IOMMU.
fn machine_x86_64(cmd: &mut Command, accel: &str, opts: &Options) -> bool {
    cmd.arg("-machine")
        .arg(format!("q35,accel={accel}"))
        .args(["-cpu", "max", "-smp", "4", "-m", "512M"]);
    // Graficka karta s vlastnim id (jinak stejna jako vychozi `-vga std`
    // na 00:01.0): k jeji konzoli se navazi klavesnice a mys virtio, viz nize.
    cmd.args(["-vga", "none", "-device", "VGA,id=video0"]);
    // IOMMU (Intel VT-d nebo AMD-Vi) pred zarizenimi PCI, ktera preklada;
    // preklad preruseni zatim ne (intremap=off).
    match opts.iommu.as_str() {
        "intel" => {
            cmd.args(["-device", "intel-iommu,intremap=off"]);
        }
        "amd" => {
            cmd.args(["-device", "amd-iommu,dma-remap=on,intremap=off"]);
        }
        _ => {}
    }
    if opts.test {
        cmd.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    }
    // Klavesnice a mys virtio (ovladac virtio-input, sluzba inputd),
    // navazane na konzoli grafiky: vstup z okna QEMU a udalosti QMP
    // s `device=video0` jdou do nich, udalosti QMP bez zarizeni do
    // PS/2 (radic i8042 ma q35 vzdy). QMP slouzi testu vstupu.
    cmd.args(["-device", "virtio-keyboard-pci,iommu_platform=on,display=video0"])
        .args(["-device", "virtio-mouse-pci,iommu_platform=on,display=video0"])
        // USB: radic xHCI s klavesnici (ovladace xhci, usb-hub, usb-hid),
        // navazanou na konzoli druhe graficke karty, aby ji QMP umelo
        // oslovit zvlast (`device=video1`). Mys USB nema vlastnost
        // `display` a brala by udalosti PS/2 - test ji pripoji az za behu.
        .args(["-device", "secondary-vga,id=video1"])
        .args(["-device", "qemu-xhci,id=xhci"])
        .args(["-device", "usb-kbd,id=usbkbd0,bus=xhci.0,display=video1"]);
    gpu_device(cmd, opts);
    // Pod AMD-Vi firmware OVMF z disku s prekladem DMA nenabootuje, proto
    // tam systemovy disk prekladem neprochazi (ostatni disky ano).
    opts.iommu != "amd"
}

/// Stroj virt: GICv3 s ITS (MSI pro PCIe), volitelne SMMUv3, CPU Cortex-A76
/// (stejne jadro jako Raspberry Pi 5). S EL2 (virtualization=on): firmware
/// bezi v EL2 a loader sestupuje do EL1 stejne jako na realnem hardwaru
/// (PSCI pak pres SMC). Vraci, zda smi systemovy disk prochazet
/// IOMMU (SMMU je do zapnuti jadrem v rezimu bypass, firmware ho nevidi).
fn machine_aarch64(cmd: &mut Command, accel: &str, opts: &Options) -> bool {
    let iommu = if opts.iommu == "smmu" {
        ",iommu=smmuv3"
    } else {
        ""
    };
    cmd.arg("-machine")
        .arg(format!(
            "virt,gic-version=3,its=on,virtualization=on,secure=off{iommu},accel={accel}"
        ))
        .args(["-cpu", "cortex-a76", "-smp", "4", "-m", "1G"]);
    // Jednoduchy framebuffer v RAM (GOP firmwaru, bez ovladace v jadru) jako
    // konzole `video0` a druha karta `video1`: k nim se navazi klavesnice a mys
    // virtio a klavesnice USB stejne jako na x86 (udalosti QMP testu vstupu).
    // Druha karta je bochs-display, pro kterou edk2 na ARM nema ovladac:
    // virtio-gpu by firmware vzal jako GOP a jeho framebuffer by po konci
    // boot services uz nikdo neobnovoval (prazdna obrazovka).
    cmd.args(["-device", "ramfb,id=video0"])
        .args(["-device", "bochs-display,id=video1"])
        .args(["-device", "virtio-keyboard-pci,iommu_platform=on,display=video0"])
        .args(["-device", "virtio-mouse-pci,iommu_platform=on,display=video0"])
        .args(["-device", "qemu-xhci,id=xhci"])
        .args(["-device", "usb-kbd,id=usbkbd0,bus=xhci.0,display=video1"]);
    gpu_device(cmd, opts);
    if opts.test {
        cmd.args(["-semihosting-config", "enable=on,target=native"]);
    }
    true
}

/// Grafika virtio-gpu (ovladac `virtio-gpu`, kompozitor `compd`) s prekladem
/// DMA pres IOMMU. Rezim 2D funguje s kazdym displejem QEMU (i bez okna).
fn gpu_device(cmd: &mut Command, opts: &Options) {
    match opts.gpu.as_str() {
        // Venus: Vulkan pres virtio-gpu (blob zdroje v pameti hostitele
        // mapovane pres BAR). Potrebuje displej s OpenGL (`egl-headless`
        // bez okna). Bez IOMMU: blob pamet hostitele preklad DMA nepouziva.
        "venus" => {
            cmd.args([
                "-device",
                "virtio-gpu-gl-pci,id=gpu0,disable-legacy=on,venus=on,blob=on,hostmem=256M,xres=1280,yres=800",
            ]);
        }
        // virgl 3D (OpenGL hostitele); DMA pres IOMMU jako 2D.
        "gl" => {
            cmd.args(["-device", "virtio-gpu-gl-pci,id=gpu0,disable-legacy=on,iommu_platform=on,xres=1280,yres=800"]);
        }
        _ => {
            cmd.args(["-device", "virtio-gpu-pci,id=gpu0,disable-legacy=on,iommu_platform=on,xres=1280,yres=800"]);
        }
    }
}

fn run_interactive(mut cmd: Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("QEMU skoncilo s {status}").into());
    }
    Ok(())
}

enum TestError {
    /// QEMU skoncilo hned po startu, protoze port nekdo obsadil.
    PortTaken(String),
    Other(Box<dyn std::error::Error>),
}

impl<E: Into<Box<dyn std::error::Error>>> From<E> for TestError {
    fn from(e: E) -> Self {
        Self::Other(e.into())
    }
}

/// Hlaska QEMU o obsazenem portu (QMP nebo hostfwd), Linux i Windows.
fn port_error(stderr: &str) -> Option<String> {
    const SIGNS: [&str; 5] = ["Address already in use", "host forwarding", "Failed to bind", "10048", "10013"];
    let line = stderr.lines().find(|l| SIGNS.iter().any(|s| l.contains(s)))?;
    Some(line.trim().to_string())
}

fn run_test(mut cmd: Command, opts: &Options) -> std::result::Result<(), TestError> {
    // Opakovany beh (--repeat): kazde dalsi kolo az 60 s navic.
    let timeout = opts.arch.test_timeout + std::time::Duration::from_secs(60) * opts.test_repeat.saturating_sub(1);
    // Chybovy vystup QEMU jde dal na terminal a zaroven do bufferu (hlaska
    // o obsazenem portu => novy pokus).
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let stderr = Arc::new(Mutex::new(String::new()));
    let copier = child.stderr.take().map(|mut pipe| {
        let stderr = stderr.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buf[..n]);
                eprint!("{text}");
                if let Ok(mut s) = stderr.lock() {
                    s.push_str(&text);
                }
            }
        })
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > timeout {
            child.kill()?;
            child.wait()?;
            return Err(TestError::Other(format!("testy nedobehly do {} s", timeout.as_secs()).into()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if let Some(t) = copier {
        let _ = t.join();
    }
    match status.code() {
        Some(code) if code == opts.arch.test_success => {
            println!("xtask: testy prosly");
            Ok(())
        }
        code => {
            if start.elapsed() < START_WINDOW
                && let Some(e) = stderr.lock().ok().and_then(|s| port_error(&s))
            {
                return Err(TestError::PortTaken(e));
            }
            Err(TestError::Other(format!("testy selhaly (navratovy kod QEMU {code:?})").into()))
        }
    }
}

/// Prazdny disk 1 MiB pro testy ovladace (pri kazdem behu znovu vynulovany).
fn scratch_disk() -> Result<PathBuf> {
    let path = target_dir().join("scratch.img");
    std::fs::write(&path, vec![0u8; 1024 * 1024])?;
    Ok(path)
}

fn pflash_arg(path: &Path, readonly: bool) -> String {
    let ro = if readonly { ",readonly=on" } else { "" };
    format!("if=pflash,format=raw{ro},file={}", path.display())
}
