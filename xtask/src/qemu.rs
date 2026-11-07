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

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::{Artifacts, Options, Result, datadisk, symbolize, target_dir};

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
    let fw_vars = target_dir().join(format!("fw-{}-vars-{}.fd", arch.name, opts.iommu));
    if !fw_vars.exists() {
        std::fs::copy(&fw_vars_template, &fw_vars)?;
    }

    let serial_log = target_dir().join(if opts.test {
        format!("serial-test-{}.log", arch.name)
    } else {
        format!("serial-{}.log", arch.name)
    });
    let accel = opts.accel.as_deref().unwrap_or(default_accel());
    let mut cmd = Command::new(&qemu);
    let translate_os_disk = match arch.name {
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
        .arg(pflash_arg(&fw_code, true))
        .arg("-drive")
        .arg(pflash_arg(&fw_vars, false))
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
        // Bez sitove karty, jinak firmware zbytecne zkousi PXE boot.
        .args(["-nic", "none"])
        // Seriova linka do terminalu a zaroven do souboru, aby se dal vystup
        // dohledat i po skonceni behu.
        .arg("-chardev")
        .arg(format!("stdio,id=serial0,logfile={}", serial_log.display()))
        .args(["-serial", "chardev:serial0"])
        .arg("-no-reboot");
    if opts.headless {
        cmd.args(["-display", "none"]);
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

    println!("xtask: spoustim {}", qemu.display());
    let result = if opts.test {
        run_test(cmd, opts)
    } else {
        run_interactive(cmd)
    };
    // Pri panice nebo fatalni vyjimce prelozit adresy na jmena funkci.
    if let Err(e) = symbolize::report(&serial_log, &artifacts.kernel) {
        eprintln!("xtask: preklad adres selhal: {e}");
    }
    println!("xtask: vystup seriove linky je v {}", serial_log.display());
    result
}

/// Stroj q35 s OVMF. Vraci, zda smi systemovy disk prochazet IOMMU.
fn machine_x86_64(cmd: &mut Command, accel: &str, opts: &Options) -> bool {
    cmd.arg("-machine")
        .arg(format!("q35,accel={accel}"))
        .args(["-cpu", "max", "-smp", "4", "-m", "512M"]);
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
    // Pod AMD-Vi firmware OVMF z disku s prekladem DMA nenabootuje, proto
    // tam systemovy disk prekladem neprochazi (ostatni disky ano).
    opts.iommu != "amd"
}

/// Stroj virt: GICv3 s ITS (MSI pro PCIe), volitelne SMMUv3, CPU Cortex-A76
/// (stejne jadro jako Raspberry Pi 5). Bez EL2 (virtualization=off), aby
/// loader i jadro bezely v EL1. Vraci, zda smi systemovy disk prochazet
/// IOMMU (SMMU je do zapnuti jadrem v rezimu bypass, firmware ho nevidi).
fn machine_aarch64(cmd: &mut Command, accel: &str, opts: &Options) -> bool {
    let iommu = if opts.iommu == "smmu" {
        ",iommu=smmuv3"
    } else {
        ""
    };
    cmd.arg("-machine")
        .arg(format!(
            "virt,gic-version=3,its=on,virtualization=off,secure=off{iommu},accel={accel}"
        ))
        .args(["-cpu", "cortex-a76", "-smp", "4", "-m", "1G"]);
    if !opts.headless {
        // Jednoduchy framebuffer v RAM (GOP firmwaru), bez ovladace v jadru.
        cmd.args(["-device", "ramfb"]);
    }
    if opts.test {
        cmd.args(["-semihosting-config", "enable=on,target=native"]);
    }
    true
}

fn run_interactive(mut cmd: Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("QEMU skoncilo s {status}").into());
    }
    Ok(())
}

fn run_test(mut cmd: Command, opts: &Options) -> Result<()> {
    let timeout = opts.arch.test_timeout;
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > timeout {
            child.kill()?;
            child.wait()?;
            return Err(format!("testy nedobehly do {} s", timeout.as_secs()).into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    match status.code() {
        Some(code) if code == opts.arch.test_success => {
            println!("xtask: testy prosly");
            Ok(())
        }
        code => Err(format!("testy selhaly (navratovy kod QEMU {code:?})").into()),
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
