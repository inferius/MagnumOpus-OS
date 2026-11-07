//! Stavba tabulek stranek pro aarch64 (granule 4 KiB, 4 urovne, 48bitove
//! adresy).
//!
//! Rozdeleni indexu je stejne jako na x86_64 (9 bitu na uroven), lisi se jen
//! format polozek. Loader stavi jednu korenovou tabulku a nahraje ji do
//! TTBR0_EL1 (identita, spodni polovina) i TTBR1_EL1 (prima mapa a jadro,
//! horni polovina): pruchod pres TTBR0 pouziva jen indexy 0..256 korenove
//! tabulky, pres TTBR1 jen 256..512, takze se navzajem neovlivni.
//!
//! Tabulky se zapisuji pres identicke mapovani, ktere UEFI garantuje.

use boot_info::aarch64::{ATTR_DEVICE, ATTR_NORMAL};
use uefi::boot::{self, AllocateType, MemoryType};

const VALID: u64 = 1 << 0;
/// Na urovnich 0-2: tabulka (jinak blok); na urovni 3: stranka.
const TABLE_OR_PAGE: u64 = 1 << 1;
const AP_RO: u64 = 1 << 7;
const SH_INNER: u64 = 3 << 8;
const AF: u64 = 1 << 10;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

pub const SIZE_4K: u64 = 0x1000;
pub const SIZE_2M: u64 = 0x20_0000;

/// Prava mapovane stranky.
#[derive(Clone, Copy)]
pub struct Flags {
    pub writable: bool,
    pub executable: bool,
    /// Registry zarizeni (Device-nGnRE, nikdy spustitelne).
    pub device: bool,
}

pub struct PageTables {
    root: u64,
    memory_type: MemoryType,
}

impl PageTables {
    /// Vytvori prazdnou korenovou tabulku. `_nx` je kvuli shode s x86
    /// (zakaz spousteni ma aarch64 vzdy).
    pub fn new(memory_type: MemoryType, _nx: bool) -> uefi::Result<Self> {
        let root = alloc_table(memory_type)?;
        Ok(Self { root, memory_type })
    }

    /// Fyzicka adresa korenove tabulky (hodnota pro TTBR0/TTBR1).
    pub fn root(&self) -> u64 {
        self.root
    }

    fn leaf_bits(flags: Flags) -> u64 {
        let mut bits = VALID | AF;
        if flags.device {
            // Zarizeni: bez cache, bez spousteni (spekulativni nacitani
            // instrukci z registru zarizeni je zakazane).
            bits |= ATTR_DEVICE << 2 | PXN | UXN;
        } else {
            bits |= ATTR_NORMAL << 2 | SH_INNER | UXN;
            if !flags.executable {
                bits |= PXN;
            }
        }
        if !flags.writable {
            bits |= AP_RO;
        }
        bits
    }

    /// Vrati ukazatel na polozku tabulky urovne `level` (0 = koren,
    /// 3 = stranky) pro adresu `virt`; chybejici mezilehle tabulky alokuje.
    fn entry(&mut self, virt: u64, level: u32) -> uefi::Result<*mut u64> {
        let mut table = self.root;
        for l in 0..level {
            let e = entry_ptr(table, virt, l);
            // SAFETY: `e` ukazuje do tabulky alokovane touto strukturou,
            // dostupne pres identicke mapovani firmwaru.
            let val = unsafe { *e };
            if val & VALID == 0 {
                let new = alloc_table(self.memory_type)?;
                // SAFETY: viz vyse.
                unsafe { *e = new | VALID | TABLE_OR_PAGE };
                table = new;
            } else {
                assert!(
                    val & TABLE_OR_PAGE != 0,
                    "konflikt bloku a 4K stranky na {virt:#x}"
                );
                table = val & ADDR_MASK;
            }
        }
        Ok(entry_ptr(table, virt, level))
    }

    /// Namapuje jednu 4 KiB stranku. Pokud uz je namapovana na stejnou
    /// fyzickou adresu, prava se sjednoti (povolnejsi vyhrava).
    pub fn map_4k(&mut self, virt: u64, phys: u64, flags: Flags) -> uefi::Result {
        let bits = Self::leaf_bits(flags) | TABLE_OR_PAGE;
        let e = self.entry(virt, 3)?;
        // SAFETY: `e` je platna polozka posledni urovne vracena `entry`.
        unsafe {
            let old = *e;
            *e = if old & VALID != 0 {
                assert_eq!(
                    old & ADDR_MASK,
                    phys,
                    "4K stranka {virt:#x} uz je namapovana jinam"
                );
                // Zapis: povoleny, kdyz ho povoluje jedno z mapovani (AP_RO
                // AND); spousteni: povolene, kdyz jedno z nich (PXN AND).
                (old | bits) & !(AP_RO | PXN) | (old & bits & (AP_RO | PXN))
            } else {
                phys | bits
            };
        }
        Ok(())
    }

    /// Namapuje jeden 2 MiB blok.
    pub fn map_2m(&mut self, virt: u64, phys: u64, flags: Flags) -> uefi::Result {
        let bits = Self::leaf_bits(flags);
        let e = self.entry(virt, 2)?;
        // SAFETY: `e` je platna polozka urovne 2 vracena `entry`.
        unsafe {
            let old = *e;
            assert!(
                old & VALID == 0 || old & TABLE_OR_PAGE == 0,
                "konflikt bloku a 4K stranky na {virt:#x}"
            );
            // Prekryv (zarovnani na 2 MiB): bezna pamet ma prednost pred
            // zarizenim, prava se sjednoti jako u `map_4k`.
            *e = if old & VALID != 0 && !flags.device {
                (phys | bits) & !(AP_RO | PXN) | (old & bits & (AP_RO | PXN))
            } else if old & VALID != 0 {
                old
            } else {
                phys | bits
            };
        }
        Ok(())
    }

    /// Namapuje fyzicky rozsah na `phys + virt_offset` 2 MiB bloky.
    /// Rozsah se zarovna ven na hranice 2 MiB, prekryvy jsou neskodne.
    pub fn map_range_2m(
        &mut self,
        virt_offset: u64,
        phys: u64,
        len: u64,
        flags: Flags,
    ) -> uefi::Result {
        let start = phys & !(SIZE_2M - 1);
        let end = (phys + len).next_multiple_of(SIZE_2M);
        let mut p = start;
        while p < end {
            self.map_2m(p.wrapping_add(virt_offset), p, flags)?;
            p += SIZE_2M;
        }
        Ok(())
    }
}

/// Polozka tabulky pro adresu `virt` na urovni `level` (0 = koren).
fn entry_ptr(table: u64, virt: u64, level: u32) -> *mut u64 {
    let index = (virt >> (39 - 9 * level)) & 0x1FF;
    (table + index * 8) as *mut u64
}

fn alloc_table(memory_type: MemoryType) -> uefi::Result<u64> {
    let ptr = boot::allocate_pages(AllocateType::AnyPages, memory_type, 1)?;
    // SAFETY: prave alokovana stranka o velikosti 4 KiB, nikdo jiny ji nepouziva.
    unsafe { ptr.as_ptr().write_bytes(0, SIZE_4K as usize) };
    Ok(ptr.as_ptr() as u64)
}
