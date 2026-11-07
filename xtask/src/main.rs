//! Build a spousteni systemu: `cargo xtask <prikaz>`.
//!
//! Prikazy:
//! - `build`  - prelozi kernel, loader a programy userspace
//! - `image`  - build + bootovatelny obraz disku (GPT + ESP s FAT32)
//! - `run`    - image + spusteni v QEMU s UEFI firmwarem
//! - `test`   - testy na hostiteli, pak jadro a userspace v QEMU
//! - `check`  - jen kontrola prekladu jadra
//!
//! Prepinace: `--release`, `--arch <x86_64|aarch64>` (vsechny prikazy), pro
//! `run` navic `--gdb`, `--headless`, `--accel <tcg|kvm|whpx|hvf>` a cokoli
//! za `--` se preda primo QEMU.

mod arch;
mod datadisk;
mod docs;
mod highlight;
mod image;
mod qemu;
mod qmp;
mod symbolize;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use arch::Arch;

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

const USAGE: &str = "\
pouziti: cargo xtask <prikaz> [prepinace]

prikazy:
  build    prelozi kernel, loader a programy userspace
  image    build + vytvori target/os-<arch>-<profil>.img (GPT, ESP FAT32)
  run      image + spusti QEMU (x86_64: q35 + OVMF, aarch64: virt + edk2)
  test     testy knihoven na hostiteli, kontrola prekladu jadra pro ostatni
           architektury, pak jadro a userspace v QEMU bez okna
  check    jen kontrola prekladu jadra
  docs     HTML dokumentace z docs/ do target/docs (s kontrolou odkazu)

prepinace:
  --release        release profil
  --arch <arch>    architektura: x86_64 (vychozi), aarch64
  --gdb            (run) QEMU ceka na GDB na localhost:1234
  --headless       (run) bez okna, jen seriova linka v terminalu
  --accel <name>   (run) akcelerace QEMU: tcg, kvm, whpx, hvf
  --iommu <typ>    (run, test) IOMMU v QEMU: x86_64 intel (vychozi), amd,
                   none; aarch64 smmu (vychozi), none
  -- <args>        (run) dalsi argumenty pro QEMU

promenne prostredi:
  QEMU_X86_64, QEMU_AARCH64   cesta k QEMU (QEMU plati pro x86_64)
  X86_64_FW_CODE, _FW_VARS    firmware UEFI (take OVMF_CODE, OVMF_VARS)
  AARCH64_FW_CODE, _FW_VARS   firmware UEFI pro aarch64 (edk2, AAVMF)
";

pub struct Options {
    release: bool,
    gdb: bool,
    headless: bool,
    accel: Option<String>,
    /// Emulovana IOMMU (`Arch::iommus`): x86_64 "intel", "amd", "none";
    /// aarch64 "smmu", "none".
    iommu: String,
    qemu_args: Vec<OsString>,
    /// Cilova architektura.
    arch: &'static Arch,
    /// Testovaci beh: kernel s feature `qemu-test`, QEMU s testovacim
    /// koncem (x86_64 isa-debug-exit, aarch64 semihosting).
    test: bool,
}

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: chyba: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(command) = args.next() else {
        eprint!("{USAGE}");
        return Err("chybi prikaz".into());
    };

    let mut opts = Options {
        release: false,
        gdb: false,
        headless: false,
        accel: None,
        iommu: String::new(),
        qemu_args: Vec::new(),
        arch: &arch::X86_64,
        test: false,
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--release") => opts.release = true,
            Some("--iommu") => {
                let value = args.next().ok_or("--iommu ocekava typ IOMMU")?;
                opts.iommu = value.to_string_lossy().into_owned();
            }
            Some("--arch") => {
                let value = args.next().ok_or("--arch ocekava x86_64 nebo aarch64")?;
                opts.arch = arch::by_name(&value.to_string_lossy())
                    .ok_or("--arch ocekava x86_64 nebo aarch64")?;
            }
            Some("--gdb") => opts.gdb = true,
            Some("--headless") => opts.headless = true,
            Some("--accel") => {
                let value = args.next().ok_or("--accel ocekava hodnotu")?;
                opts.accel = Some(
                    value
                        .into_string()
                        .map_err(|_| "neplatna hodnota --accel")?,
                );
            }
            Some("--") => {
                opts.qemu_args.extend(args.by_ref());
            }
            _ => return Err(format!("neznamy prepinac {arg:?}\n\n{USAGE}").into()),
        }
    }
    if opts.iommu.is_empty() {
        opts.iommu = opts.arch.iommus[0].to_string();
    } else if !opts.arch.iommus.contains(&opts.iommu.as_str()) {
        return Err(format!(
            "--iommu pro {} ocekava jednu z hodnot {:?}",
            opts.arch.name, opts.arch.iommus
        )
        .into());
    }

    match command.to_str() {
        Some("build") => build(&opts).map(|_| ()),
        Some("image") => build(&opts).and_then(|a| image_for(&a, &opts)).map(|_| ()),
        Some("run") => {
            let artifacts = build(&opts)?;
            let image = image_for(&artifacts, &opts)?;
            qemu::run(&image, &artifacts, &opts)
        }
        Some("test") => {
            opts.test = true;
            opts.headless = true;
            host_tests()?;
            // Jadro se musi prelozit i pro ostatni architektury (rozhrani
            // architektury nesmi zustat jen pro jednu).
            for other in arch::ALL.iter().filter(|a| a.name != opts.arch.name) {
                check_kernel(other, &opts)?;
            }
            build_docs()?;
            let artifacts = build(&opts)?;
            let image = image_for(&artifacts, &opts)?;
            qemu::run(&image, &artifacts, &opts)
        }
        Some("check") => check_kernel(opts.arch, &opts),
        Some("docs") => build_docs().map(|_| ()),
        Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("neznamy prikaz {command:?}\n\n{USAGE}").into()),
    }
}

/// Testy sdilenych knihoven na hostiteli (format initfs, ABI, dokumentace
/// chyb a syscallu, casova pasma, protokol vstupu a rozlozeni klavesnice).
fn host_tests() -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(workspace_root())
        .args([
            "test",
            "--quiet",
            "--package",
            "abi",
            "--package",
            "initfs",
            "--package",
            "tz",
            "--package",
            "fs",
            "--package",
            "inputproto",
            "--package",
            "keymap",
            "--package",
            "i8042",
            "--package",
            "usbproto",
            "--package",
            "shlang",
            "--package",
            "term",
            "--features",
            "initfs/std,fs/std",
        ])
        .status()?;
    if !status.success() {
        return Err("testy knihoven na hostiteli selhaly".into());
    }
    println!("xtask: testy knihoven na hostiteli prosly");
    Ok(())
}

/// Overi, ze se jadro prelozi (a slinkuje) pro architekturu `arch`. Hlida,
/// ze rozhrani architektury (`kernel/src/arch/api`) jde implementovat pro
/// vsechny architektury a ze obecna cast jadra nepouziva nic specifickeho
/// jen pro jednu z nich.
fn check_kernel(arch: &Arch, opts: &Options) -> Result<()> {
    let target = arch.kernel_target;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(workspace_root())
        .args(["rustc", "--quiet", "--package", "kernel", "--bin", "kernel"])
        .args(["--target", target]);
    if opts.release {
        cmd.arg("--release");
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!(
            "kontrola prekladu jadra pro {} ({target}) selhala",
            arch.name
        )
        .into());
    }
    println!("xtask: jadro se prelozi pro {}", arch.name);
    Ok(())
}

/// Vygeneruje HTML dokumentaci a overi odkazy.
fn build_docs() -> Result<PathBuf> {
    let index = docs::build(&workspace_root(), &target_dir().join("docs"))?;
    println!("xtask: dokumentace {}", index.display());
    Ok(index)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn target_dir() -> PathBuf {
    env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
}

fn profile_dir(opts: &Options) -> &'static str {
    if opts.release { "release" } else { "debug" }
}

pub struct Artifacts {
    kernel: PathBuf,
    loader: PathBuf,
    initfs: PathBuf,
}

/// Programy v userspace, ktere se pribali do initfs (jmeno crate = jmeno
/// souboru v initfs).
const USER_PROGRAMS: &[&str] = &[
    "init",
    "echo",
    "devmgr",
    "virtio-blk",
    "virtio-input",
    "inputd",
    "ps2",
    "xhci",
    "usb-hub",
    "usb-hid",
    "fsd",
    "timed",
    "termd",
];

fn cargo_build(packages: &[&str], target: &str, opts: &Options, user: bool) -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(workspace_root())
        .args(["build", "--target", target]);
    for p in packages {
        cmd.args(["--package", p]);
    }
    if opts.release {
        cmd.arg("--release");
    }
    if opts.test && packages.contains(&"kernel") {
        cmd.args(["--features", "qemu-test"]);
    }
    if user {
        // Userspace ma jine priznaky nez jadro (bez code-model=kernel
        // z .cargo/config.toml). Vlastni adresar, aby se buildy jadra
        // a userspace navzajem neprekladaly znovu.
        // CARGO_ENCODED_RUSTFLAGS oddeluje priznaky znakem 0x1F.
        // Linker skript `user/user.ld` (adresy, zarovnani segmentu na
        // MAX_PAGE_SIZE) plati pro vsechny programy - jedno misto misto
        // build.rs v kazdem programu.
        let separator = char::from(0x1F).to_string();
        let script = format!(
            "-Clink-arg=-T{}",
            workspace_root().join("user").join("user.ld").display()
        );
        let flags = [
            "-Crelocation-model=static",
            "-Cforce-frame-pointers=yes",
            script.as_str(),
        ]
        .join(&separator);
        cmd.env("CARGO_ENCODED_RUSTFLAGS", flags)
            .arg("--target-dir")
            .arg(user_target_dir());
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("build {} ({target}) selhal", packages.join(", ")).into());
    }
    Ok(())
}

fn user_target_dir() -> PathBuf {
    target_dir().join("user")
}

fn build(opts: &Options) -> Result<Artifacts> {
    let a = opts.arch;
    cargo_build(&["kernel"], a.kernel_target, opts, false)?;
    cargo_build(&["loader"], a.loader_target, opts, false)?;
    cargo_build(USER_PROGRAMS, a.user_target, opts, true)?;
    let target = target_dir();
    let profile = profile_dir(opts);

    let user_bin = user_target_dir().join(a.user_target).join(profile);
    let mut files = Vec::new();
    for name in USER_PROGRAMS {
        let data = std::fs::read(user_bin.join(name))
            .map_err(|e| format!("nelze cist program {name}: {e}"))?;
        files.push((name.to_string(), data));
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    let initfs = target.join(format!("initfs-{}-{profile}.img", a.name));
    std::fs::write(&initfs, initfs::build(&refs))?;

    Ok(Artifacts {
        kernel: target.join(a.kernel_target).join(profile).join("kernel"),
        loader: target.join(a.loader_target).join(profile).join("loader.efi"),
        initfs,
    })
}

fn image_for(artifacts: &Artifacts, opts: &Options) -> Result<PathBuf> {
    let suffix = if opts.test { "-test" } else { "" };
    let path = target_dir().join(format!(
        "os-{}-{}{suffix}.img",
        opts.arch.name,
        profile_dir(opts)
    ));
    let files = [
        (opts.arch.boot_file, artifacts.loader.as_path()),
        ("os/kernel.elf", artifacts.kernel.as_path()),
        ("os/initfs.img", artifacts.initfs.as_path()),
    ];
    image::create_disk_image(&path, &files)?;
    println!("xtask: obraz disku {}", path.display());
    Ok(path)
}
