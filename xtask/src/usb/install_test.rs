//! Test instalatoru v QEMU (`cargo xtask test`, `--only instalace`):
//!
//! 1. **cely disk**: zive USB (soubor odpovedi `install-answers` v initfs:
//!    init nainstaluje bez dotazu) + prazdny disk NVMe -> instalace; pak
//!    start BEZ flashky z NVMe (nainstalovany zavadec, jadro, initfs bez
//!    vyvojoveho profilu) az na prihlasovaci obrazovku s polozkou NVRAM;
//!    na hostiteli overi GPT, FS, `system.toml` a heslo zalozeneho uctu
//!    (stejna kontrola jako sluzba uctu pri prihlaseni: Argon2id),
//! 2. **vedle Windows**: disk NVMe s "Windows" (ESP a datovy oddil se
//!    znackou) a volnym mistem -> instalace do volneho mista; oddily
//!    Windows (tabulka i obsah) zustanou bajt po bajtu stejne.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use diskguard::gpt::{self, NewPartition, Table};
use diskguard::install::OUR_ESP_LABEL;
use diskguard::{ESP_TYPE, OUR_TYPE};

use super::{INSTALL_ANSWERS, boot, create_image, with_dev_profile};
use crate::arch::Arch;
use crate::{Options, Result, target_dir};

const GIB: u64 = 1 << 30;
/// Cilovy disk (instalator chce aspon 512 MiB ESP + 8 GiB dat).
const TARGET: u64 = 12 * GIB;
const PASSWORD: &str = "test instalace heslo";
const MODEL: &str = "QEMU NVMe Ctrl";

fn answers(mode: &str) -> String {
    format!(
        "disk = \"{MODEL}\"\nzpusob = \"{mode}\"\npotvrzeni = \"{MODEL}\"\njmeno = \"tester\"\nzobrazovane = \"Tester Instalace\"\n\
         heslo = \"{PASSWORD}\"\npin = \"\"\nklavesnice = [\"cz\", \"us\"]\npasmo = \"Europe/Prague\"\nvychozi-system = \"magnumopus\"\n"
    )
}

/// Ridky soubor (Windows: priznak sparse, jinak by NTFS pri zapisu na konec
/// disku nuloval cely soubor).
fn sparse(path: &Path, bytes: u64) -> Result<()> {
    let f = File::create(path)?;
    drop(f);
    if cfg!(windows) {
        let _ = std::process::Command::new("fsutil").args(["sparse", "setflag"]).arg(path).output();
    }
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_len(bytes)?;
    Ok(())
}

fn read_at(f: &mut File, at: u64, buf: &mut [u8]) -> bool {
    f.seek(SeekFrom::Start(at)).is_ok() && f.read_exact(buf).is_ok()
}

fn table(path: &Path) -> Result<Table> {
    super::image::read_gpt(path)
}

/// Spusti oba scenare.
pub fn run(opts: &mut Options) -> Result<()> {
    whole_disk(opts)?;
    next_to_windows(opts)
}

fn live_image(opts: &mut Options, mode: &str) -> Result<PathBuf> {
    with_dev_profile(opts);
    opts.extra_initfs.retain(|(n, _)| n != INSTALL_ANSWERS);
    opts.extra_initfs.push((String::from(INSTALL_ANSWERS), answers(mode).into_bytes()));
    let artifacts = crate::build(opts);
    opts.extra_initfs.retain(|(n, _)| n != INSTALL_ANSWERS);
    let artifacts = artifacts?;
    let path = target_dir().join(format!("usb-install-{}-{}.img", opts.arch.name, std::process::id()));
    create_image(opts, &artifacts, &path, super::TEST_MIB)?;
    Ok(path)
}

fn nvme(path: &Path) -> Vec<String> {
    vec![
        String::from("-drive"),
        format!("if=none,id=target,format=raw,file={}", path.display()),
        String::from("-device"),
        format!("nvme,drive=target,serial=MO-TARGET,bootindex=1"),
    ]
}

fn install_markers(t: &str) -> Vec<&'static str> {
    if t.contains("init: test instalace selhala") {
        // Neprazdne = test bezi dal az do vyprseni; selhani poznat hned.
        return vec!["CHYBA: init: test instalace selhala (viz log)"];
    }
    ["init: test instalace: HOTOVO", "installd: instalace dokoncena"].into_iter().filter(|m| !t.contains(m)).collect()
}

fn whole_disk(opts: &mut Options) -> Result<()> {
    let arch: &Arch = opts.arch;
    let usb = live_image(opts, "cely-disk")?;
    let target = target_dir().join(format!("install-target-{}.img", std::process::id()));
    sparse(&target, TARGET)?;
    let r = (|| -> Result<()> {
        boot::boot_until(arch, &usb, &nvme(&target), false, "instalace na prazdny disk", install_markers)?;
        // Start z nainstalovaneho disku bez flashky.
        boot::boot_until(arch, Path::new(""), &nvme(&target), false, "start nainstalovaneho systemu", |t| {
            ["NVRAM: zapsana polozka", "fsd: pripojeno 'MagnumOpus'", "accountd: bezim (1 uctu", "compd: prihlasovaci obrazovka:"]
                .into_iter()
                .filter(|m| !t.contains(m))
                .collect()
        })?;
        verify_installed(&target)
    })();
    let _ = std::fs::remove_file(&usb);
    let _ = std::fs::remove_file(&target);
    r?;
    println!("xtask: instalace na prazdny disk a start z nej prosly");
    Ok(())
}

/// Najde nase oddily (ESP, data) v GPT.
fn ours(t: &Table) -> Result<((u64, u64), (u64, u64))> {
    let esp = t.entries().find(|e| e.type_guid == ESP_TYPE && e.name_is(OUR_ESP_LABEL)).ok_or("chybi ESP MagnumOpus")?;
    let data = t.entries().find(|e| e.type_guid == OUR_TYPE && e.name_is(diskguard::OUR_LABEL)).ok_or("chybi oddil MagnumOpus")?;
    Ok(((esp.first, esp.last), (data.first, data.last)))
}

/// GPT, soubory na ESP, FS s nastavenim a heslo uctu.
fn verify_installed(target: &Path) -> Result<()> {
    let t = table(target)?;
    let ((esp0, esp1), (d0, d1)) = ours(&t)?;
    if (esp1 - esp0 + 1) * 512 != installplan::ESP_BYTES {
        return Err("ESP nema 512 MiB".into());
    }
    // ESP: FAT32 se soubory (nezavisle cteni crate fatfs).
    let mut f = File::open(target)?;
    let mut esp = vec![0u8; ((esp1 - esp0 + 1) * 512) as usize];
    if !read_at(&mut f, esp0 * 512, &mut esp) {
        return Err("ESP nejde precist".into());
    }
    {
        let fs = fatfs::FileSystem::new(std::io::Cursor::new(&mut esp[..]), fatfs::FsOptions::new())?;
        for p in ["os/kernel.elf", "os/initfs.img", "os/boot.cfg"] {
            fs.root_dir().open_file(p).map_err(|e| format!("ESP: {p}: {e}"))?;
        }
        let mut s = String::new();
        fs.root_dir().open_file("os/boot.cfg")?.read_to_string(&mut s)?;
        if !s.contains("nvram = \"hotovo\"") {
            return Err(format!("boot.cfg po prvnim startu: {s}").into());
        }
        // Instalovany initfs nesmi mit vyvojovy profil ani odpovedi.
        let mut init = Vec::new();
        fs.root_dir().open_file("os/initfs.img")?.read_to_end(&mut init)?;
        let i = initfs::Initfs::parse(&init).map_err(|e| format!("initfs: {e:?}"))?;
        for bad in [super::DEV_PROFILE.0, INSTALL_ANSWERS] {
            if i.find(bad).is_some() {
                return Err(format!("nainstalovany initfs obsahuje {bad}").into());
            }
        }
    }
    drop(esp);
    // FS: nastaveni a ucet.
    let mut fsys = super::image::open_fs(&f, d0 * 512, (d1 - d0 + 1) * 512)?;
    let read = |fsys: &mut fs::Fs<super::image::PartDevice>, p: &str| -> Result<Vec<u8>> {
        let st = fsys.stat(p).map_err(|e| format!("{p}: {e:?}"))?;
        fsys.read(p, 0, st.size as usize).map_err(|e| format!("{p}: {e:?}").into())
    };
    let toml = String::from_utf8(read(&mut fsys, installplan::config::PATH)?)?;
    let s = installplan::config::Settings::parse(&toml);
    if s.keyboards != ["cz", "us"] || s.zone != "Europe/Prague" {
        return Err(format!("system.toml: {toml}").into());
    }
    let users = fsys.list("/system/accounts/users").map_err(|e| format!("ucty: {e:?}"))?;
    let [u] = users.as_slice() else { return Err(format!("ocekavan 1 ucet, je {}", users.len()).into()) };
    let rec = read(&mut fsys, &format!("/system/accounts/users/{}", u.name))?;
    let a = accounts::Account::decode(&rec).map_err(|e| format!("zaznam uctu: {e:?}"))?;
    if a.name != "tester" {
        return Err(format!("ucet {}", a.name).into());
    }
    let v = a.password.ok_or("ucet nema heslo")?;
    let pw = accounts::auth::normalize(&accountproto::Secret::from_slice(PASSWORD.as_bytes())).map_err(|e| format!("{e:?}"))?;
    let d = accounts::kdf::derive(&v.params, pw.expose(), &v.salt).map_err(|e| format!("{e:?}"))?;
    if !accounts::kdf::ct_eq(d.verifier(), &v.value) {
        return Err("heslo zalozeneho uctu neplati".into());
    }
    fsys.stat("/home/tester").map_err(|e| format!("domov: {e:?}"))?;
    Ok(())
}

/// Disk s "Windows": ESP 100 MiB a datovy oddil se znackou do 3 GiB,
/// zbytek volny.
fn windows_disk(path: &Path) -> Result<()> {
    use std::io::Write;
    sparse(path, TARGET)?;
    let basic = diskguard::BASIC_DATA_TYPE;
    let parts = [
        NewPartition { type_guid: ESP_TYPE, guid: [1; 16], first: 2048, last: 206_847, attrs: 0, name: "EFI system partition" },
        NewPartition { type_guid: basic, guid: [2; 16], first: 206_848, last: 3 * GIB / 512, attrs: 0, name: "Basic data partition" },
    ];
    let ends = gpt::build(TARGET / 512, 512, [7; 16], &parts)?;
    let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&ends.head)?;
    f.seek(SeekFrom::Start(TARGET - ends.tail.len() as u64))?;
    f.write_all(&ends.tail)?;
    for (at, mark) in [(2048 * 512, b"WINDOWS-ESP".as_slice()), (206_848 * 512, b"WINDOWS-C".as_slice())] {
        f.seek(SeekFrom::Start(at))?;
        f.write_all(mark)?;
    }
    Ok(())
}

/// Bajty oddilu Windows (zacatek a konec kazdeho, 1 MiB) a jejich polozky.
fn windows_snapshot(path: &Path) -> Result<Vec<u8>> {
    let t = table(path)?;
    let mut f = File::open(path)?;
    let mut out = Vec::new();
    for e in t.entries().filter(|e| !e.is_empty() && !diskguard::install::is_installer_owned(e)) {
        out.extend_from_slice(&e.first.to_le_bytes());
        out.extend_from_slice(&e.last.to_le_bytes());
        out.extend_from_slice(&e.type_guid);
        for at in [e.first, e.last.saturating_sub(2047)] {
            let mut b = vec![0u8; 1 << 20];
            read_at(&mut f, at * 512, &mut b);
            out.extend_from_slice(&b);
        }
    }
    Ok(out)
}

fn next_to_windows(opts: &mut Options) -> Result<()> {
    let arch: &Arch = opts.arch;
    let usb = live_image(opts, "volne-misto")?;
    let target = target_dir().join(format!("install-windows-{}.img", std::process::id()));
    windows_disk(&target)?;
    let before = windows_snapshot(&target)?;
    let mbr_before = {
        let mut b = vec![0u8; 512];
        read_at(&mut File::open(&target)?, 0, &mut b);
        b
    };
    let r = (|| -> Result<()> {
        boot::boot_until(arch, &usb, &nvme(&target), false, "instalace vedle Windows", install_markers)?;
        let after = windows_snapshot(&target)?;
        if after != before {
            return Err("instalace ZMENILA oddily Windows".into());
        }
        let mut mbr = vec![0u8; 512];
        read_at(&mut File::open(&target)?, 0, &mut mbr);
        if mbr != mbr_before {
            return Err("instalace zmenila ochranny MBR".into());
        }
        let t = table(&target)?;
        ours(&t)?;
        if t.entries().filter(|e| !e.is_empty()).count() != 4 {
            return Err("ocekavany 4 oddily (2 Windows + 2 nase)".into());
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&usb);
    let _ = std::fs::remove_file(&target);
    r?;
    println!("xtask: instalace vedle Windows prosla (oddily Windows beze zmeny)");
    Ok(())
}
