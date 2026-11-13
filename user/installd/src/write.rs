//! Provedeni instalace podle planu (`installplan::Plan`): tabulka oddilu,
//! ESP se zavadecem a systemem, souborovy system s nastavenim a prvnim
//! uctem, overeni cteni zpet. Kazdy krok hlasi prubeh.
//!
//! Poradi zapisu GPT: nejdriv zalozni tabulka a hlavicka na konci disku,
//! pak primarni na zacatku (u pridani oddilu bez sektoru 0 - ochranny MBR
//! jineho systemu zustane). Preruseni mezi nimi = primarni je stara
//! a platna, zalozni nova; disk zustane citelny.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use blockproto::SECTOR;
use diskguard::ESP_TYPE;
use fs::{Algo, Fs, FormatOptions, Kind, MountOptions};
use installplan::config::{self, Settings};
use installplan::proto::AccountInput;
use installplan::{GptWrite, Plan};
use libos::{Error, Result, println};

use crate::disks::{Disk, FsPart, Part};

/// Soubory systemu z ESP zive flashky: (cesta na flashce, cesta na cili).
/// Instaluje se `initfs-install.img` (bez vyvojoveho autoprihlaseni).
fn files() -> [(&'static str, &'static str); 3] {
    let boot = if cfg!(target_arch = "aarch64") { "EFI/BOOT/BOOTAA64.EFI" } else { "EFI/BOOT/BOOTX64.EFI" };
    [(boot, boot), ("os/kernel.elf", "os/kernel.elf"), ("os/initfs-install.img", "os/initfs.img")]
}

/// Volby zavadece na cilove ESP (`os/boot.toml`, cte loader): vychozi
/// system a zda ma loader pri prvnim startu zapsat polozku do NVRAM.
fn boot_toml(default_magnum: bool) -> String {
    format!(
        "# Zavadec MagnumOpus (zapsal instalator)\nnvram = \"zapsat\"\nvychozi = \"{}\"\n",
        if default_magnum { "magnumopus" } else { "jiny" }
    )
}

fn now_s() -> i64 {
    libos::time::wall().map(|t| t.secs).unwrap_or(0)
}

pub struct Ctx<'a> {
    pub target: &'a Disk,
    /// Disk zive flashky (zdroj systemu).
    pub live: &'a Disk,
    pub progress: &'a mut dyn FnMut(u32, &str),
    pub qemu_kdf: bool,
}

/// Provede plan; vraci souhrn pro uzivatele.
pub fn install(cx: &mut Ctx, plan: &Plan, settings: &Settings, account: &AccountInput) -> Result<String> {
    // Zdroj nejdriv (chyba = nic se nezapsalo).
    let sources = read_sources(cx.live)?;

    (cx.progress)(0, "Zapisuji tabulku oddílů");
    match &plan.gpt {
        GptWrite::New(e) | GptWrite::Add(e) => {
            let sectors = cx.target.sectors();
            let tail_at = sectors - (e.tail.len() / SECTOR) as u64;
            cx.target.write(tail_at, &e.tail)?;
            cx.target.client.flush()?;
            cx.target.write(e.head_lba, &e.head)?;
            cx.target.client.flush()?;
        }
        GptWrite::Keep => {}
    }

    (cx.progress)(1, "Formátuji oddíl EFI");
    let esp_sectors = plan.esp.1 - plan.esp.0 + 1;
    let mut esp = Part { disk: cx.target, first: plan.esp.0, sectors: esp_sectors };
    let mut id = [0u8; 4];
    crate::random(&mut id);
    fatw::format::format(&mut esp, esp_sectors, b"MAGNUMOS   ", plan.esp.0, u32::from_le_bytes(id)).map_err(fat_err)?;

    (cx.progress)(2, "Kopíruji zavaděč a systém");
    let mut vol = fatw::Volume::open(esp).map_err(fat_err)?;
    for ((_, to), data) in files().iter().zip(&sources) {
        let (dir, name) = to.rsplit_once('/').unwrap_or(("", to));
        vol.write_file(dir, name, data, fatw::Stamp::default()).map_err(fat_err)?;
    }
    vol.write_file("os", "boot.toml", boot_toml(settings.default_magnum).as_bytes(), fatw::Stamp::default())
        .map_err(fat_err)?;

    let data_part = || Part { disk: cx.target, first: plan.data.0, sectors: plan.data.1 - plan.data.0 + 1 };
    let mut fs = if plan.format_data {
        (cx.progress)(3, "Vytvářím souborový systém");
        let mut uuid = [0u8; 16];
        crate::random(&mut uuid);
        let opts = FormatOptions {
            label: String::from(diskguard::OUR_LABEL),
            algo: Algo::Xxh3,
            ag_blocks: None,
            uuid: Some(uuid),
            time: fs::Timestamp::new(now_s(), 0),
        };
        let dev = Fs::format(FsPart(data_part()), &opts)?;
        Fs::mount(dev)?
    } else {
        (cx.progress)(3, "Připojuji existující souborový systém (data zůstanou)");
        Fs::mount(FsPart(data_part()))?
    };
    fs.set_time(fs::Timestamp::new(now_s(), 0));

    (cx.progress)(4, "Ukládám nastavení");
    for dir in ["/system"] {
        match fs.create(dir, Kind::Dir) {
            Ok(()) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    for ds in ["/system/config", "/system/log"] {
        match fs.create_dataset(ds) {
            Ok(_) | Err(Error::AlreadyExists) => {}
            Err(e) => return Err(e),
        }
    }
    let toml = settings.to_toml();
    match fs.create(config::PATH, Kind::File) {
        Ok(()) | Err(Error::AlreadyExists) => {}
        Err(e) => return Err(e),
    }
    fs.truncate(config::PATH, 0)?;
    fs.write(config::PATH, 0, toml.as_bytes())?;

    (cx.progress)(5, "Vytvářím účet");
    let created = crate::account::create_first(
        &mut fs,
        &account.name,
        &account.display,
        &account.password,
        &account.pin,
        cx.qemu_kdf,
        now_s(),
    )?;
    fs.commit()?;
    drop(fs);

    (cx.progress)(6, "Ověřuji instalaci");
    verify(cx, plan, &sources)?;

    let mut s = String::from("Instalace je hotová.\n");
    match created {
        Some((uid, phrase)) => {
            s.push_str(&format!("Účet {} (číslo {uid}) je založený.\n", account.name));
            if !phrase.is_empty() {
                s.push_str(&format!("Bezpečnostní fráze: \"{phrase}\" - ukazují ji jen opravdové systémové dialogy.\n"));
            }
        }
        None => s.push_str("Účty z předchozí instalace zůstaly (nový účet se nezakládal).\n"),
    }
    s.push_str("Po restartu vyndej USB.\n");
    if plan.other_os {
        s.push_str(&format!(
            "Výchozí systém při startu: {}. Druhý vybereš v nabídce firmwaru (F12, F11 nebo Esc).\n",
            installplan::default_os_text(settings.default_magnum)
        ));
    }
    s.push_str("Secure Boot ve firmwaru musí být vypnutý.\n");
    println!("installd: instalace dokoncena na '{}'", cx.target.model);
    Ok(s)
}

fn fat_err(e: fatw::Error) -> Error {
    match e {
        fatw::Error::Io => Error::IoError,
        fatw::Error::Full => Error::NoSpace,
        fatw::Error::BadName => Error::InvalidArgs,
        fatw::Error::NotFat32 | fatw::Error::Corrupted => Error::Corrupted,
    }
}

/// ESP zive flashky: soubory systemu.
fn read_sources(live: &Disk) -> Result<Vec<Vec<u8>>> {
    let t = live.table().ok_or(Error::NotFound)?;
    let esp = t.entries().find(|e| !e.is_empty() && e.type_guid == ESP_TYPE).ok_or(Error::NotFound)?;
    let part = Part { disk: live, first: esp.first, sectors: esp.last - esp.first + 1 };
    let mut vol = fatw::Volume::open(part).map_err(fat_err)?;
    let mut out = Vec::new();
    for (from, _) in files() {
        match vol.read_file(from).map_err(fat_err)? {
            Some(d) => out.push(d),
            None => {
                println!("installd: na flashce chybi {from}");
                return Err(Error::NotFound);
            }
        }
    }
    Ok(out)
}

/// Overeni: GPT ukazuje nas oddil, soubory na ESP sedi, FS jde pripojit
/// (jen pro cteni) a ma nastaveni.
fn verify(cx: &Ctx, plan: &Plan, sources: &[Vec<u8>]) -> Result<()> {
    let sectors = cx.target.sectors();
    let d = diskguard::decide(sectors, SECTOR, |lba, buf| cx.target.read(lba, buf).is_ok());
    if !d.kind.is_ours() || d.start != plan.data.0 {
        println!("installd: overeni: ochrana disku nevidi nas oddil ({:?})", d.kind);
        return Err(Error::Corrupted);
    }
    let esp = Part { disk: cx.target, first: plan.esp.0, sectors: plan.esp.1 - plan.esp.0 + 1 };
    let mut vol = fatw::Volume::open(esp).map_err(fat_err)?;
    for ((_, to), data) in files().iter().zip(sources) {
        if vol.read_file(to).map_err(fat_err)?.as_deref() != Some(data.as_slice()) {
            println!("installd: overeni: {to} na ESP nesouhlasi");
            return Err(Error::Corrupted);
        }
    }
    let part = Part { disk: cx.target, first: plan.data.0, sectors: plan.data.1 - plan.data.0 + 1 };
    let mut fs = Fs::mount_with(FsPart(part), MountOptions { read_only: true, ..MountOptions::default() })?;
    fs.stat(config::PATH)?;
    Ok(())
}
