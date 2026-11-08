//! MSI (bez X): zprava (adresa, data) je primo v capability konfiguracniho
//! prostoru. Pouzivaji ho zarizeni bez MSI-X (napr. vetsina radicu AHCI).
//! Ovladac nastavuje jen jednu zpravu (Multiple Message Enable = 0).

use libos::abi::object::MsiMessage;
use libos::{Error, Result};

use crate::{Address, Ecam};

pub const CAP_MSI: u8 = 0x05;

const CTRL_ENABLE: u16 = 1 << 0;
const CTRL_MME_MASK: u16 = 7 << 4;
const CTRL_64BIT: u16 = 1 << 7;
const CTRL_PER_VECTOR_MASK: u16 = 1 << 8;

/// Nastavi a zapne MSI funkce `a` se zpravou `msi`. `NOT_SUPPORTED` =
/// funkce nema capability MSI.
pub fn enable(ecam: &Ecam, a: Address, msi: MsiMessage) -> Result<()> {
    let cap = ecam
        .capabilities(a)
        .into_iter()
        .find(|&(_, id)| id == CAP_MSI)
        .map(|(off, _)| off)
        .ok_or(Error::NotSupported)?;
    if msi.data > u16::MAX as u32 {
        return Err(Error::NotSupported);
    }
    let ctrl = ecam.read16(a, cap + 2);
    // Behem zmeny zpravy vypnout, jedna zprava.
    ecam.write16(a, cap + 2, ctrl & !(CTRL_ENABLE | CTRL_MME_MASK));
    ecam.write32(a, cap + 4, msi.address as u32);
    let data_off = if ctrl & CTRL_64BIT != 0 {
        ecam.write32(a, cap + 8, (msi.address >> 32) as u32);
        cap + 12
    } else {
        if msi.address >> 32 != 0 {
            return Err(Error::NotSupported);
        }
        cap + 8
    };
    // Registr dat je 16bitovy (horni polovina dvojslova je rezerva nebo
    // rozsirena data, ktera nepouzivame).
    ecam.write16(a, data_off, msi.data as u16);
    if ctrl & CTRL_PER_VECTOR_MASK != 0 {
        // Maska vektoru lezi za daty (zarovnana na dvojslovo): odmaskovat.
        ecam.write32(a, data_off + 4, 0);
    }
    ecam.write16(a, cap + 2, (ctrl & !CTRL_MME_MASK) | CTRL_ENABLE);
    Ok(())
}
