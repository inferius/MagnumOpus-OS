//! MSI-X: zarizeni posila preruseni zapisem zpravy (adresa, data) do pameti;
//! tabulka zprav lezi v jednom z BARu zarizeni.

use libos::abi::object::MsiMessage;
use libos::abi::vm::{MAP_READ, MAP_WRITE};
use libos::hw::Resource;
use libos::{Error, Mapping, Result};

use crate::{Address, CAP_MSIX, Ecam};

const ENTRY_SIZE: u64 = 16;
const CTRL_ENABLE: u16 = 1 << 15;
const CTRL_FUNCTION_MASK: u16 = 1 << 14;

/// Namapovana tabulka MSI-X jedne funkce.
pub struct MsixTable {
    map: Mapping,
    /// Offset tabulky uvnitr mapovani (BAR nemusi zacinat na strance).
    base: u64,
    entries: u16,
    cap: u16,
    address: Address,
}

impl MsixTable {
    /// Najde capability MSI-X a namapuje tabulku.
    pub fn open(ecam: &Ecam, a: Address, resource: &Resource) -> Result<Self> {
        let cap = ecam
            .capabilities(a)
            .into_iter()
            .find(|&(_, id)| id == CAP_MSIX)
            .map(|(off, _)| off)
            .ok_or(Error::NotSupported)?;
        let ctrl = ecam.read16(a, cap + 2);
        let entries = (ctrl & 0x7FF) + 1;
        let table = ecam.read32(a, cap + 4);
        let bar = ecam
            .memory_bar(a, (table & 7) as u8)
            .ok_or(Error::NotSupported)?;
        let phys = bar.phys + (table & !7) as u64;
        let page_size = libos::vm::page_size();
        let page = phys & !(page_size - 1);
        let len = (phys - page + entries as u64 * ENTRY_SIZE).next_multiple_of(page_size);
        let vmo = resource.device_memory(page, len)?;
        let map = Mapping::new(&vmo, 0, len, MAP_READ | MAP_WRITE)?;
        Ok(Self {
            map,
            base: phys - page,
            entries,
            cap,
            address: a,
        })
    }

    pub fn entries(&self) -> u16 {
        self.entries
    }

    /// Nastavi zpravu pro polozku `index` a odmaskuje ji.
    pub fn set(&self, index: u16, msi: MsiMessage) -> Result<()> {
        if index >= self.entries {
            return Err(Error::OutOfRange);
        }
        let e = self.base + index as u64 * ENTRY_SIZE;
        self.map.write::<u32>(e, msi.address as u32);
        self.map.write::<u32>(e + 4, (msi.address >> 32) as u32);
        self.map.write::<u32>(e + 8, msi.data);
        self.map.write::<u32>(e + 12, 0); // vector control: odmaskovano
        Ok(())
    }

    /// Zapne MSI-X pro funkci (a zrusi masku cele funkce).
    pub fn enable(&self, ecam: &Ecam) {
        let ctrl = ecam.read16(self.address, self.cap + 2);
        ecam.write16(
            self.address,
            self.cap + 2,
            (ctrl | CTRL_ENABLE) & !CTRL_FUNCTION_MASK,
        );
    }
}
