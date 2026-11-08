//! Spusteni systemu v QEMU (interaktivne nebo jako test).
//!
//! - x86_64: stroj q35, firmware OVMF, IOMMU Intel VT-d nebo AMD-Vi, konec
//!   testu pres zarizeni isa-debug-exit.
//! - aarch64: stroj virt (GICv3 s ITS, volitelne SMMUv3), firmware edk2,
//!   konec testu pres semihosting (SYS_EXIT s kodem od jadra, 33 = uspech).
//!
//! Zvuk: radic intel-hda s kodekem hda-duplex (viz `audio.rs`).
//!
//! Disky jsou na obou stejne: systemovy (ESP s loaderem, jadrem a initfs),
//! maly pro testy ovladace a datovy s nasim souborovym systemem, vse jako
//! virtio-blk na PCI Express. Datovy disk muze byt i na NVMe nebo AHCI
//! (`--data-disk`). V testech navic radic NVMe a AHCI s testovacimi disky
//! (`testdisks.rs`).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{Artifacts, Options, Result, datadisk, qmp, symbolize, target_dir, testdisks};

/// Zvysit pri kazde zmene zarizeni PCI v QEMU (posune sloty disku).
const DEVICE_LAYOUT: u32 = 8;

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
    // Bez virtio-gpu (`--gpu none`) se sloty PCI take posunou.
    let no_gpu = if opts.gpu == "none" { "-nogpu" } else { "" };
    let fw_vars = target_dir().join(format!("fw-{}-vars-{}-l{DEVICE_LAYOUT}{no_gpu}.fd", arch.name, opts.iommu));
    if !fw_vars.exists() {
        std::fs::copy(&fw_vars_template, &fw_vars)?;
    }

    // Kazdy beh ma vlastni disky a testy i vlastni log: soubezne behy
    // (vic vlaken na jednom stroji) si jinak prepisovaly datovy disk a QMP
    // pomocnik cetl znacky testu z cizi seriove linky.
    let run = RunFiles::new(arch.name, opts.test);
    let serial_log = run.serial_log.clone();
    let qmp_port = Arc::new(AtomicU16::new(0));
    match opts.power_test {
        // Beh testu napajeni: misto vstupu tlacitko napajeni pres QMP.
        Some("button") => crate::powertest::spawn_button(serial_log.clone(), qmp_port.clone()),
        Some(_) => {}
        None => qmp::spawn_injector(serial_log.clone(), qmp_port.clone(), arch.ps2),
    }
    let mut hostfwd = opts.hostfwd.clone();
    let mut attempt = 0;
    let result = loop {
        attempt += 1;
        // Stary log nesmi obsahovat znacku testu vstupu z minuleho behu.
        let _ = std::fs::remove_file(&serial_log);
        let port = qmp::free_port()?;
        qmp_port.store(port, Ordering::Relaxed);
        let mut cmd = command(&qemu, image, artifacts, opts, (&fw_code, &fw_vars), &run, port, hostfwd.as_deref())?;
        // Port pro hostfwd uvolnit az ted (QEMU ho hned obsadi).
        if let Some(h) = opts.net_helper.borrow_mut().as_mut() {
            h.release_forwarded_port();
        }
        if opts.qemu_wsl {
            cmd = wsl_command(&cmd, opts.arch.qemu, opts.headless && opts.gpu != "2d");
        }
        println!("xtask: spoustim {}", if opts.qemu_wsl { String::from("QEMU ve WSL") } else { qemu.display().to_string() });
        if !opts.test {
            break run_interactive(cmd);
        }
        match run_test(cmd, opts, &serial_log) {
            Err(TestError::PortTaken(e)) if attempt < START_ATTEMPTS => {
                println!("xtask: QEMU nenastartovalo ({e}), novy pokus s jinymi porty ({attempt}/{START_ATTEMPTS})");
                if let Some(h) = opts.net_helper.borrow_mut().as_mut() {
                    hostfwd = Some(h.rearm_forwarded_port()?);
                }
            }
            Err(TestError::PortTaken(e)) => break Err(format!("QEMU nenastartovalo: {e}").into()),
            Err(TestError::Other(e)) => break Err(e),
            Ok(()) => break crate::audio::check(&run.wav, &serial_log, opts),
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
    run: &RunFiles,
    qmp_port: u16,
    hostfwd: Option<&str>,
) -> Result<Command> {
    let (fw_code, fw_vars) = fw;
    let serial_log = &run.serial_log;
    // Ve WSL je KVM (vnorena virtualizace).
    let accel = opts.accel.as_deref().unwrap_or(if opts.qemu_wsl { "kvm" } else { default_accel() });
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
        // Maly disk pro test ovladace (GPT s nasim oddilem 1 MiB, testy do
        // nej zapisuji).
        .arg("-drive")
        .arg(format!(
            "if=none,id=scratch,format=raw,file={}",
            testdisks::scratch(&run.scratch)?.display()
        ))
        .args(["-device", &blk("scratch")])
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
        // Druha sitova karta Intel 82574L (ovladac e1000e) ve vlastni siti
        // slirp; netd pouziva prvni kartu (virtio-net), test ovladacu tuto.
        // Posledni zarizeni PCI: sloty ostatnich se nemeni (DEVICE_LAYOUT
        // zustava). Emulovana karta jde pres IOMMU vzdy (bez prepinace).
        .args(["-netdev", "user,id=net1"])
        .args(["-device", "e1000e,netdev=net1,romfile="])
        // Seriova linka do terminalu a zaroven do souboru, aby se dal vystup
        // dohledat i po skonceni behu.
        .arg("-chardev")
        .arg(format!("stdio,id=serial0,logfile={}", serial_log.display()))
        .args(["-serial", "chardev:serial0"])
        .arg("-no-reboot");
    if opts.headless {
        // Venus potrebuje OpenGL i bez okna.
        // Ve WSL neni uzel DRM pro egl-headless: GTK s OpenGL (EGL) pod
        // xvfb-run; SDL tam kontext OpenGL 3+ pro virgl nevytvori.
        let display = match (opts.gpu.as_str(), opts.qemu_wsl) {
            ("2d" | "none", _) => "none",
            (_, true) => "gtk,gl=on",
            _ => "egl-headless",
        };
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
    disk_controllers(&mut cmd, opts, run, &blk)?;
    // Vyukove zarizeni pro ukazkovy ovladac (az za ostatnimi, sloty PCI
    // drivejsich zarizeni se neposunou).
    if opts.edu {
        cmd.args(["-device", crate::drivers::EDU_DEVICE]);
    }
    // Zvuk: radice Intel HDA (ovladac hda, sluzba audiod); v testu do WAV.
    // Az za ostatnimi zarizenimi, aby se jejich sloty PCI neposunuly.
    crate::audio::qemu_args(&mut cmd, opts, &run.wav);
    cmd.args(&opts.qemu_args);
    Ok(cmd)
}

/// Datovy disk (na zvolenem radici) a v testech radice NVMe a AHCI
/// s testovacimi disky. Zarizeni jsou az za ostatnimi, aby se jejich sloty
/// PCI neposunuly.
fn disk_controllers(cmd: &mut Command, opts: &Options, run: &RunFiles, blk: &dyn Fn(&str) -> String) -> Result<()> {
    // Datovy disk s nasim souborovym systemem (sluzba fsd); TRIM povolen.
    // `cache=unsafe`: flush disku (kazdy commit fsd) nedela fsync
    // souboru na hostiteli. Disk vznika pri kazdem behu znovu, takze
    // trvanlivost pri padu hostitele nema cenu; bez toho stal radek
    // shellu se zapisem (nekolik commitu: klon, prevzeti, zahozeni)
    // na vytizenem hostiteli sekundy. Vypadek proudu testuje fsd
    // simulaci, ne flush hostitele. V beznem behu bezpecne.
    let cache = if opts.test { ",cache=unsafe" } else { "" };
    let drive = |id: &str, path: &Path| format!("if=none,id={id},format=raw,discard=unmap{cache},file={}", path.display());
    let raw = datadisk::create(&run.data)?;
    // Na NVMe a AHCI jako na skutecnem pocitaci: GPT s nasim oddilem.
    let data = match opts.data_disk.as_str() {
        "virtio" => raw,
        "nvme4k" => testdisks::wrap_data(&raw, &run.data_gpt, 4096)?,
        _ => testdisks::wrap_data(&raw, &run.data_gpt, 512)?,
    };
    cmd.arg("-drive").arg(drive("data", &data));
    let ahci = opts.test || opts.data_disk == "ahci";
    if ahci {
        cmd.args(["-device", "ahci,id=ahci0"]);
    }
    match opts.data_disk.as_str() {
        "nvme" => cmd.args(["-device", "nvme,drive=data,serial=MO-DATA"]),
        // Disk se sektorem 4 KiB (jako NVMe naformatovane na 4 KiB).
        "nvme4k" => cmd.args([
            "-device",
            "nvme,drive=data,serial=MO-DATA,logical_block_size=4096,physical_block_size=4096",
        ]),
        "ahci" => cmd.args(["-device", "ide-hd,drive=data,bus=ahci0.2"]),
        _ => cmd.args(["-device", &blk("data")]),
    };
    if opts.test {
        cmd.arg("-drive")
            .arg(drive("nvmetest", &testdisks::nvme(&run.nvme)?))
            .args(["-device", "nvme,drive=nvmetest,serial=MO-NVME-TEST"])
            .arg("-drive")
            .arg(drive("nvme4ktest", &testdisks::nvme4k(&run.nvme4k)?))
            .args([
                "-device",
                "nvme,drive=nvme4ktest,serial=MO-NVME-4K,logical_block_size=4096,physical_block_size=4096",
            ])
            .arg("-drive")
            .arg(drive("ahcitest", &testdisks::ahci(&run.ahci)?))
            .args(["-device", "ide-hd,drive=ahcitest,bus=ahci0.0"])
            .arg("-drive")
            .arg(drive("foreign", &testdisks::foreign(&run.foreign)?))
            .args(["-device", "ide-hd,drive=foreign,bus=ahci0.1"]);
    }
    Ok(())
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
/// DMA pres IOMMU. Rezim 2D funguje s kazdym displejem QEMU (i bez okna);
/// `--gpu none` virtio-gpu neprida.
fn gpu_device(cmd: &mut Command, opts: &Options) {
    match opts.gpu.as_str() {
        // Bez virtio-gpu: GUI bezi nad framebufferem z firmwaru (`fbdisplay`;
        // x86_64 VGA, aarch64 ramfb).
        "none" => {}
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

/// Windows cesta `X:\a\b` -> WSL `/mnt/x/a/b` (vsechny vyskyty v argumentu,
/// cesta konci carkou nebo koncem argumentu).
fn to_wsl_paths(arg: &str) -> String {
    let b = arg.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let drive = b[i].is_ascii_alphabetic()
            && b.get(i + 1) == Some(&b':')
            && matches!(b.get(i + 2), Some(b'\\') | Some(b'/'))
            && (i == 0 || matches!(b[i - 1], b'=' | b','));
        if drive {
            out.push_str("/mnt/");
            out.push((b[i] as char).to_ascii_lowercase());
            i += 2;
            while i < b.len() && b[i] != b',' {
                out.push(if b[i] == b'\\' { '/' } else { b[i] as char });
                i += 1;
            }
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// Prikaz QEMU prevedeny na beh ve WSL: `wsl.exe -d <distro> -- <qemu> ...`.
fn wsl_command(cmd: &Command, qemu_name: &str, xvfb: bool) -> Command {
    let distro = std::env::var("QEMU_WSL_DISTRO").unwrap_or_else(|_| String::from("Ubuntu"));
    let prefix = std::env::var("QEMU_WSL_PREFIX").unwrap_or_else(|_| String::from("~/venus/prefix"));
    // Bez shellu by se `~` nerozvinula: spustit pres bash -c s exec.
    // Knihovny z prefixu (virglrenderer s Venusem); bez okna virtualni X
    // server (OpenGL softwarove, Vulkan hostitele lavapipe nebo GPU).
    // OpenGL pod Xvfb jen softwarove (llvmpipe): ovladac D3D12 z WSL tam
    // kontext nevytvori.
    let (env, run) = if xvfb {
        ("export LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe; ", "xvfb-run -a -s '-screen 0 1280x1024x24 +extension GLX' ")
    } else {
        ("", "")
    };
    let mut line = format!(
        "export LD_LIBRARY_PATH={prefix}/lib/x86_64-linux-gnu:{prefix}/lib; export VIRGL_DEBUG=${{VIRGL_DEBUG:-}} VKR_DEBUG=${{VKR_DEBUG:-}}; {env}exec {run}{prefix}/bin/{qemu_name}"
    );
    for a in cmd.get_args() {
        let a = to_wsl_paths(&a.to_string_lossy());
        line.push(' ');
        line.push('\'');
        line.push_str(&a.replace('\'', "'\\''"));
        line.push('\'');
    }
    let mut w = Command::new("wsl.exe");
    w.args(["-d", &distro, "--", "bash", "-lc", &line]);
    w
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

fn run_test(mut cmd: Command, opts: &Options, serial_log: &Path) -> std::result::Result<(), TestError> {
    // Opakovany beh (--repeat): kazde dalsi kolo az 60 s navic. Beh testu
    // napajeni je kratky (jen start a vypnuti).
    let timeout = match opts.power_test {
        Some(_) => Duration::from_secs(300),
        None => opts.arch.test_timeout + std::time::Duration::from_secs(60) * opts.test_repeat.saturating_sub(1),
    };
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
    if let Some(mode) = opts.power_test {
        // Uspech = QEMU skoncilo samo (vypnuti/restart ACPI), kod 0, a log
        // ma znacky celeho postupu.
        return match (status.code(), crate::powertest::check_log(serial_log, mode)) {
            (Some(0), Ok(())) => Ok(()),
            (Some(0), Err(e)) => Err(TestError::Other(format!("test napajeni ({mode}): {e}").into())),
            (code, _) => Err(TestError::Other(
                format!("test napajeni ({mode}): QEMU skoncilo s kodem {code:?} (ocekavano 0 po vypnuti ACPI)").into(),
            )),
        };
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

/// Soubory jednoho behu QEMU. Disky jsou jen pro tento beh (cislo procesu
/// xtask v nazvu) a po skonceni se smazou; log testu take nese cislo
/// procesu, aby soubezne behy nesdilely znacky pro QMP. Interaktivni beh
/// pise do `target/serial-<arch>.log` jako dosud.
struct RunFiles {
    data: PathBuf,
    /// Datovy disk s GPT (na NVMe/AHCI).
    data_gpt: PathBuf,
    scratch: PathBuf,
    /// Zvuk z QEMU (backend wav).
    wav: PathBuf,
    nvme: PathBuf,
    nvme4k: PathBuf,
    ahci: PathBuf,
    foreign: PathBuf,
    serial_log: PathBuf,
}

impl RunFiles {
    fn new(arch: &str, test: bool) -> Self {
        let pid = std::process::id();
        let t = target_dir();
        Self {
            data: t.join(format!("data-{pid}.img")),
            data_gpt: t.join(format!("data-gpt-{pid}.img")),
            scratch: t.join(format!("scratch-{pid}.img")),
            wav: t.join(format!("audio-{arch}-{pid}.wav")),
            nvme: t.join(format!("nvme-test-{pid}.img")),
            nvme4k: t.join(format!("nvme4k-test-{pid}.img")),
            ahci: t.join(format!("ahci-test-{pid}.img")),
            foreign: t.join(format!("foreign-{pid}.img")),
            serial_log: t.join(if test {
                format!("serial-test-{arch}-{pid}.log")
            } else {
                format!("serial-{arch}.log")
            }),
        }
    }
}

impl Drop for RunFiles {
    fn drop(&mut self) {
        // `OS_KEEP_DATA=1`: datovy disk nechat (rozbor stavu FS po behu,
        // napr. mereni na hostiteli nad skutecnym obsahem).
        if std::env::var_os("OS_KEEP_DATA").is_some() {
            eprintln!("xtask: datovy disk zustava v {} ({})", self.data.display(), self.data_gpt.display());
        } else {
            let _ = std::fs::remove_file(&self.data);
            let _ = std::fs::remove_file(&self.data_gpt);
        }
        for f in [&self.scratch, &self.nvme, &self.nvme4k, &self.ahci, &self.foreign, &self.wav] {
            let _ = std::fs::remove_file(f);
        }
    }
}

fn pflash_arg(path: &Path, readonly: bool) -> String {
    let ro = if readonly { ",readonly=on" } else { "" };
    format!("if=pflash,format=raw{ro},file={}", path.display())
}
