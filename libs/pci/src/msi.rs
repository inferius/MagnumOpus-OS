//! MSI (bez -X): zprava preruseni (adresa, data) lezi primo v capability
//! konfiguracniho prostoru, jedna zprava na funkci. Potrebuji ho zarizeni
//! bez MSI-X (napr. sitove karty Intel I217-I219 v cipove sade PCH,
//! vetsina radicu AHCI).
//!
//! Rozlozeni capability (id 0x05):
//!
//! | offset | 32bitova adresa | 64bitova adresa |
//! |---|---|---|
//! | +2 | rizeni (u16) | rizeni (u16) |
//! | +4 | adresa | adresa (dolni) |
//! | +8 | data (u16) | adresa (horni) |
//! | +0xC | maska (s maskovanim) | data (u16) |
//! | +0x10 | - | maska (s maskovanim) |

use libos::abi::object::MsiMessage;
use libos::{Error, Result};

use crate::{Address, CAP_MSI, Ecam};

const CTRL_ENABLE: u16 = 1 << 0;
/// Pocet povolenych zprav (bity 6:4); 0 = jedna.
const CTRL_MME_MASK: u16 = 7 << 4;
const CTRL_64BIT: u16 = 1 << 7;
const CTRL_PER_VECTOR_MASK: u16 = 1 << 8;

/// Capability MSI jedne funkce.
pub struct Msi {
    cap: u16,
    address: Address,
}

impl Msi {
    /// Najde capability MSI.
    pub fn find(ecam: &Ecam, a: Address) -> Result<Self> {
        let cap = ecam
            .capabilities(a)
            .into_iter()
            .find(|&(_, id)| id == CAP_MSI)
            .map(|(off, _)| off)
            .ok_or(Error::NotSupported)?;
        Ok(Self { cap, address: a })
    }

    /// Nastavi jedinou zpravu `msi`, odmaskuje ji a zapne MSI.
    /// `NOT_SUPPORTED` = zpravu nejde zapsat (data nad 16 bitu, 64bitova
    /// adresa u funkce jen s 32bitovou adresou); MSI pak zustane vypnute.
    pub fn enable(&self, ecam: &Ecam, msi: MsiMessage) -> Result<()> {
        let (a, cap) = (self.address, self.cap);
        let ctrl = ecam.read16(a, cap + 2);
        if msi.data > u16::MAX as u32 || (ctrl & CTRL_64BIT == 0 && msi.address >> 32 != 0) {
            return Err(Error::NotSupported);
        }
        // Behem zmeny adresy a dat vypnuto (zarizeni nesmi poslat
        // polovicni zpravu).
        ecam.write16(a, cap + 2, ctrl & !(CTRL_ENABLE | CTRL_MME_MASK));
        ecam.write32(a, cap + 4, msi.address as u32);
        let (data, mask) = if ctrl & CTRL_64BIT != 0 {
            ecam.write32(a, cap + 8, (msi.address >> 32) as u32);
            (cap + 0xC, cap + 0x10)
        } else {
            (cap + 8, cap + 0xC)
        };
        ecam.write16(a, data, msi.data as u16);
        if ctrl & CTRL_PER_VECTOR_MASK != 0 {
            ecam.write32(a, mask, 0);
        }
        ecam.write16(a, cap + 2, (ctrl & !CTRL_MME_MASK) | CTRL_ENABLE);
        Ok(())
    }

    /// Vypne MSI (pred koncem ovladace).
    pub fn disable(&self, ecam: &Ecam) {
        let ctrl = ecam.read16(self.address, self.cap + 2);
        ecam.write16(self.address, self.cap + 2, ctrl & !CTRL_ENABLE);
    }
}

/// Najde capability MSI funkce `a` a zapne ji se zpravou `msi`.
/// `NOT_SUPPORTED` = funkce MSI nema nebo zpravu nejde zapsat.
pub fn enable(ecam: &Ecam, a: Address, msi: MsiMessage) -> Result<()> {
    Msi::find(ecam, a)?.enable(ecam, msi)
}
