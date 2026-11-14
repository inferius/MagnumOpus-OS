//! Build a spousteni systemu: `cargo xtask <prikaz>`.
//!
//! Prikazy:
//! - `build`  - prelozi kernel, loader a programy userspace
//! - `std`    - std pro aplikace (zdroje s lepidlem, sysroot), `stdsys.rs`
//! - `image`  - build + bootovatelny obraz disku (GPT + ESP s FAT32)
//! - `run`    - image + spusteni v QEMU s UEFI firmwarem
//! - `test`   - testy na hostiteli, pak jadro a userspace v QEMU
//! - `usb`    - zive USB (obraz, zapis na flashku, start v QEMU), `usb/`
//! - `check`  - jen kontrola prekladu jadra
//! - `new-driver` - kostra noveho ovladace (viz `drivers.rs`)
//!
//! Prepinace: `--release`, `--arch <x86_64|aarch64>` (vsechny prikazy), pro
//! `run` navic `--gdb`, `--headless`, `--accel <tcg|kvm|whpx|hvf>` a cokoli
//! za `--` se preda primo QEMU.

mod arch;
mod audio;
mod boottest;
mod crashtest;
mod datadisk;
mod drivers;
mod docs;
mod docs_books;
mod docs_check;
mod highlight;
mod image;
mod qemu;
mod nethelper;
mod dnshelper;
mod ntshelper;
mod revhelper;
mod stdsys;
mod webhelper;
mod powertest;
mod qmp;
mod symbolize;
mod testdisks;
mod usb;
mod vmware;
mod venus;

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use arch::Arch;
use driverkit::manifest::Manifest;

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

const USAGE: &str = "\
pouziti: cargo xtask <prikaz> [prepinace]

prikazy:
  build    prelozi kernel, loader a programy userspace (i aplikace se std)
  std      jen std pro aplikace: zdroje s lepidlem a sysroot pro --arch
           (docs/cs/rust-std.md)
  std-patch  upravy v target/std-src/<nightly>/library ulozi zpet do
           third_party/rust-std/std.patch
  image    build + vytvori target/os-<arch>-<profil>.img (GPT, ESP FAT32)
  run      image + spusti QEMU (x86_64: q35 + OVMF, aarch64: virt + edk2)
  test     testy knihoven na hostiteli, kontrola prekladu jadra pro ostatni
           architektury, pak jadro a userspace v QEMU bez okna
  check    jen kontrola prekladu jadra
  symbolize <soubor>  adresy v zaznamu padu, balicku nebo logu na jmena
           funkci (jadro a programy z target/; stejne --arch a --release
           jako build, ze ktereho system bezel)
  usb      zive USB: target/usb-<arch>-<profil>.img (GPT, ESP, datovy
           oddil); --list disky hostitele, --write <disk> zapis na
           flashku (s potvrzenim), --run start v QEMU jako flash disk,
           --test totez bez okna, --size <MiB> (vychozi 1024), --out <soubor>
  vmware   obraz ziveho USB jako disk VMware: sestavi, prevede na VMDK,
           zazalohuje a nahradi <slozka>/magnumopus.vmdk; --dir <slozka>
           nebo MO_VMWARE_DIR; bezici VM odmitne
  docs     HTML dokumentace z docs/ do target/docs (s kontrolou odkazu)
  new-driver <jmeno> --pci <vendor:device>
           kostra ovladace na driverkitu: kod, manifest, model a test
           v simulaci, stranka dokumentace cs+en (docs/cs/ovladace.md)
  venus-gen  serializace Venusu z libs/venus/xml do libs/venus/src/vk
  venus-check  overi, ze libs/venus/src/vk odpovida vstupum

prepinace:
  --release        release profil
  --arch <arch>    architektura: x86_64 (vychozi), aarch64
  --gdb            (run) QEMU ceka na GDB na localhost:1234
  --headless       (run) bez okna, jen seriova linka v terminalu
  --accel <name>   (run) akcelerace QEMU: tcg, kvm, whpx, hvf
  --res <SxV>      (run, x86_64) vychozi rozliseni, napr. 1920x1080
  --iommu <typ>    (run, test) IOMMU v QEMU: x86_64 intel (vychozi), amd,
                   none; aarch64 smmu (vychozi), none
  --data-disk <r>  (run, test) radic datoveho disku: virtio (vychozi), nvme,
                   nvme4k (sektor 4 KiB), ahci, usb (flash disk na xHCI);
                   na NVMe, AHCI a USB ma disk GPT s nasim oddilem
  --qemu-wsl       (run, test) spustit QEMU ve WSL (QEMU_WSL_DISTRO, vychozi
                   Ubuntu; QEMU_WSL_PREFIX, vychozi ~/venus/prefix) - Venus
  --gpu <druh>     (run, test) grafika virtio-gpu: 2d (vychozi pro test a run
                   --headless), gl (virgl 3D)
                   nebo venus (Vulkan; QEMU s virglrenderer a render serverem);
                   gl a venus bez okna pouziji displej egl-headless;
                   none = bez virtio-gpu (GUI nad framebufferem z firmwaru,
                   ovladac fbdisplay; vychozi pro run s oknem)
  --only <sady>    (test) jen tyto sady testu userspace (jmena oddelena
                   carkou, napr. sit,firewall; jmena jako v logu);
                   --only release = jen start v release profilu (test
                   po hlavnim behu, hlida chyby, ktere debug neukaze)
  --repeat <n>     (test) vybrane sady n krat za sebou (nestabilni testy)
  --update-shots   (test) jen prepise referencni snimky vzhledu libs/ui
                   (tests/shots/*.png) a skonci; nikdy automaticky
  --audio <druh>   (run, test) zvuk QEMU (radic intel-hda, kodek hda-duplex):
                   test wav (vychozi; soubor se po testu zkontroluje), none;
                   run none (vychozi), dsound, pa, alsa, sdl, coreaudio
  --driver <jmeno> (test) jen jeden ovladac z manifestu: jeho testy na
                   hostiteli (simulace) a v QEMU sada ovladace z manifestu
                   jen pro nej; bez kontroly jader ostatnich architektur
                   a dokumentace (rychla smycka vyvoje ovladace)
  --net-online     (test) i testy proti skutecne siti hostitele (resolver
                   hostitele pres slirp); bez nej jen lokalni servery
  --no-power       (test) bez zvlastnich behu testu napajeni (vypnuti
                   tlacitkem pres ACPI, restart); s --only se vynechaji take,
                   --only napajeni = jen tyto behy
  -- <args>        (run) dalsi argumenty pro QEMU

promenne prostredi:
  QEMU_X86_64, QEMU_AARCH64   cesta k QEMU (QEMU plati pro x86_64)
  X86_64_FW_CODE, _FW_VARS    firmware UEFI (take OVMF_CODE, OVMF_VARS)
  AARCH64_FW_CODE, _FW_VARS   firmware UEFI pro aarch64 (edk2, AAVMF)
  OS_KEEP_DATA=1              datovy disk po behu nemazat (rozbor stavu FS)
";

pub struct Options {
    release: bool,
    gdb: bool,
    headless: bool,
    accel: Option<String>,
    /// Emulovana IOMMU (`Arch::iommus`): x86_64 "intel", "amd", "none";
    /// aarch64 "smmu", "none".
    iommu: String,
    /// `--res SxV`: vychozi rozliseni grafiky v QEMU (x86_64).
    res: Option<(u32, u32)>,
    /// Radic datoveho disku: "virtio", "nvme", "nvme4k" (sektor 4 KiB), "ahci",
    /// "usb" (flash disk usb-storage na radici xHCI).
    data_disk: String,
    /// Grafika virtio-gpu: "2d" (vychozi), "gl" (virgl 3D), "venus" (Vulkan
    /// pres virtio-gpu); "gl" a "venus" potrebuji displej s OpenGL (egl);
    /// "none" = bez virtio-gpu (displej `fbdisplay` nad framebufferem GOP).
    gpu: String,
    /// QEMU ve WSL (Linux): `--qemu-wsl` (vychozi distribuce Ubuntu, QEMU
    /// z `~/venus/prefix`, kde je preklad s Venusem). Cesty se prevedou na
    /// `/mnt/<disk>/...`.
    qemu_wsl: bool,
    qemu_args: Vec<OsString>,
    /// Cilova architektura.
    arch: &'static Arch,
    /// Testovaci beh: kernel s feature `qemu-test`, QEMU s testovacim
    /// koncem (x86_64 isa-debug-exit, aarch64 semihosting).
    test: bool,
    /// Dalsi soubory do initfs (konfigurace testu site).
    extra_initfs: Vec<(String, Vec<u8>)>,
    /// `--only` (sady testu userspace) a `--repeat` (kolikrat).
    test_only: String,
    test_repeat: u32,
    /// `--update-shots`: prepsat referencni snimky vzhledu `libs/ui`.
    update_shots: bool,
    /// `--net-online`: i testy proti skutecne siti hostitele.
    net_online: bool,
    /// Backend zvuku QEMU (`--audio`); prazdne = test wav, beh none.
    audio: String,
    /// `--driver`: jen tento ovladac z manifestu (testy).
    test_driver: Option<String>,
    /// QEMU ma vyukove zarizeni `edu` (ukazkovy ovladac); zjisti se pred
    /// buildem (`-device help`).
    edu: bool,
    /// Presmerovani portu z hostitele do hosta (`hostfwd` user-mode site).
    hostfwd: Option<String>,
    /// Pomocne servery testu site (port `hostfwd` se uvolni tesne pred
    /// startem QEMU).
    net_helper: std::cell::RefCell<Option<nethelper::Helper>>,
    /// Zvlastni beh testu napajeni (`powertest`): "button" nebo "reboot".
    /// QEMU ma skoncit samo (vypnuti ACPI) s kodem 0.
    power_test: Option<&'static str>,
    /// Datovy disk sdileny vic behy QEMU (`boottest`); `None` = novy disk
    /// pro kazdy beh.
    data_image: Option<PathBuf>,
    /// Zvlastni beh testu zaznamu padu (`crashtest`): QEMU bez
    /// `-no-reboot` (jadro po panice restartuje).
    crash_test: bool,
}

fn main() -> ExitCode {
    console_utf8();
    let result = run_cli();
    remove_run_files();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: chyba: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Docasne soubory tohoto behu v `target/` (jmeno s `<pid>`, aby si
/// soubezne behy neprepisovaly initfs a obraz, ktery jiny beh prave cte
/// nebo ma otevreny QEMU). Smazou se na konci `main`.
static RUN_FILES: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Cesta `target/<stem>-<pid>.<ext>` tohoto behu (smaze se na konci).
pub(crate) fn run_file(stem: &str, ext: &str) -> PathBuf {
    let p = target_dir().join(format!("{stem}-{}.{ext}", std::process::id()));
    let mut files = RUN_FILES.lock().unwrap_or_else(|e| e.into_inner());
    if !files.contains(&p) {
        files.push(p.clone());
    }
    p
}

/// Vylucny zamek `target/<name>.lock` pro sdileny vystup (web dokumentace,
/// sysroot std), ktery soubezne behy jinak mazaly navzajem pod rukama
/// (os error 145). Uvolni se zavrenim souboru, i pri padu procesu.
pub(crate) fn lock_shared(name: &str) -> Result<std::fs::File> {
    let dir = target_dir();
    std::fs::create_dir_all(&dir)?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(format!("{name}.lock")))?;
    f.lock()?;
    Ok(f)
}

fn remove_run_files() {
    let files = std::mem::take(&mut *RUN_FILES.lock().unwrap_or_else(|e| e.into_inner()));
    for f in files {
        let _ = std::fs::remove_file(f);
    }
}

fn run_cli() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let Some(command) = args.next() else {
        eprint!("{USAGE}");
        return Err("chybi prikaz".into());
    };
    if command == "new-driver" {
        let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
        let d = drivers::parse_new_driver(&rest)?;
        return drivers::new_driver(&workspace_root(), &d);
    }
    // Prikaz `usb` ma vlastni prepinace; zbytek (--release, --arch) je
    // spolecny.
    let rest: Vec<OsString> = args.collect();
    // `symbolize <soubor>`: prvni argument je soubor.
    let mut rest = rest;
    let symbolize_file = if command == "symbolize" && !rest.is_empty() { Some(PathBuf::from(rest.remove(0))) } else { None };
    let (usb_args, rest) = if command == "usb" {
        let (u, r) = usb::split_args(rest)?;
        (Some(u), r)
    } else {
        (None, rest)
    };
    let mut args = rest.into_iter();

    let mut opts = Options {
        release: false,
        gdb: false,
        headless: false,
        accel: None,
        iommu: String::new(),
        res: None,
        data_disk: String::from("virtio"),
        gpu: String::from("2d"),
        qemu_wsl: false,
        qemu_args: Vec::new(),
        arch: &arch::X86_64,
        test: false,
        extra_initfs: Vec::new(),
        test_only: String::new(),
        test_repeat: 1,
        update_shots: false,
        net_online: false,
        audio: String::new(),
        test_driver: None,
        edu: false,
        hostfwd: None,
        net_helper: std::cell::RefCell::new(None),
        power_test: None,
        data_image: None,
        crash_test: false,
    };
    let mut gpu_set = false;
    let mut vmware_dir: Option<std::path::PathBuf> = None;
    let mut no_power = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--release") => opts.release = true,
            Some("--res") => {
                let value = args.next().ok_or("--res ocekava SxV, napr. 1920x1080")?;
                let value = value.to_string_lossy().into_owned();
                let parsed = value
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
                    .filter(|&(w, h)| (320..=7680).contains(&w) && (200..=4320).contains(&h));
                opts.res = Some(parsed.ok_or("--res ocekava SxV, napr. 1920x1080")?);
            }
            Some("--iommu") => {
                let value = args.next().ok_or("--iommu ocekava typ IOMMU")?;
                opts.iommu = value.to_string_lossy().into_owned();
            }
            Some("--data-disk") => {
                let value = args.next().ok_or("--data-disk ocekava radic")?;
                let value = value.to_string_lossy().into_owned();
                if !["virtio", "nvme", "nvme4k", "ahci", "usb"].contains(&value.as_str()) {
                    return Err("--data-disk ocekava virtio, nvme, nvme4k, ahci nebo usb".into());
                }
                opts.data_disk = value;
            }
            Some("--qemu-wsl") => opts.qemu_wsl = true,
            Some("--gpu") => {
                gpu_set = true;
                let value = args.next().ok_or("--gpu ocekava 2d, gl, venus nebo none")?;
                opts.gpu = value.to_string_lossy().into_owned();
                if !matches!(opts.gpu.as_str(), "2d" | "gl" | "venus" | "none") {
                    return Err("--gpu ocekava 2d, gl, venus nebo none".into());
                }
            }
            Some("--arch") => {
                let value = args.next().ok_or("--arch ocekava x86_64 nebo aarch64")?;
                opts.arch = arch::by_name(&value.to_string_lossy())
                    .ok_or("--arch ocekava x86_64 nebo aarch64")?;
            }
            Some("--only") => {
                let value = args.next().ok_or("--only ocekava jmena sad oddelena carkou")?;
                opts.test_only = value.to_string_lossy().into_owned();
            }
            Some("--repeat") => {
                let value = args.next().ok_or("--repeat ocekava pocet")?;
                opts.test_repeat = value.to_string_lossy().parse().map_err(|_| "--repeat ocekava cislo")?;
            }
            Some("--net-online") => opts.net_online = true,
            Some("--update-shots") => opts.update_shots = true,
            Some("--dir") => {
                let value = args.next().ok_or("--dir ocekava slozku")?;
                vmware_dir = Some(std::path::PathBuf::from(value));
            }
            Some("--audio") => {
                let value = args.next().ok_or("--audio ocekava backend zvuku QEMU")?;
                opts.audio = value.to_string_lossy().into_owned();
            }
            Some("--no-power") => no_power = true,
            Some("--driver") => {
                let value = args.next().ok_or("--driver ocekava jmeno ovladace")?;
                opts.test_driver = Some(value.to_string_lossy().into_owned());
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
        Some("std-patch") => stdsys::save_patch().map(|p| println!("xtask: patch std ulozen do {}", p.display())),
        Some("std") => stdsys::build_sysroot(opts.arch).map(|p| println!("xtask: sysroot std {}", p.display())),
        Some("image") => build(&opts).and_then(|a| image_for(&a, &opts)).map(|_| ()),
        Some("run") => {
            // Okno QEMU ukazuje prvni grafickou kartu (VGA, framebuffer
            // z firmwaru) a na ni jsou navazane klavesnice a mys virtio.
            // S virtio-gpu by kompozitor kreslil na druhou konzoli QEMU,
            // kterou okno neukazuje (obraz by "zamrzl" na startu). Bez
            // `--gpu` proto beh s oknem GUI kresli pres fbdisplay na VGA.
            if !gpu_set && !opts.headless {
                opts.gpu = String::from("none");
                println!("xtask: grafika pres framebuffer firmwaru (virtio-gpu: --gpu 2d, pak v okne QEMU View -> virtio-gpu-pci)");
            }
            // Ucty: rychly profil Argon2id pro QEMU a vyvojove automaticke
            // prihlaseni (jen `run`; `image` pro uzivatele je nema).
            opts.extra_initfs.push((String::from("accounts-qemu"), b"1".to_vec()));
            opts.extra_initfs.push((String::from("accounts-dev"), b"1".to_vec()));
            opts.edu = edu_available(&opts);
            let artifacts = build(&opts)?;
            let image = image_for(&artifacts, &opts)?;
            qemu::run(&image, &artifacts, &opts)
        }
        Some("test") if opts.update_shots => update_shots(),
        Some("test") => {
            opts.test = true;
            opts.headless = true;
            let found = drivers::discover(&workspace_root())?;
            if let Some(d) = &opts.test_driver
                && !found.iter().any(|f| &f.manifest.name == d)
            {
                return Err(format!("--driver {d}: user/{d}/{d}.driver.toml neexistuje").into());
            }
            if opts.test_driver.is_none() {
                venus::run(true)?;
            }
            host_tests(&found, opts.test_driver.as_deref())?;
            if opts.test_driver.is_none() {
                // Jadro se musi prelozit i pro ostatni architektury (rozhrani
                // architektury nesmi zustat jen pro jednu).
                for other in arch::ALL.iter().filter(|a| a.name != opts.arch.name) {
                    check_kernel(other, &opts)?;
                }
                build_docs()?;
            } else if opts.test_only.is_empty() {
                opts.test_only = String::from("ovladace z manifestu");
            }
            opts.edu = edu_available(&opts);
            let expect: Vec<&str> = if opts.edu { vec!["edu"] } else { Vec::new() };
            opts.extra_initfs.push((
                String::from("driver-test"),
                drivers::test_plan(&expect, opts.test_driver.as_deref()).into_bytes(),
            ));
            // Pomocne servery pro testy site bezi, dokud bezi xtask.
            let helper = nethelper::start()?;
            let web = webhelper::start()?;
            let config = format!("{}{}", helper.config, web.config);
            opts.extra_initfs.push((String::from("net-test"), config.into_bytes()));
            opts.extra_initfs.push((String::from("accounts-qemu"), b"1".to_vec()));
            if opts.net_online {
                opts.extra_initfs.push((String::from("net-online"), b"1".to_vec()));
            }
            // `zive-usb` jde kombinovat se sadami (`--only zive-usb,ucty`):
            // do filtru sad v QEMU nepatri.
            let only: Vec<String> = opts.test_only.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let usb_selected = only.iter().any(|s| s == "zive-usb");
            let install_selected = only.iter().any(|s| s == "instalace");
            let install_gui_selected = only.iter().any(|s| s == "instalace-gui");
            let suites: Vec<String> =
                only.iter().filter(|s| !["zive-usb", "instalace", "instalace-gui"].contains(&s.as_str())).cloned().collect();
            if !opts.test_only.is_empty() || opts.test_repeat > 1 {
                let filter = format!("suites = {}
repeat = {}
", suites.join(","), opts.test_repeat);
                opts.extra_initfs.push((String::from("test-filter"), filter.into_bytes()));
            }
            opts.extra_initfs
                .push((String::from("net-test-ca.pem"), web.ca_pem.into_bytes()));
            opts.extra_initfs
                .push((String::from("net-test-dns-anchor"), web.dns_anchor.into_bytes()));
            opts.hostfwd = Some(helper.hostfwd.clone());
            *opts.net_helper.borrow_mut() = Some(helper);
            // `--only napajeni`: jen zvlastni behy testu napajeni,
            // `--only zive-usb`: jen start z flash disku USB (`usb` je sada
            // testu userspace).
            let power_only = opts.test_only == "napajeni";
            let usb_only = (usb_selected || install_selected || install_gui_selected) && suites.is_empty();
            let release_only = opts.test_only == "release";
            let reboot_only = opts.test_only == "opakovany-start";
            if !power_only && !usb_only && !release_only && !reboot_only && opts.test_only != "zaznam-padu" {
                let artifacts = build(&opts)?;
                let image = image_for(&artifacts, &opts)?;
                qemu::run(&image, &artifacts, &opts)?;
            }
            // Vypnuti a restart pres ACPI: vlastni kratke behy QEMU (konec
            // behu = vypnuti, nejde to spojit s hlavnim behem testu).
            if !no_power && (opts.test_only.is_empty() || power_only) {
                powertest::run_all(&mut opts)?;
            }
            // Start v release profilu (jine optimalizace a casovani nez
            // debug): jadro, init, sluzby a ACPI az po restart. Hlida chyby,
            // ktere se v debug neprojevi (`--only release` jen tento beh).
            if !opts.release && (opts.test_only.is_empty() || release_only) {
                powertest::run_release(&mut opts)?;
            }
            // Vic startu po sobe na jednom datovem disku (`--only
            // opakovany-start` jen tento beh): chyby "prvni start vs. dalsi".
            if opts.test_only.is_empty() || reboot_only {
                boottest::run(&mut opts)?;
            }
            // Zaznam padu jadra prezije restart (`--only zaznam-padu` jen ten).
            if opts.test_only.is_empty() || opts.test_only == "zaznam-padu" {
                crashtest::run(&mut opts)?;
            }
            // Zive USB: obraz s beznym jadrem (ne testovacim) nastartuje
            // z flash disku USB az do plochy s terminalem (vyvojovy profil);
            // obraz bez `accounts-dev` (jako obraz pro uzivatele) jen do
            // prihlasovaci obrazovky.
            if opts.test_only.is_empty() || usb_selected {
                opts.test = false;
                let extra = std::mem::take(&mut opts.extra_initfs);
                let r = usb::test_run(&mut opts);
                opts.extra_initfs = extra;
                opts.test = true;
                r?;
            }
            // Instalator: zive USB -> prazdny disk -> start z nej; vedle
            // Windows (`--only instalace` jen tyto behy).
            if opts.test_only.is_empty() || install_selected {
                opts.test = false;
                let extra = std::mem::take(&mut opts.extra_initfs);
                let r = usb::install_test::run(&mut opts);
                opts.extra_initfs = extra;
                opts.test = true;
                r?;
            }
            // Graficky instalator: stejna instalace pres okno se snimky
            // kroku (`--only instalace-gui` jen tento beh).
            if opts.test_only.is_empty() || install_gui_selected {
                opts.test = false;
                let extra = std::mem::take(&mut opts.extra_initfs);
                let r = usb::install_test::run_gui(&mut opts);
                opts.extra_initfs = extra;
                opts.test = true;
                r?;
            }
            Ok(())
        }
        Some("check") => check_kernel(opts.arch, &opts),
        Some("usb") => usb::command(&mut opts, usb_args.as_ref().unwrap()),
        Some("vmware") => vmware::command(&mut opts, vmware_dir),
        Some("symbolize") => {
            let file = symbolize_file.ok_or("symbolize ocekava soubor (zaznam padu, balicek, log)")?;
            let a = opts.arch;
            let profile = profile_dir(&opts);
            let kernel = target_dir().join(a.kernel_target).join(profile).join("kernel");
            let user = user_target_dir().join(a.user_target).join(profile);
            symbolize::file(&file, &kernel, &user)
        }
        Some("docs") => build_docs().map(|_| ()),
        Some("venus-gen") => venus::run(false),
        Some("venus-check") => venus::run(true),
        Some("help" | "-h" | "--help") => {
            print!("{USAGE}");
            Ok(())
        }
        _ => Err(format!("neznamy prikaz {command:?}\n\n{USAGE}").into()),
    }
}

/// Ma QEMU zarizeni `edu` (ukazkovy ovladac)? Ve WSL se nezjistuje (bez
/// zarizeni test edu jen preskoci).
fn edu_available(opts: &Options) -> bool {
    if opts.qemu_wsl {
        return false;
    }
    let ok = opts
        .arch
        .find_qemu()
        .is_ok_and(|q| drivers::qemu_has_device(&q, "edu"));
    if !ok {
        println!("xtask: QEMU nema zarizeni edu: test ukazkoveho ovladace se preskoci");
    }
    ok
}

/// Testy sdilenych knihoven na hostiteli (format initfs, ABI, dokumentace
/// chyb a syscallu, casova pasma, protokol vstupu a rozlozeni klavesnice,
/// GUI: protokol, kresleni, pismo, virtio-gpu, kompozitor, knihovna UI,
/// ochrana cizich disku), driverkit se simulaci a ovladace z manifestu
/// (`found`; s `--driver` jen ten jeden).
/// `cargo xtask test --update-shots`: znovu vytvori referencni snimky
/// vzhledu `libs/ui/tests/shots/` a `libs/comp/tests/shots/` (jen vyslovne, nikdy automaticky).
fn update_shots() -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo).current_dir(workspace_root()).env("UPDATE_SHOTS", "1").args(["test", "--quiet", "--package", "ui", "--package", "comp", "shot_"]).status()?;
    if !status.success() {
        return Err("snimky vzhledu se nepodarilo vytvorit".into());
    }
    println!("xtask: referencni snimky v libs/ui/tests/shots prepsany; zkontroluj je okem (git diff) pred commitem");
    Ok(())
}

fn host_tests(found: &[drivers::Found], only_driver: Option<&str>) -> Result<()> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    if let Some(d) = only_driver {
        let status = Command::new(&cargo)
            .current_dir(workspace_root())
            .args(["test", "--quiet", "--package", "driverkit", "--package", d])
            .args(["--features", "driverkit/sim"])
            .status()?;
        if !status.success() {
            return Err(format!("testy ovladace {d} na hostiteli selhaly").into());
        }
        println!("xtask: testy ovladace {d} na hostiteli prosly");
        return Ok(());
    }
    let mut drivers_args: Vec<&str> = vec!["--package", "driverkit"];
    for f in found {
        drivers_args.extend(["--package", f.manifest.name.as_str()]);
    }
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
            "sizeclass",
            "--package",
            "schedcore",
            "--package",
            "term",
            "--package",
            "netproto",
            "--package",
            "nic",
            "--package",
            "csprng",
            "--package",
            "wire",
            "--package",
            "audioproto",
            "--package",
            "audiomix",
            "--package",
            "hdaspec",
            "--package",
            "conf",
            "--package",
            "manifest",
            "--package",
            "netfilter",
            "--package",
            "guiproto",
            "--package",
            "clipproto",
            "--package",
            "gpuproto",
            "--package",
            "venus",
            "--package",
            "spirv",
            "--package",
            "vkcomp",
            "--package",
            "taskproto",
            "--package",
            "accountproto",
            "--package",
            "accounts",
            "--package",
            "taskcore",
            "--package",
            "svcclient",
            "--package",
            "taskview",
            "--package",
            "fileops",
            "--package",
            "filesview",
            "--package",
            "openwith",
            "--package",
            "installview",
            "--package",
            "gfx",
            "--package",
            "font",
            "--package",
            "vgpu",
            "--package",
            "fbcopy",
            "--package",
            "display",
            "--package",
            "displaydrv",
            "--package",
            "intelgfx",
            "--package",
            "comp",
            "--package",
            "ui",
            "--package",
            "tls",
            "--package",
            "httpc",
            "--package",
            "dns",
            "--package",
            "diskguard",
            "--package",
            "crashrec",
            "--package",
            "qr",
            "--package",
            "fatw",
            "--package",
            "installplan",
            "--package",
            "agentproto",
            "--package",
            "agentsh",
            "--features",
            "initfs/std,fs/std,netproto/std,netproto/nts,driverkit/sim,crashrec/alloc",
        ])
        .args(&drivers_args)
        .status()?;
    if !status.success() {
        return Err("testy knihoven na hostiteli selhaly".into());
    }
    // Testy xtasku samotneho (dokumentace, zive USB: pravidla zapisu na
    // disk, rozlozeni obrazu). Prelozi se zvlastni testovaci binarka, bezici
    // xtask se neprepisuje.
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .current_dir(workspace_root())
        .args(["test", "--quiet", "--package", "xtask"])
        .status()?;
    if !status.success() {
        return Err("testy xtasku selhaly".into());
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
    // Stranka "Prikazy shellu" z metadat prikazu (jeden zdroj pravdy:
    // libs/shlang/src/doc); zapise se jen pri zmene.
    let root = workspace_root();
    // Soubezne `cargo xtask test` stavi web do stejneho `target/docs`.
    let _lock = lock_shared("docs")?;
    for (en, file) in [(false, "docs/cs/prikazy-shellu.md"), (true, "docs/en/shell-commands.md")] {
        let md = shlang::doc::markdown(en);
        let path = root.join(file);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(md.as_str()) {
            std::fs::write(&path, md)?;
            println!("xtask: {file} vygenerovano z metadat prikazu");
        }
    }
    let index = docs::build(&root, &target_dir().join("docs"))?;
    println!("xtask: dokumentace {}", index.display());
    Ok(index)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

pub(crate) fn target_dir() -> PathBuf {
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
    "svctest",
    "fsrtest",
    "devmgr",
    "acpid",
    "virtio-blk",
    "nvme",
    "ahci",
    "virtio-input",
    "inputd",
    "ps2",
    "xhci",
    "usb-hub",
    "usb-hid",
    "usb-storage",
    "logd",
    "logrescue",
    "installd",
    "fsd",
    "timed",
    "termd",
    "shsess",
    "taskd",
    "console",
    "virtio-net",
    "e1000e",
    "igc",
    "r8169",
    "virtio-rng",
    "virtio-gpu",
    "fbdisplay",
    "splash",
    "hda",
    "audiod",
    "bochs-display",
    "intel-display",
    "compd",
    "gui-demo",
    "gui-komponenty",
    "gui-term",
    "gui-tasks",
    "gui-files",
    "files-copy",
    "gui-install",
    "netd",
    "fwd",
    "netprobe",
    "ntpd",
    "fetch",
    "revocd",
    "accountd",
    "dnsd",
    "vk-probe",
    "agentd",
];

/// Dalsi soubory initfs ze zdrojaku: (jmeno v initfs, cesta v repozitari).
/// Manifesty aplikaci lezi vedle programu (`<program>.manifest.toml`).
const INITFS_FILES: &[(&str, &str)] = &[
    ("firewall.toml", "user/fwd/firewall.toml"),
    ("fwd.manifest.toml", "user/fwd/fwd.manifest.toml"),
    ("netd.manifest.toml", "user/netd/netd.manifest.toml"),
    ("netprobe.manifest.toml", "user/netprobe/netprobe.manifest.toml"),
    ("gui-demo.manifest.toml", "user/gui-demo/gui-demo.manifest.toml"),
    ("gui-komponenty.manifest.toml", "user/gui-komponenty/gui-komponenty.manifest.toml"),
    ("gui-term.manifest.toml", "user/gui-term/gui-term.manifest.toml"),
    ("ntpd.manifest.toml", "user/ntpd/ntpd.manifest.toml"),
    ("ntpd.toml", "user/ntpd/ntpd.toml"),
    ("fetch.manifest.toml", "user/fetch/fetch.manifest.toml"),
    ("revocd.manifest.toml", "user/revocd/revocd.manifest.toml"),
    ("revocd.toml", "user/revocd/revocd.toml"),
    ("accountd.manifest.toml", "user/accountd/accountd.manifest.toml"),
    ("dnsd.manifest.toml", "user/dnsd/dnsd.manifest.toml"),
    ("dnsd.toml", "user/dnsd/dnsd.toml"),
    ("gui-tasks.manifest.toml", "user/gui-tasks/gui-tasks.manifest.toml"),
    ("gui-files.manifest.toml", "user/gui-files/gui-files.manifest.toml"),
    ("files-copy.manifest.toml", "user/files-copy/files-copy.manifest.toml"),
    ("gui-install.manifest.toml", "user/gui-install/gui-install.manifest.toml"),
    ("agentd.manifest.toml", "user/agentd/agentd.manifest.toml"),
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
    if packages.contains(&"kernel") {
        // Verze a commit do jadra (log pri startu, zaznam padu). Jen jadru:
        // zmena commitu tak neprekladá znovu cely userspace.
        let (version, build) = build_info();
        cmd.env("MO_VERSION", version).env("MO_BUILD", build);
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
        // Kryptografie RustCrypto (libs/tls) jen softwarove: programy bezi
        // bez SSE/NEON a detekce AES-NI za behu by spustila instrukce SIMD.
        let flags = [
            "-Crelocation-model=static",
            "-Cforce-frame-pointers=yes",
            script.as_str(),
            "--cfg=aes_force_soft",
            "--cfg=polyval_force_soft",
            "--cfg=poly1305_force_soft",
            "--cfg=chacha20_force_soft",
            "--cfg=curve25519_dalek_backend=\"serial\"",
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

/// Program pro initfs: jen to, co zavadec potrebuje (hlavicka ELF, tabulka
/// segmentu a obsah segmentu PT_LOAD). Sekce a ladici informace (u ladiciho
/// buildu desitky MiB) se vynechaji - zavadec je nepouziva a initfs by se
/// nevesel na systemovy disk. Plne binarky zustavaji na hostiteli (GDB).
/// Soubor, ktery neni ELF64 little endian, zustane beze zmeny.
fn loadable_only(data: &[u8]) -> Vec<u8> {
    let u16_at = |o: usize| data.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as u64);
    let u64_at = |o: usize| {
        data.get(o..o + 8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    };
    if data.len() < 64 || &data[..4] != b"\x7fELF" || data[4] != 2 || data[5] != 1 {
        return data.to_vec();
    }
    let (Some(phoff), Some(phentsize), Some(phnum)) = (u64_at(32), u16_at(54), u16_at(56)) else {
        return data.to_vec();
    };
    let mut end = phoff + phentsize * phnum;
    for i in 0..phnum {
        let ph = (phoff + i * phentsize) as usize;
        let (Some(off), Some(filesz)) = (u64_at(ph + 8), u64_at(ph + 32)) else {
            return data.to_vec();
        };
        end = end.max(off + filesz);
    }
    let end = (end as usize).min(data.len());
    let mut out = data[..end].to_vec();
    // Bez tabulky sekci: e_shoff = 0, e_shnum = 0, e_shstrndx = 0.
    out[40..48].fill(0);
    out[60..64].fill(0);
    out
}

fn user_target_dir() -> PathBuf {
    target_dir().join("user")
}

fn build(opts: &Options) -> Result<Artifacts> {
    let a = opts.arch;
    cargo_build(&["kernel"], a.kernel_target, opts, false)?;
    cargo_build(&["loader"], a.loader_target, opts, false)?;
    // Ovladace z manifestu (user/*/<jmeno>.driver.toml) se pridaji samy.
    let found = drivers::discover(&workspace_root())?;
    let mut programs: Vec<&str> = USER_PROGRAMS.to_vec();
    for f in &found {
        if !programs.contains(&f.manifest.name.as_str()) {
            programs.push(f.manifest.name.as_str());
        }
    }
    cargo_build(&programs, a.user_target, opts, true)?;
    // Aplikace se std (vlastni cil a sysroot, docs/cs/rust-std.md).
    let std_bin = stdsys::build_apps(stdsys::STD_PROGRAMS, opts)?;
    let target = target_dir();
    let profile = profile_dir(opts);

    let user_bin = user_target_dir().join(a.user_target).join(profile);
    let mut files = Vec::new();
    for name in &programs {
        let data = std::fs::read(user_bin.join(name))
            .map_err(|e| format!("nelze cist program {name}: {e}"))?;
        files.push((name.to_string(), loadable_only(&data)));
    }
    for name in stdsys::STD_PROGRAMS {
        let data = std::fs::read(std_bin.join(name))
            .map_err(|e| format!("nelze cist aplikaci {name}: {e}"))?;
        files.push((name.to_string(), loadable_only(&data)));
    }
    for (name, path) in INITFS_FILES {
        let data = std::fs::read(workspace_root().join(path))
            .map_err(|e| format!("nelze cist {path}: {e}"))?;
        files.push((name.to_string(), data));
    }
    for f in &found {
        let data = std::fs::read(&f.path).map_err(|e| format!("nelze cist {}: {e}", f.path.display()))?;
        files.push((Manifest::file_name(&f.manifest.name), data));
    }
    files.push((String::from("certs.pem"), webhelper::bundle_pem()));
    files.extend(opts.extra_initfs.iter().cloned());
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, d)| (n.as_str(), d.as_slice()))
        .collect();
    // Vlastni soubor behu: jiny soubezny beh z nej muze prave skladat obraz.
    let initfs = run_file(&format!("initfs-{}-{profile}", a.name), "img");
    std::fs::write(&initfs, initfs::build(&refs))?;

    Ok(Artifacts {
        kernel: target.join(a.kernel_target).join(profile).join("kernel"),
        loader: target.join(a.loader_target).join(profile).join("loader.efi"),
        initfs,
    })
}

fn image_for(artifacts: &Artifacts, opts: &Options) -> Result<PathBuf> {
    // Testovaci obraz je vlastni pro beh (soubezne testy, QEMU ho ma
    // otevreny); bezny obraz (`image`, `run`) ma pevnou cestu.
    let path = if opts.test {
        run_file(&format!("os-{}-{}-test", opts.arch.name, profile_dir(opts)), "img")
    } else {
        target_dir().join(format!("os-{}-{}.img", opts.arch.name, profile_dir(opts)))
    };
    let files = [
        (opts.arch.boot_file, artifacts.loader.as_path()),
        ("os/kernel.elf", artifacts.kernel.as_path()),
        ("os/initfs.img", artifacts.initfs.as_path()),
    ];
    image::create_disk_image(&path, &files)?;
    println!("xtask: obraz disku {}", path.display());
    Ok(path)
}

/// Konzole Windows v UTF-8: vystup seriove linky (cesky text, ramecky)
/// je v UTF-8, konzole ve vychozi kodove strance (852, 437) by z nej
/// udelala "┼ż". Na ostatnich systemech nic.
fn console_utf8() {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn SetConsoleOutputCP(code_page: u32) -> i32;
        }
        // SAFETY: volani bez ukazatelu; selhani (zadna konzole) nevadi.
        unsafe {
            SetConsoleOutputCP(65001);
        }
    }
}

/// Informace o sestaveni z gitu: (kratka verze do zaznamu padu, max 32
/// znaku, napr. `MagnumOpus 0.1.0 130f2e5*`; uplny popis do logu, napr.
/// `130f2e5* (2026-11-08, release)`). `*` = neulozene zmeny v kodu.
/// Bez gitu `?`.
pub fn build_info() -> (String, String) {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(workspace_root())
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| String::from("?"));
    let dirty = git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    let date = git(&["log", "-1", "--format=%cd", "--date=format:%Y-%m-%d"]).unwrap_or_default();
    let mark = if dirty { "*" } else { "" };
    let version = format!("MagnumOpus {} {commit}{mark}", env!("CARGO_PKG_VERSION"));
    (version, format!("{commit}{mark} ({date})"))
}
