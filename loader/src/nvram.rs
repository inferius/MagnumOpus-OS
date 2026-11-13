//! Polozka "MagnumOpus" v nabidce firmwaru (NVRAM), jen jednou po
//! instalaci.
//!
//! Instalator zapise na nasi ESP `\os\boot.cfg` s `nvram = "zapsat"`
//! a vychozim systemem. Pri prvnim startu z interniho disku loader (jeste
//! s boot services):
//! 1. zjisti, jestli polozka s popisem "MagnumOpus" uz neni,
//! 2. overi volne misto v promennych (QueryVariableInfo) - plna nebo
//!    chybova NVRAM muze desku zablokovat, proto se pri malem miste nic
//!    nezapisuje,
//! 3. prida `Boot####` (prvni volne cislo) na vlastni cestu zavadece a cislo
//!    vlozi do `BootOrder` na zacatek (vychozi MagnumOpus) nebo na konec,
//! 4. v `boot.cfg` zmeni `nvram` na `hotovo` (dalsi starty uz nic nedelaji,
//!    i kdyz zapis selhal - jen jeden pokus).
//!
//! Cizi polozky se nikdy nemazou ani neprepisuji; meni se jen `BootOrder`
//! (pridani naseho cisla).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use uefi::proto::device_path::LoadedImageDevicePath;
use uefi::runtime::{self, VariableAttributes, VariableVendor};
use uefi::{CStr16, CString16, boot, cstr16};

const BOOT_CFG: &CStr16 = cstr16!("\\os\\boot.cfg");
const DESCRIPTION: &str = "MagnumOpus";
/// Nejmene volneho mista v NVRAM pro zapis (polozka ma ~150 B; rezerva pro
/// firmware, ktery pri skoro plne NVRAM selhava).
const MIN_FREE: u64 = 16 * 1024;

fn attrs() -> VariableAttributes {
    VariableAttributes::NON_VOLATILE | VariableAttributes::BOOTSERVICE_ACCESS | VariableAttributes::RUNTIME_ACCESS
}

/// Provede registraci, je-li o ni pozadano (`boot.cfg`).
pub fn register_once() {
    let fs = || crate::boot_fs();
    let Ok(raw) = fs().read(BOOT_CFG) else { return };
    let text = String::from_utf8_lossy(&raw).into_owned();
    if !text.contains("nvram = \"zapsat\"") {
        return;
    }
    let default_magnum = !text.contains("vychozi = \"jiny\"");
    let result = register(default_magnum);
    match &result {
        Ok(msg) => log::info!("NVRAM: {msg}"),
        Err(e) => log::warn!("NVRAM: polozka MagnumOpus nezapsana: {e} (start z nabidky firmwaru funguje dal)"),
    }
    // Jeden pokus: priste uz nic.
    let done = text.replace("nvram = \"zapsat\"", "nvram = \"hotovo\"");
    if let Err(e) = fs().write(BOOT_CFG, done.as_bytes()) {
        log::warn!("NVRAM: boot.cfg nejde prepsat: {e:?}");
    }
}

fn var_name(n: u16) -> CString16 {
    CString16::try_from(format!("Boot{n:04X}").as_str()).unwrap()
}

/// Popis polozky Boot#### (UTF-16 za atributy a delkou cesty).
fn description(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .get(6..)
        .unwrap_or(&[])
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

fn register(default_magnum: bool) -> Result<String, String> {
    let vendor = VariableVendor::GLOBAL_VARIABLE;
    let order: Vec<u16> = runtime::get_variable_boxed(cstr16!("BootOrder"), &vendor)
        .map(|(b, _)| b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect())
        .unwrap_or_default();
    // Uz je?
    let mut free = None;
    for n in 0..=0xFFFFu16 {
        match runtime::get_variable_boxed(&var_name(n), &vendor) {
            Ok((data, _)) => {
                if description(&data) == DESCRIPTION {
                    return Ok(format!("polozka Boot{n:04X} uz existuje"));
                }
            }
            Err(_) => {
                if free.is_none() {
                    free = Some(n);
                }
                // Cisla jsou obvykle souvisla od 0: po 32 volnych dal nehledat.
                if n > free.unwrap() + 32 {
                    break;
                }
            }
        }
    }
    let n = free.ok_or("neni volne cislo Boot####")?;
    let info = runtime::query_variable_info(attrs()).map_err(|e| format!("QueryVariableInfo: {e:?}"))?;
    if info.remaining_variable_storage_size < MIN_FREE {
        return Err(format!("malo mista v NVRAM ({} B volno)", info.remaining_variable_storage_size));
    }
    // Cesta zavadece (zarizeni + soubor) z LoadedImageDevicePath.
    let path = boot::open_protocol_exclusive::<LoadedImageDevicePath>(boot::image_handle())
        .map_err(|e| format!("cesta zavadece: {e:?}"))?;
    let dp = path.as_bytes();
    // EFI_LOAD_OPTION: atributy (ACTIVE), delka cesty, popis, cesta.
    let mut opt = Vec::new();
    opt.extend_from_slice(&1u32.to_le_bytes());
    opt.extend_from_slice(&(dp.len() as u16).to_le_bytes());
    for u in DESCRIPTION.encode_utf16().chain(core::iter::once(0)) {
        opt.extend_from_slice(&u.to_le_bytes());
    }
    opt.extend_from_slice(dp);
    runtime::set_variable(&var_name(n), &vendor, attrs(), &opt).map_err(|e| format!("Boot{n:04X}: {e:?}"))?;
    let mut new_order = order.clone();
    new_order.retain(|&x| x != n);
    if default_magnum {
        new_order.insert(0, n);
    } else {
        new_order.push(n);
    }
    let bytes: Vec<u8> = new_order.iter().flat_map(|x| x.to_le_bytes()).collect();
    runtime::set_variable(cstr16!("BootOrder"), &vendor, attrs(), &bytes).map_err(|e| format!("BootOrder: {e:?}"))?;
    Ok(format!(
        "zapsana polozka Boot{n:04X} MagnumOpus ({})",
        if default_magnum { "vychozi" } else { "na konci poradi" }
    ))
}
