//! `cargo xtask usb`: zive USB pro zkouseni na skutecnem pocitaci.
//!
//! - `cargo xtask usb` - obraz `target/usb-<arch>-<profil>.img` (GPT, ESP
//!   se zavadecem, jadrem a initfs, datovy oddil s nasim FS),
//! - `--list` - disky hostitele (ktere jsou vyjimatelne, ktery je
//!   systemovy),
//! - `--write <disk>` - obraz zapise na vyjimatelny disk po vyslovnem
//!   potvrzeni (`hostdisk.rs`: pravidla bezpecnosti),
//! - `--run` - start obrazu v QEMU jako flash disk USB (okno),
//! - `--test` - totez bez okna, ceka na start GUI s terminalem,
//! - `--size <MiB>` - velikost obrazu (vychozi [`DEFAULT_MIB`]),
//! - `--out <soubor>` - jina cesta obrazu.
//!
//! Navod pro uzivatele: `docs/cs/zive-usb.md`, `docs/en/live-usb.md`.

pub mod boot;
pub mod hostdisk;
pub mod install_test;
mod image;
pub mod platform;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::{Artifacts, Options, Result, target_dir};

/// Vychozi velikost obrazu (vejde se na kazdou flashku od 2 GB; zbytek
/// flashky zustane nevyuzity, dokud system neumi oddil zvetsit).
pub const DEFAULT_MIB: u64 = 1024;
/// Velikost obrazu pro test v QEMU.
pub const TEST_MIB: u64 = 320;

/// Prepinace prikazu `usb`.
#[derive(Debug, Default)]
pub struct UsbArgs {
    pub list: bool,
    pub write: Option<String>,
    pub run: bool,
    pub test: bool,
    pub size_mib: Option<u64>,
    pub out: Option<PathBuf>,
}

/// Oddeli prepinace prikazu `usb` od spolecnych (`--release`, `--arch`).
pub fn split_args(args: Vec<OsString>) -> Result<(UsbArgs, Vec<OsString>)> {
    let mut u = UsbArgs::default();
    let mut rest = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--list") => u.list = true,
            Some("--run") => u.run = true,
            Some("--test") => u.test = true,
            Some("--write") => {
                let v = it.next().ok_or("--write ocekava disk (cislo na Windows, /dev/sdX na Linuxu)")?;
                u.write = Some(v.to_string_lossy().into_owned());
            }
            Some("--size") => {
                let v = it.next().ok_or("--size ocekava velikost v MiB")?;
                let n: u64 = v.to_string_lossy().parse().map_err(|_| "--size ocekava cislo (MiB)")?;
                u.size_mib = Some(n);
            }
            Some("--out") => {
                let v = it.next().ok_or("--out ocekava cestu k souboru")?;
                u.out = Some(PathBuf::from(v));
            }
            _ => rest.push(a),
        }
    }
    Ok((u, rest))
}

/// Soubory ESP: zavadec, jadro, initfs a kratky navod pro uzivatele
/// (videt ve Windows po zasunuti flashky).
fn esp_files<'a>(opts: &Options, a: &'a Artifacts, readme: &'a Path, install: &'a Path) -> Vec<(&'static str, &'a Path)> {
    vec![
        (opts.arch.boot_file, a.loader.as_path()),
        ("os/kernel.elf", a.kernel.as_path()),
        ("os/initfs.img", a.initfs.as_path()),
        ("os/instfs.img", install),
        ("CTI-MNE.txt", readme),
    ]
}

/// Initfs pro instalaci na disk (`installd` ho kopiruje): samostatne
/// sestaveni bez vyvojoveho profilu (`accounts-dev`) a bez souboru
/// odpovedi - nainstalovany system nesmi mit ucet bez hesla
/// s automatickym prihlasenim ani kod rezimu vyvojare (feature `dev-mode`
/// zapina xtask jen s vyvojovym profilem, `crate::devmode`; sestaveni
/// vydani ji samo zkontroluje). Programy se tak prekladaji dvakrat
/// (cargo si drzi obe varianty, dalsi beh jen prekopiruje).
fn install_initfs(opts: &mut Options, out: &Path) -> Result<()> {
    let live = opts.extra_initfs.clone();
    opts.extra_initfs.retain(|(n, _)| n != DEV_PROFILE.0 && n != INSTALL_ANSWERS);
    let a = crate::build(opts);
    opts.extra_initfs = live;
    std::fs::copy(&a?.initfs, out)?;
    Ok(())
}

const README: &str = "MagnumOpus - zive USB\r\n\
\r\n\
Start: v nabidce zavadeni firmwaru (F12, F11, F8 nebo Esc podle vyrobce)\r\n\
vyberte tuto flashku (UEFI). Secure Boot musi byt vypnuty.\r\n\
\r\n\
Navod: docs/cs/zive-usb.md v repozitari (anglicky docs/en/live-usb.md).\r\n\
\r\n\
MagnumOpus - live USB. Boot it from the firmware boot menu (UEFI,\r\n\
Secure Boot off). Guide: docs/en/live-usb.md in the repository.\r\n";

/// Vytvori obraz USB; vraci cestu.
pub fn create_image(opts: &mut Options, artifacts: &Artifacts, path: &Path, size_mib: u64) -> Result<PathBuf> {
    let readme = target_dir().join("usb-readme.txt");
    std::fs::write(&readme, README)?;
    let install = target_dir().join(format!("initfs-install-{}.img", std::process::id()));
    install_initfs(opts, &install)?;
    let files = esp_files(opts, artifacts, &readme, &install);
    // ESP podle skutecne velikosti souboru; maly obraz (test) se zvetsi.
    let need = image::min_mib(image::esp_mib(&files)?);
    if size_mib < need {
        println!("xtask: obraz USB zvetsen z {size_mib} na {need} MiB (soubory na ESP)");
    }
    let layout = image::create(path, size_mib.max(need), &files);
    let _ = std::fs::remove_file(&install);
    let layout = layout?;
    println!(
        "xtask: obraz USB {} ({} MiB, ESP {} MiB, datovy oddil {} MiB od {} MiB)",
        path.display(),
        layout.size >> 20,
        layout.esp_mib,
        layout.data.1 >> 20,
        layout.data.0 >> 20
    );
    Ok(path.to_path_buf())
}

/// Zive USB je vyvojovy profil (rozhodnuti uzivatele 2026-11-13): obraz
/// ma vyvojove automaticke prihlaseni (`accounts-dev` v initfs) v ladicim
/// i release profilu; disk pro VMware (`xtask vmware`) je tentyz obraz.
/// Obraz pro uzivatele (`xtask image`) ne.
pub const DEV_PROFILE: (&str, &[u8]) = ("accounts-dev", b"1");
/// Soubor odpovedi pro test instalace (init podle nej nainstaluje bez
/// dotazu); do instalovaneho initfs nepatri.
pub const INSTALL_ANSWERS: &str = "install-answers";

/// Varovani pri tvorbe obrazu.
pub const WARNING: &str = "xtask: POZOR: zive USB = vyvojovy profil: ucet `vyvojar` BEZ HESLA s automatickym prihlasenim; \
datovy oddil na flashce NENI sifrovany (kdo ma flashku, precte ho). Na zkouseni HW a VMware, ne pro osobni data.";

pub fn with_dev_profile(opts: &mut Options) {
    if !opts.extra_initfs.iter().any(|(n, _)| n == DEV_PROFILE.0) {
        opts.extra_initfs.push((String::from(DEV_PROFILE.0), DEV_PROFILE.1.to_vec()));
    }
}

fn without_dev_profile(opts: &mut Options) {
    opts.extra_initfs.retain(|(n, _)| n != DEV_PROFILE.0);
}

/// Prikaz `cargo xtask usb`.
pub fn command(opts: &mut Options, u: &UsbArgs) -> Result<()> {
    if u.list {
        print_disks()?;
        if u.write.is_none() && !u.run && !u.test {
            return Ok(());
        }
    }
    let size = u.size_mib.unwrap_or(if u.test { TEST_MIB } else { DEFAULT_MIB });
    // Disk pro zapis zkontrolovat pred dlouhym buildem.
    if let Some(id) = &u.write {
        let disks = platform::list()?;
        hostdisk::check(&disks, id, size << 20).map_err(|e| format!("zapis na disk {id}: {e}"))?;
    }
    with_dev_profile(opts);
    println!("{WARNING}");
    let artifacts = crate::build(opts)?;
    let profile = if opts.release { "release" } else { "debug" };
    let path = u
        .out
        .clone()
        .unwrap_or_else(|| target_dir().join(format!("usb-{}-{profile}.img", opts.arch.name)));
    create_image(opts, &artifacts, &path, size)?;
    if let Some(id) = &u.write {
        write_to_disk(&path, id)?;
    }
    if u.test {
        boot::test(opts.arch, &path)?;
    }
    if u.run {
        boot::run(opts.arch, &path)?;
    }
    Ok(())
}

/// Obraz USB pro `cargo xtask test`: maly, smaze se po testu.
pub fn test_run(opts: &mut Options) -> Result<()> {
    with_dev_profile(opts);
    let artifacts = crate::build(opts)?;
    let path = boot::test_image_path(opts.arch);
    create_image(opts, &artifacts, &path, TEST_MIB)?;
    // Start do plochy s terminalem (s kopii logu na FAT), pak na novem
    // obrazu rezim "vytahni logy".
    let r = boot::test(opts.arch, &path).and_then(|()| {
        create_image(opts, &artifacts, &path, TEST_MIB)?;
        boot::test_rescue(opts.arch, &path)
    });
    let _ = std::fs::remove_file(&path);
    without_dev_profile(opts);
    r?;
    // Obraz bez vyvojoveho profilu (jako obraz pro uzivatele): zadne
    // automaticke prihlaseni, start konci na prihlasovaci obrazovce.
    let artifacts = crate::build(opts)?;
    create_image(opts, &artifacts, &path, TEST_MIB)?;
    let r = boot::test_user_image(opts.arch, &path);
    let _ = std::fs::remove_file(&path);
    r
}

/// Disk "nainstalovaneho systemu" pro test vytazeni logu: cely disk s nasim
/// FS a logy v /system/log a /system/pady.
pub fn installed_disk(path: &Path) -> Result<()> {
    use fs::{Algo, FileDevice, FormatOptions, Fs, Kind};
    let file = std::fs::File::create(path)?;
    file.set_len(64 << 20)?;
    drop(file);
    let e = |what: &str, err: fs::Error| -> Box<dyn std::error::Error> { format!("disk instalace: {what}: {err:?}").into() };
    let dev = FileDevice::open(path)?;
    let opts = FormatOptions {
        label: "instalace".into(),
        algo: Algo::Xxh3,
        ag_blocks: None,
        uuid: Some(image::random_uuid()),
        time: fs::Timestamp::UNIX_EPOCH,
    };
    let dev = Fs::format(dev, &opts).map_err(|x| e("format", x))?;
    let mut f = Fs::mount(dev).map_err(|x| e("mount", x))?;
    for d in ["/system", "/system/log", "/system/pady"] {
        f.create(d, Kind::Dir).map_err(|x| e(d, x))?;
    }
    let files: [(&str, &[u8]); 2] = [
        ("/system/log/boot-000007.log", b"[    0.000000] cpu0 I kernel: os kernel (nainstalovany system)\n"),
        ("/system/pady/000006-jadro-MO-1234-5678.crash", b"MagnumOpus: zaznam padu jadra\nduvod:   KERNEL_PANIC\n"),
    ];
    for (p, data) in files {
        f.create(p, Kind::File).map_err(|x| e(p, x))?;
        f.write(p, 0, data).map_err(|x| e(p, x))?;
    }
    f.commit().map_err(|x| e("commit", x))?;
    Ok(())
}

fn print_disks() -> Result<()> {
    let disks = platform::list()?;
    println!("disky tohoto pocitace (zapsat jde jen na vyjimatelny, ne systemovy):");
    for d in &disks {
        println!("  {}", hostdisk::describe(d));
    }
    Ok(())
}

/// Zapis obrazu na disk `id` po potvrzeni uzivatelem.
fn write_to_disk(image: &Path, id: &str) -> Result<()> {
    let size = std::fs::metadata(image)?.len();
    // Seznam znovu: mezi buildem a zapisem mohl uzivatel flashku vymenit.
    let disks = platform::list()?;
    let d = hostdisk::check(&disks, id, size).map_err(|e| format!("zapis na disk {id}: {e}"))?;
    let expected = hostdisk::confirmation(d);
    println!();
    println!("POZOR: cely disk se prepise, vsechna data na nem se ztrati:");
    println!("  {}", hostdisk::describe(d));
    println!("  cesta {}", d.path);
    println!("Pro potvrzeni napis presne: {expected}");
    print!("> ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    if line.trim() != expected {
        return Err("zapis zrusen (potvrzeni nesouhlasi)".into());
    }
    let mut target = platform::open_for_write(d)?;
    let mut last = 0;
    hostdisk::write_image(image, &mut target, d.size, |done, total| {
        let pct = done * 100 / total;
        if pct >= last + 5 || done == total {
            last = pct;
            print!("\rxtask: zapisuji {pct:>3} %");
            let _ = std::io::stdout().flush();
        }
    })?;
    target.sync_all()?;
    drop(target);
    println!();
    platform::rescan(d);
    println!("xtask: obraz zapsan na {} ({}); flashku bezpecne odeberte", d.path, d.model);
    Ok(())
}
