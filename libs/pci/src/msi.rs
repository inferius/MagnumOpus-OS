//! MSI (bez X): jedna zprava v konfiguracnim prostoru funkce. Pouzivaji
//! ho zarizeni bez tabulky MSI-X (napr. radice Intel HDA).

use libos::abi::object::MsiMessage;
use libos::{Error, Result};

use crate::{Address, Ecam};

pub const CAP_MSI: u8 = 0x05;

const CTRL_ENABLE: u16 = 1 << 0;
/// Pocet povolenych zprav (MME, bity 6:4): 0 = jedna.
const CTRL_MME: u16 = 7 << 4;
const CTRL_64BIT: u16 = 1 << 7;
const CTRL_MASKABLE: u16 = 1 << 8;

/// Nastavi a zapne MSI funkce se zpravou `msi` (jen jedna zprava).
/// `NOT_SUPPORTED` = funkce MSI nema.
pub fn enable(ecam: &Ecam, a: Address, msi: MsiMessage) -> Result<()> {
    let cap = ecam
        .capabilities(a)
        .into_iter()
        .find(|&(_, id)| id == CAP_MSI)
        .map(|(off, _)| off)
        .ok_or(Error::NotSupported)?;
    let ctrl = ecam.read16(a, cap + 2);
    // Behem nastavovani vypnuto (zarizeni nesmi poslat rozpracovanou zpravu).
    ecam.write16(a, cap + 2, ctrl & !(CTRL_ENABLE | CTRL_MME));
    ecam.write32(a, cap + 4, msi.address as u32);
    let (data_off, mask_off) = if ctrl & CTRL_64BIT != 0 {
        ecam.write32(a, cap + 8, (msi.address >> 32) as u32);
        (cap + 12, cap + 16)
    } else {
        if msi.address >> 32 != 0 {
            return Err(Error::NotSupported);
        }
        (cap + 8, cap + 12)
    };
    // Data jsou 16bitova; horni polovina dvojslova je rezervovana.
    ecam.write16(a, data_off, msi.data as u16);
    if ctrl & CTRL_MASKABLE != 0 {
        ecam.write32(a, mask_off, 0);
    }
    ecam.write16(a, cap + 2, (ctrl & !CTRL_MME) | CTRL_ENABLE);
    Ok(())
}

/// Vypne MSI funkce (pred ukoncenim ovladace).
pub fn disable(ecam: &Ecam, a: Address) {
    if let Some((cap, _)) = ecam.capabilities(a).into_iter().find(|&(_, id)| id == CAP_MSI) {
        let ctrl = ecam.read16(a, cap + 2);
        ecam.write16(a, cap + 2, ctrl & !CTRL_ENABLE);
    }
}
