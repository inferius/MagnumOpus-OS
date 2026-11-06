//! Build a spousteni systemu: `cargo xtask <prikaz>`.
//!
//! Prikazy:
//! - `build`  - prelozi kernel a loader
//! - `image`  - build + bootovatelny obraz disku (GPT + ESP s FAT32)
//! - `run`    - image + spusteni v QEMU s OVMF
//!
//! Prepinace: `--release`, a pro `run` navic `--gdb`, `--headless`,
//! `--accel <tcg|kvm|whpx|hvf>` a cokoli za `--` se preda primo QEMU.

mod docs;
mod image;
mod symbolize;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

const KERNEL_TARGET: &str = "x86_64-unknown-none";
const LOADER_TARGET: &str = "x86_64-unknown-uefi";

const USAGE: &str = "\
pouziti: cargo xtask <prikaz> [prepinace]

prikazy:
  build    prelozi kernel a loader
  image    build + vytvori target/os-x86_64.img (GPT, ESP FAT32)
  run      image + spusti QEMU (x86_64, OVMF)
  test     testy knihoven na hostiteli, pak jadro a userspace v QEMU bez okna
  docs     HTML dokumentace z docs/ do target/docs (s kontrolou odkazu)

prepinace:
  --release        release profil
  --gdb            (run) QEMU ceka na GDB na localhost:1234
  --headless       (run) bez okna, jen seriova linka v terminalu
  --accel <name>   (run) akcelerace QEMU: tcg, kvm, whpx, hvf
  -- <args>        (run) dalsi argumenty pro QEMU

promenne prostredi:
  QEMU             cesta k qemu-system-x86_64
  OVMF_CODE        cesta k OVMF code (pflash, read-only)
  OVMF_VARS        cesta k sablone OVMF vars
";

struct Options {
    release: bool,
    gdb: bool,
    headless: bool,
    accel: Option<String>,
    qemu_args: Vec<OsString>,
    /// Testovaci beh: kernel s feature `qemu-test`, QEMU s isa-debug-exit.
    test: bool,
}

/// Navratovy kod QEMU pri uspechu testu: isa-debug-exit vraci (v << 1) | 1
/// a kernel zapisuje v = 0x10 (viz arch::qemu_exit).
const QEMU_TEST_SUCCESS: i32 = 33;
const QEMU_TEST_TIMEOUT: Duration = Duration::from_secs(120);

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
        qemu_args: Vec::new(),
        test: false,
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--release") => opts.release = true,
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

    match command.to_str() {
        Some("build") => build(&opts).map(|_| ()),
        Some("image") => build(&opts).and_then(|a| image_for(&a, &opts)).map(|_| ()),
        Some("run") => {
            let artifacts = build(&opts)?;
            let image = image_for(&artifacts, &opts)?;
            run_qemu(&image, &opts)
        }
        Some("test") => {
            opts.test = true;
            opts.headless = true;
            host_tests()?;
            build_docs()?;
            let artifacts = build(&opts)?;
            let image = image_for(&artifacts, &opts)?;
            run_qemu(&image, &opts)
        }
        Some("docs") => build_docs().map(|_| ()),
        Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("neznamy prikaz {command:?}\n\n{USAGE}").into()),
    }
}

/// Testy sdilenych knihoven na hostiteli (format initfs, ABI, dokumentace
/// chyb a syscallu).
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
            "--features",
            "initfs/std",
        ])
        .status()?;
    if !status.success() {
        return Err("testy knihoven na hostiteli selhaly".into());
    }
    println!("xtask: testy knihoven na hostiteli prosly");
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

struct Artifacts {
    kernel: PathBuf,
    loader: PathBuf,
    initfs: PathBuf,
}

/// Programy v userspace, ktere se pribali do initfs (jmeno crate = jmeno
/// souboru v initfs).
const USER_PROGRAMS: &[&str] = &["init", "echo"];

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
        // Userspace ma stejny target jako jadro, ale jine priznaky (bez
        // code-model=kernel z .cargo/config.toml). Vlastni adresar, aby se
        // buildy jadra a userspace navzajem neprekladaly znovu.
        // CARGO_ENCODED_RUSTFLAGS oddeluje priznaky znakem 0x1F.
        let separator = char::from(0x1F).to_string();
        let flags = ["-Crelocation-model=static", "-Cforce-frame-pointers=yes"].join(&separator);
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
    cargo_build(&["kernel"], KERNEL_TARGET, opts, false)?;
    cargo_build(&["loader"], LOADER_TARGET, opts, false)?;
    cargo_build(USER_PROGRAMS, KERNEL_TARGET, opts, true)?;
    let target = target_dir();
    let profile = profile_dir(opts);

    let user_bin = user_target_dir().join(KERNEL_TARGET).join(profile);
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
    let initfs = target.join(format!("initfs-{profile}.img"));
    std::fs::write(&initfs, initfs::build(&refs))?;

    Ok(Artifacts {
        kernel: target.join(KERNEL_TARGET).join(profile).join("kernel"),
        loader: target.join(LOADER_TARGET).join(profile).join("loader.efi"),
        initfs,
    })
}

fn image_for(artifacts: &Artifacts, opts: &Options) -> Result<PathBuf> {
    let suffix = if opts.test { "-test" } else { "" };
    let path = target_dir().join(format!("os-x86_64-{}{suffix}.img", profile_dir(opts)));
    let files = [
        ("EFI/BOOT/BOOTX64.EFI", artifacts.loader.as_path()),
        ("os/kernel.elf", artifacts.kernel.as_path()),
        ("os/initfs.img", artifacts.initfs.as_path()),
    ];
    image::create_disk_image(&path, &files)?;
    println!("xtask: obraz disku {}", path.display());
    Ok(path)
}

/// Najde spustitelny soubor QEMU: promenna QEMU, PATH, bezna instalacni mista.
fn find_qemu() -> Result<PathBuf> {
    if let Some(path) = env::var_os("QEMU") {
        return Ok(path.into());
    }
    let exe = if cfg!(windows) {
        "qemu-system-x86_64.exe"
    } else {
        "qemu-system-x86_64"
    };
    if let Some(paths) = env::var_os("PATH") {
        for dir in env::split_paths(&paths) {
            let candidate = dir.join(exe);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    let fallback = [
        r"C:\Program Files\qemu",
        "/opt/homebrew/bin",
        "/usr/local/bin",
    ];
    fallback
        .iter()
        .map(|dir| Path::new(dir).join(exe))
        .find(|p| p.is_file())
        .ok_or_else(|| "QEMU nenalezeno (nastav promennou QEMU nebo ho pridej do PATH)".into())
}

/// Najde par OVMF code + vars sablona. Pary musi byt ze stejne distribuce.
fn find_ovmf(qemu: &Path) -> Result<(PathBuf, PathBuf)> {
    if let (Some(code), Some(vars)) = (env::var_os("OVMF_CODE"), env::var_os("OVMF_VARS")) {
        return Ok((code.into(), vars.into()));
    }
    let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();
    // Firmware dodavany primo s QEMU (Windows instalator, Homebrew, ...).
    if let Some(bin_dir) = qemu.parent() {
        for share in [bin_dir.join("share"), bin_dir.join("../share/qemu")] {
            candidates.push((
                share.join("edk2-x86_64-code.fd"),
                share.join("edk2-i386-vars.fd"),
            ));
        }
    }
    for (code, vars) in [
        (
            "/usr/share/OVMF/OVMF_CODE_4M.fd",
            "/usr/share/OVMF/OVMF_VARS_4M.fd",
        ),
        (
            "/usr/share/OVMF/OVMF_CODE.fd",
            "/usr/share/OVMF/OVMF_VARS.fd",
        ),
        (
            "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
            "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
        ),
        (
            "/usr/share/edk2/ovmf/OVMF_CODE.fd",
            "/usr/share/edk2/ovmf/OVMF_VARS.fd",
        ),
        (
            "/usr/share/qemu/edk2-x86_64-code.fd",
            "/usr/share/qemu/edk2-i386-vars.fd",
        ),
    ] {
        candidates.push((code.into(), vars.into()));
    }
    candidates
        .into_iter()
        .find(|(code, vars)| code.is_file() && vars.is_file())
        .ok_or_else(|| "OVMF nenalezen (nastav OVMF_CODE a OVMF_VARS)".into())
}

fn default_accel() -> &'static str {
    if cfg!(target_os = "linux") && Path::new("/dev/kvm").exists() {
        "kvm"
    } else {
        "tcg"
    }
}

fn run_qemu(image: &Path, opts: &Options) -> Result<()> {
    let qemu = find_qemu()?;
    let (ovmf_code, ovmf_vars_template) = find_ovmf(&qemu)?;

    // OVMF do vars zapisuje (boot polozky), proto pracujeme s kopii.
    let ovmf_vars = target_dir().join("ovmf-x86_64-vars.fd");
    if !ovmf_vars.exists() {
        std::fs::copy(&ovmf_vars_template, &ovmf_vars)?;
    }

    let serial_log = target_dir().join(if opts.test {
        "serial-test.log"
    } else {
        "serial.log"
    });
    let accel = opts.accel.as_deref().unwrap_or(default_accel());
    let mut cmd = Command::new(&qemu);
    cmd.arg("-machine")
        .arg(format!("q35,accel={accel}"))
        .args(["-cpu", "max", "-smp", "4", "-m", "512M"])
        .arg("-drive")
        .arg(pflash_arg(&ovmf_code, true))
        .arg("-drive")
        .arg(pflash_arg(&ovmf_vars, false))
        .arg("-drive")
        .arg(format!(
            "if=none,id=osdisk,format=raw,file={}",
            image.display()
        ))
        .args(["-device", "virtio-blk-pci,drive=osdisk"])
        // Bez sitove karty, jinak OVMF zbytecne zkousi PXE boot.
        .args(["-nic", "none"])
        // Seriova linka do terminalu a zaroven do souboru (target/serial.log),
        // aby se dal vystup dohledat i po skonceni behu.
        .arg("-chardev")
        .arg(format!("stdio,id=serial0,logfile={}", serial_log.display()))
        .args(["-serial", "chardev:serial0"])
        .arg("-no-reboot");
    if opts.headless {
        cmd.args(["-display", "none"]);
    }
    if opts.gdb {
        cmd.args(["-s", "-S"]);
        let kernel = target_dir()
            .join(KERNEL_TARGET)
            .join(profile_dir(opts))
            .join("kernel");
        println!("xtask: QEMU ceka na GDB na localhost:1234, napr.:");
        println!(
            "  gdb {} -ex \"target remote localhost:1234\"",
            kernel.display()
        );
    }
    if opts.test {
        cmd.args(["-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"]);
    }
    cmd.args(&opts.qemu_args);

    println!("xtask: spoustim {}", qemu.display());
    let kernel = target_dir()
        .join(KERNEL_TARGET)
        .join(profile_dir(opts))
        .join("kernel");
    let result = if opts.test {
        run_qemu_test(cmd)
    } else {
        run_qemu_interactive(cmd)
    };
    // Pri panice nebo fatalni vyjimce prelozit adresy na jmena funkci.
    if let Err(e) = symbolize::report(&serial_log, &kernel) {
        eprintln!("xtask: preklad adres selhal: {e}");
    }
    println!("xtask: vystup seriove linky je v {}", serial_log.display());
    result
}

fn run_qemu_interactive(mut cmd: Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("QEMU skoncilo s {status}").into());
    }
    Ok(())
}

fn run_qemu_test(mut cmd: Command) -> Result<()> {
    let mut child = cmd.spawn()?;
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if start.elapsed() > QEMU_TEST_TIMEOUT {
            child.kill()?;
            child.wait()?;
            return Err(format!("testy nedobehly do {} s", QEMU_TEST_TIMEOUT.as_secs()).into());
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    match status.code() {
        Some(QEMU_TEST_SUCCESS) => {
            println!("xtask: testy prosly");
            Ok(())
        }
        code => Err(format!("testy selhaly (navratovy kod QEMU {code:?})").into()),
    }
}

fn pflash_arg(path: &Path, readonly: bool) -> String {
    let ro = if readonly { ",readonly=on" } else { "" };
    format!("if=pflash,format=raw{ro},file={}", path.display())
}
