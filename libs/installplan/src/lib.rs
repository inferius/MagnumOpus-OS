//! Instalator: kroky, volby disku, plan oddilu a nahled zmen (cista logika
//! bez I/O, testy na hostiteli). Pouziva ji sluzba `installd` i textovy
//! pruvodce (`install` v shellu) a pozdeji graficke rozhrani, takze kroky,
//! volby a texty jsou na jednom miste (`design/instalace.html`).
//!
//! Rozlozeni noveho systemu: nase ESP ([`ESP_BYTES`], jmeno
//! `MagnumOpus EFI`) a datovy oddil se souborovym systemem (zbytek, aspon
//! [`MIN_DATA_BYTES`]). Cizi oddily se nikdy neprepisuji; rezim "cely
//! disk" prepise jen tabulku oddilu (po opsani nazvu disku).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod answers;
pub mod config;
pub mod proto;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diskguard::gpt::{self, Ends, NewPartition, Table};
use diskguard::install::{OUR_ESP_LABEL, add_partitions, free_ranges, is_installer_owned};
use diskguard::{ESP_TYPE, Guid, OUR_LABEL, OUR_TYPE};

/// Velikost nasi ESP: zavadec, jadro a initfs ve dvou slotech A/B pro
/// aktualizace (2 x ~45 MB) a rezerva na rust.
pub const ESP_BYTES: u64 = 512 << 20;
/// Nejmensi datovy oddil.
pub const MIN_DATA_BYTES: u64 = 8 << 30;
/// Zarovnani oddilu (1 MiB, obvykle pro SSD a flash).
pub const ALIGN_BYTES: u64 = 1 << 20;

/// Kroky instalatoru v poradi (shodne s navrhem).
pub const STEPS: [&str; 8] = [
    "Jazyk a klávesnice",
    "Kontrola počítače",
    "Kam nainstalovat",
    "Náhled změn",
    "Tvůj účet",
    "Zabezpečení",
    "Souhrn",
    "Průběh instalace",
];

/// Text kroku Zabezpečení (šifrování zatím není).
pub const NO_ENCRYPTION: &str =
    "Disk nebude šifrovaný - šifrování disku se připravuje a v této verzi ho nejde zapnout.";

/// Cílový disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskInfo {
    /// Číslo disku v seznamu installd.
    pub index: u32,
    /// Model (z ovladače), u NVMe a SATA jméno disku.
    pub model: String,
    pub sectors: u64,
    pub sector_size: usize,
    /// Disk, ze kterého běží živé USB (na něj se neinstaluje).
    pub live: bool,
}

impl DiskInfo {
    pub fn bytes(&self) -> u64 {
        self.sectors * self.sector_size as u64
    }

    fn sec(&self, bytes: u64) -> u64 {
        bytes / self.sector_size as u64
    }
}

/// Způsob instalace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Smaže tabulku oddílů a nainstaluje jen MagnumOpus.
    WholeDisk,
    /// Do nejvetšího volného místa vedle jiného systému.
    FreeSpace,
    /// Disk už má MagnumOpus: nová ESP a systém, data zůstanou.
    Reinstall,
    /// Zmenšení oddílu Windows (zatím ne).
    ShrinkOther,
}

impl Mode {
    pub const fn key(self) -> &'static str {
        match self {
            Mode::WholeDisk => "cely-disk",
            Mode::FreeSpace => "volne-misto",
            Mode::Reinstall => "preinstalovat",
            Mode::ShrinkOther => "zmensit",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        [Mode::WholeDisk, Mode::FreeSpace, Mode::Reinstall, Mode::ShrinkOther].into_iter().find(|m| m.key() == k)
    }
}

/// Volba v kroku "Kam nainstalovat".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub mode: Mode,
    /// Zašedlá volba = `false` (důvod v `detail`).
    pub available: bool,
    pub title: String,
    pub detail: String,
}

/// Velikost pro lidi (desítkově jako výrobci disků: 512 MB, 511 GB).
pub fn human(bytes: u64) -> String {
    const UNITS: [(&str, u64); 3] = [("TB", 1_000_000_000_000), ("GB", 1_000_000_000), ("MB", 1_000_000)];
    for (u, n) in UNITS {
        if bytes >= n {
            return format!("{} {u}", (bytes + n / 2) / n);
        }
    }
    format!("{} kB", bytes.div_ceil(1000))
}

/// Jméno oddílu z GPT (UTF-16).
pub fn entry_name(e: &gpt::Entry) -> String {
    let end = e.name.iter().position(|&u| u == 0).unwrap_or(e.name.len());
    String::from_utf16_lossy(&e.name[..end])
}

/// Popis existujícího oddílu pro náhled.
fn describe_entry(e: &gpt::Entry, ss: usize) -> String {
    let size = human((e.last - e.first + 1) * ss as u64);
    let name = entry_name(e);
    let kind = if e.type_guid == ESP_TYPE {
        "EFI"
    } else if e.type_guid == OUR_TYPE {
        "MagnumOpus"
    } else {
        "oddíl"
    };
    if name.is_empty() { format!("{kind} ({size})") } else { format!("{kind} {name} ({size})") }
}

/// Nejvetší volný úsek (sektory, včetně).
fn largest_free(disk: &DiskInfo, t: &Table) -> Option<(u64, u64)> {
    free_ranges(t, disk.sec(ALIGN_BYTES), 1).into_iter().max_by_key(|&(a, b)| b - a)
}

/// Naše oddíly na disku (ESP, data), pokud jsou oba.
fn ours(t: &Table) -> Option<(gpt::Entry, gpt::Entry)> {
    let esp = t.entries().find(|e| !e.is_empty() && is_installer_owned(e) && e.type_guid == ESP_TYPE)?;
    let data = t.entries().find(|e| !e.is_empty() && is_installer_owned(e) && e.type_guid == OUR_TYPE)?;
    Some((esp, data))
}

fn needed(disk: &DiskInfo) -> u64 {
    disk.sec(ESP_BYTES + MIN_DATA_BYTES)
}

/// Volby pro disk (`table` = jeho GPT, `None` = prázdný nebo bez GPT).
pub fn choices(disk: &DiskInfo, table: Option<&Table>) -> Vec<Choice> {
    let mut out = Vec::new();
    let foreign = table.is_some_and(|t| t.entries().any(|e| !e.is_empty() && !is_installer_owned(&e)));
    if let Some(t) = table
        && let Some((_, data)) = ours(t)
    {
        out.push(Choice {
            mode: Mode::Reinstall,
            available: true,
            title: String::from("Přeinstalovat systém, data ponechat"),
            detail: format!(
                "Nový zavaděč a systém do oddílu EFI MagnumOpus. Oddíl MagnumOpus ({}) s účty a soubory zůstane.",
                human((data.last - data.first + 1) * disk.sector_size as u64)
            ),
        });
    }
    if foreign {
        let t = table.unwrap();
        let free = largest_free(disk, t).map_or(0, |(a, b)| b - a + 1);
        let ok = free >= needed(disk);
        out.push(Choice {
            mode: Mode::FreeSpace,
            available: ok,
            title: String::from("Vedle jiného systému (volné místo)"),
            detail: if ok {
                format!(
                    "Použije volné místo {} na disku. Jiné oddíly zůstanou beze změny. Při startu si vybereš systém.",
                    human(free * disk.sector_size as u64)
                )
            } else {
                format!(
                    "Na disku není dost volného místa (je {}, potřeba aspoň {}). Uvolni místo zmenšením oddílu v druhém systému.",
                    human(free * disk.sector_size as u64),
                    human(ESP_BYTES + MIN_DATA_BYTES)
                )
            },
        });
        out.push(Choice {
            mode: Mode::ShrinkOther,
            available: false,
            title: String::from("Zmenšit oddíl Windows a nainstalovat vedle"),
            detail: String::from(
                "Zatím není podporováno - zmenši C: ve Správě disků Windows (pravým tlačítkem na C:, Zmenšit svazek) a vyber volné místo.",
            ),
        });
    }
    let big = disk.sectors >= needed(disk) + disk.sec(4 << 20);
    out.push(Choice {
        mode: Mode::WholeDisk,
        available: big,
        title: String::from(if foreign || table.is_some_and(|t| ours(t).is_some()) {
            "Smazat celý disk a nainstalovat jen MagnumOpus"
        } else {
            "Celý disk pro MagnumOpus"
        }),
        detail: if big {
            String::from("Smaže všechno na disku včetně ostatních systémů a dat.")
        } else {
            format!("Disk je moc malý (je {}, potřeba aspoň {}).", human(disk.bytes()), human(ESP_BYTES + MIN_DATA_BYTES))
        },
    });
    out
}

/// Řádek náhledu změn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// Zůstane beze změny.
    Keep(String),
    /// Vznikne (nebo se přepíše jen náš oddíl).
    New(String),
    /// Smaže se.
    Erase(String),
}

impl Line {
    pub fn text(&self) -> String {
        match self {
            Line::Keep(t) => format!("  zůstane   {t}"),
            Line::New(t) => format!("  + nový     {t}"),
            Line::Erase(t) => format!("  - smaže se {t}"),
        }
    }
}

/// Jak uživatel potvrdí zápis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Confirm {
    /// Opsat název disku (mazání celého disku).
    TypeName(String),
    /// Zaškrtnout "rozumím" (jiný systém zůstává, ale mění se tabulka).
    Agree,
}

/// Co udělat s tabulkou oddílů.
#[derive(Debug)]
pub enum GptWrite {
    /// Nová tabulka (celý disk).
    New(Ends),
    /// Existující tabulka + naše položky.
    Add(Ends),
    /// Beze změny (přeinstalace).
    Keep,
}

/// Plán instalace.
#[derive(Debug)]
pub struct Plan {
    pub mode: Mode,
    /// Naše ESP a datový oddíl (sektory, včetně).
    pub esp: (u64, u64),
    pub data: (u64, u64),
    pub gpt: GptWrite,
    /// Naformátovat datový oddíl (ne u přeinstalace).
    pub format_data: bool,
    /// Na disku zůstane jiný systém (otázka na výchozí systém).
    pub other_os: bool,
    pub preview: Vec<Line>,
    pub confirm: Confirm,
}

fn new_parts(esp: (u64, u64), data: (u64, u64), guids: &[Guid; 2]) -> [NewPartition<'static>; 2] {
    [
        NewPartition { type_guid: ESP_TYPE, guid: guids[0], first: esp.0, last: esp.1, attrs: 0, name: OUR_ESP_LABEL },
        NewPartition { type_guid: OUR_TYPE, guid: guids[1], first: data.0, last: data.1, attrs: 0, name: OUR_LABEL },
    ]
}

/// Plán pro disk a zvolený způsob. `guids` = náhodné GUID disku a dvou
/// nových oddílů.
pub fn plan(disk: &DiskInfo, table: Option<&Table>, mode: Mode, guids: [Guid; 3]) -> Result<Plan, String> {
    let ss = disk.sector_size;
    if disk.live {
        return Err(String::from("na disk, ze kterého systém právě běží (živé USB), instalovat nejde"));
    }
    let esp_sec = disk.sec(ESP_BYTES);
    let align = disk.sec(ALIGN_BYTES);
    let size = |a: u64, b: u64| human((b - a + 1) * ss as u64);
    match mode {
        Mode::ShrinkOther => Err(String::from(
            "zmenšení oddílu Windows zatím není podporováno - zmenši C: ve Správě disků Windows",
        )),
        Mode::WholeDisk => {
            let first = gpt::first_usable(ss).next_multiple_of(align);
            let last = (gpt::last_usable(disk.sectors, ss) + 1) / align * align - 1;
            let esp = (first, first + esp_sec - 1);
            let data = (esp.1 + 1, last);
            if data.1 < data.0 || (data.1 - data.0 + 1) * (ss as u64) < MIN_DATA_BYTES {
                return Err(format!("disk je moc malý (potřeba aspoň {})", human(ESP_BYTES + MIN_DATA_BYTES)));
            }
            let mut preview: Vec<Line> = table
                .map(|t| t.entries().filter(|e| !e.is_empty()).map(|e| Line::Erase(format!("{} a všechna data na něm", describe_entry(&e, ss)))).collect())
                .unwrap_or_default();
            preview.push(Line::New(format!("EFI ({}) - jen zavaděč MagnumOpus", size(esp.0, esp.1))));
            preview.push(Line::New(format!("MagnumOpus {}", size(data.0, data.1))));
            let ends = gpt::build(disk.sectors, ss, guids[0], &new_parts(esp, data, &[guids[1], guids[2]]))
                .map_err(String::from)?;
            Ok(Plan {
                mode,
                esp,
                data,
                gpt: GptWrite::New(ends),
                format_data: true,
                other_os: false,
                preview,
                confirm: Confirm::TypeName(disk.model.clone()),
            })
        }
        Mode::FreeSpace => {
            let t = table.ok_or_else(|| String::from("disk nemá tabulku oddílů GPT"))?;
            let (a, b) = largest_free(disk, t).ok_or_else(|| String::from("na disku není volné místo"))?;
            if b - a + 1 < needed(disk) {
                return Err(format!("málo volného místa (potřeba aspoň {})", human(ESP_BYTES + MIN_DATA_BYTES)));
            }
            let esp = (a, a + esp_sec - 1);
            let data = (esp.1 + 1, b);
            let mut preview: Vec<Line> =
                t.entries().filter(|e| !e.is_empty()).map(|e| Line::Keep(describe_entry(&e, ss))).collect();
            preview.push(Line::New(format!("EFI MagnumOpus ({}) - vlastní zavaděč, EFI jiného systému se nemění", size(esp.0, esp.1))));
            preview.push(Line::New(format!("MagnumOpus {}", size(data.0, data.1))));
            let ends = add_partitions(t, disk.sectors, ss, &new_parts(esp, data, &[guids[1], guids[2]])).map_err(String::from)?;
            Ok(Plan { mode, esp, data, gpt: GptWrite::Add(ends), format_data: true, other_os: true, preview, confirm: Confirm::Agree })
        }
        Mode::Reinstall => {
            let t = table.ok_or_else(|| String::from("disk nemá tabulku oddílů GPT"))?;
            let (esp, data) = ours(t).ok_or_else(|| String::from("na disku není MagnumOpus"))?;
            let mut preview = Vec::new();
            for e in t.entries().filter(|e| !e.is_empty()) {
                if e.first == esp.first {
                    preview.push(Line::New(format!("{} - přepíše se zavaděč a systém", describe_entry(&e, ss))));
                } else if e.first == data.first {
                    preview.push(Line::Keep(format!("{} - účty a soubory zůstanou", describe_entry(&e, ss))));
                } else {
                    preview.push(Line::Keep(describe_entry(&e, ss)));
                }
            }
            let other = t.entries().any(|e| !e.is_empty() && !is_installer_owned(&e));
            Ok(Plan {
                mode,
                esp: (esp.first, esp.last),
                data: (data.first, data.last),
                gpt: GptWrite::Keep,
                format_data: false,
                other_os: other,
                preview,
                confirm: Confirm::Agree,
            })
        }
    }
}

/// Text náhledu (textový průvodce, log).
pub fn preview_text(p: &Plan) -> String {
    let mut s = String::from("Co se s diskem stane (zatím se nic nezměnilo):\n");
    for l in &p.preview {
        s.push_str(&l.text());
        s.push('\n');
    }
    s
}

/// Krok průběhu instalace (texty pro hlášení průběhu).
pub const PROGRESS: [&str; 7] = [
    "Zapisuji tabulku oddílů",
    "Formátuji oddíl EFI",
    "Kopíruji zavaděč a systém",
    "Vytvářím souborový systém",
    "Ukládám nastavení",
    "Vytvářím účet",
    "Ověřuji instalaci",
];

/// Kontrola jména účtu (stejná pravidla jako služba účtů): `a-z`, `0-9`,
/// `-`, `_`, 1-32 znaků, začíná písmenem.
pub fn valid_user_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 32
        && n.as_bytes()[0].is_ascii_lowercase()
        && n.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

/// Síla hesla podle návrhu: aspoň 10 znaků nebo 4 slova.
pub fn password_ok(p: &str) -> bool {
    p.chars().count() >= 10 || p.split_whitespace().filter(|w| !w.is_empty()).count() >= 4
}

/// Výchozí systém při startu.
pub fn default_os_text(magnum: bool) -> String {
    if magnum { String::from("MagnumOpus") } else { String::from("druhý systém (MagnumOpus z nabídky firmwaru)") }
}

#[cfg(test)]
mod tests;
