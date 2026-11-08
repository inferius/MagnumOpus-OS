//! Vypis disku hostitele a otevreni disku pro zapis (Windows, Linux).
//!
//! - Windows: PowerShell `Get-Disk` (cislo, model, velikost, sbernice,
//!   `IsSystem`, `IsBoot`) a `Get-Partition` (pismena). Pred zapisem
//!   `Clear-Disk` odstrani oddily (Windows jinak zapis do sektoru
//!   pripojeneho svazku zablokuje); zapis jde do `\\.\PhysicalDriveN`
//!   a potrebuje prava spravce.
//! - Linux: `lsblk` (cesta, model, velikost, sbernice, priznak RM,
//!   pripojne body vcetne oddilu). Disk s pripojenym oddilem xtask
//!   odmitne (nejdriv `umount`); zapis do `/dev/sdX` potrebuje root.

use std::process::Command;

use super::hostdisk::HostDisk;
use crate::Result;

/// Disky hostitele.
pub fn list() -> Result<Vec<HostDisk>> {
    if cfg!(windows) {
        list_windows()
    } else if cfg!(target_os = "linux") {
        list_linux()
    } else {
        Err("vypis disku umi xtask jen na Windows a Linuxu (jinde dd rucne, viz navod)".into())
    }
}

fn powershell(script: &str) -> Result<String> {
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()?;
    if !out.status.success() {
        return Err(format!("PowerShell selhal: {}", String::from_utf8_lossy(&out.stderr).trim()).into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Radek na disk: `cislo|model|velikost|sbernice|system|boot|pismena`.
const WINDOWS_LIST: &str = r#"Get-Disk | ForEach-Object {
  $l = (Get-Partition -DiskNumber $_.Number -ErrorAction SilentlyContinue | Where-Object DriveLetter | ForEach-Object { "$($_.DriveLetter):" }) -join ','
  "{0}|{1}|{2}|{3}|{4}|{5}|{6}" -f $_.Number, ($_.FriendlyName -replace '\|',' '), $_.Size, $_.BusType, $_.IsSystem, $_.IsBoot, $l
}"#;

fn list_windows() -> Result<Vec<HostDisk>> {
    Ok(parse_windows(&powershell(WINDOWS_LIST)?))
}

/// Rozbor vystupu [`WINDOWS_LIST`].
pub fn parse_windows(text: &str) -> Vec<HostDisk> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.trim().split('|').collect();
            if f.len() < 7 {
                return None;
            }
            let number: u32 = f[0].parse().ok()?;
            let yes = |s: &str| s.eq_ignore_ascii_case("true");
            Some(HostDisk {
                id: number.to_string(),
                path: format!(r"\\.\PhysicalDrive{number}"),
                model: f[1].trim().to_string(),
                size: f[2].parse().ok()?,
                bus: f[3].trim().to_string(),
                removable: false,
                system: yes(f[4]) || yes(f[5]),
                mounted: f[6].split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
            })
        })
        .collect()
}

fn list_linux() -> Result<Vec<HostDisk>> {
    let out = Command::new("lsblk")
        .args(["-b", "-n", "-P", "-o", "NAME,PATH,TYPE,SIZE,RM,TRAN,MODEL,MOUNTPOINT,PKNAME"])
        .output()?;
    if !out.status.success() {
        return Err("lsblk selhal".into());
    }
    Ok(parse_lsblk(&String::from_utf8_lossy(&out.stdout)))
}

/// Pole `KEY="hodnota"` z radku `lsblk -P`.
fn lsblk_fields(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = line.trim();
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.push((key, after[..end].to_string()));
        rest = &after[end + 1..];
    }
    out
}

/// Pripojne body, ktere znamenaji system hostitele.
fn system_mount(m: &str) -> bool {
    matches!(m, "/" | "/boot" | "/boot/efi" | "/efi" | "/usr" | "/var" | "/home" | "[SWAP]")
}

/// Rozbor vystupu `lsblk -b -n -P -o NAME,PATH,TYPE,SIZE,RM,TRAN,MODEL,MOUNTPOINT,PKNAME`.
pub fn parse_lsblk(text: &str) -> Vec<HostDisk> {
    let rows: Vec<Vec<(String, String)>> = text.lines().map(lsblk_fields).collect();
    let get = |r: &[(String, String)], k: &str| r.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone()).unwrap_or_default();
    let mut disks: Vec<HostDisk> = rows
        .iter()
        .filter(|r| get(r, "TYPE") == "disk")
        .map(|r| {
            let path = get(r, "PATH");
            let m = get(r, "MOUNTPOINT");
            HostDisk {
                id: path.clone(),
                path,
                model: get(r, "MODEL").trim().to_string(),
                size: get(r, "SIZE").parse().unwrap_or(0),
                bus: get(r, "TRAN"),
                removable: get(r, "RM") == "1",
                system: system_mount(&m),
                mounted: if m.is_empty() { Vec::new() } else { vec![m] },
            }
        })
        .collect();
    // Oddily (a LVM, LUKS ...) patri disku podle PKNAME; i vnorene.
    let parent_disk = |mut name: String| -> Option<String> {
        for _ in 0..8 {
            let r = rows.iter().find(|r| get(r, "NAME") == name)?;
            if get(r, "TYPE") == "disk" {
                return Some(get(r, "PATH"));
            }
            name = get(r, "PKNAME");
        }
        None
    };
    for r in rows.iter().filter(|r| get(r, "TYPE") != "disk") {
        let m = get(r, "MOUNTPOINT");
        if m.is_empty() {
            continue;
        }
        if let Some(p) = parent_disk(get(r, "NAME"))
            && let Some(d) = disks.iter_mut().find(|d| d.path == p)
        {
            d.system |= system_mount(&m);
            d.mounted.push(m);
        }
    }
    disks
}

/// Pripravi disk k zapisu a otevre ho. Windows: odstrani oddily
/// (`Clear-Disk`), Linux: odmitne disk s pripojenym oddilem.
pub fn open_for_write(d: &HostDisk) -> Result<std::fs::File> {
    if cfg!(windows) {
        let n: u32 = d.id.parse()?;
        // Disk bez tabulky oddilu (RAW) Clear-Disk odmitne; neni co mazat.
        let script = format!(
            "$d = Get-Disk -Number {n}; if ($d.PartitionStyle -ne 'RAW') {{ Clear-Disk -Number {n} -RemoveData -RemoveOEM -Confirm:$false }}"
        );
        powershell(&script).map_err(|e| format!("{e}\n(zapis na disk potrebuje terminal spusteny jako spravce)"))?;
    } else if !d.mounted.is_empty() {
        return Err(format!(
            "na disku jsou pripojene oddily ({}): nejdriv je odpoj (umount)",
            d.mounted.join(", ")
        )
        .into());
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&d.path)
        .map_err(|e| {
            let hint = if cfg!(windows) { "spust terminal jako spravce" } else { "spust pres sudo" };
            format!("disk {} nejde otevrit pro zapis: {e} ({hint})", d.path).into()
        })
}

/// Po zapisu: Windows znovu nacte tabulku oddilu (ESP dostane pismeno),
/// Linux `blockdev --rereadpt`.
pub fn rescan(d: &HostDisk) {
    if cfg!(windows) {
        let _ = powershell(&format!("Update-Disk -Number {}", d.id));
    } else {
        let _ = Command::new("blockdev").args(["--rereadpt", &d.path]).status();
    }
}
