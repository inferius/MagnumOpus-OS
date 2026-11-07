//! Stavba 4-urovnovych tabulek stranek pro kernel.
//!
//! Tabulky se alokuji pres UEFI boot services a zapisuji se pres identicke
//! mapovani, ktere UEFI na x86_64 garantuje.

use uefi::boot::{self, AllocateType, MemoryType};

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const HUGE: u64 = 1 << 7;
const NO_EXECUTE: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

pub const SIZE_4K: u64 = 0x1000;
pub const SIZE_2M: u64 = 0x20_0000;

/// Prava mapovane stranky.
#[derive(Clone, Copy)]
pub struct Flags {
    pub writable: bool,
    pub executable: bool,
    /// Registry zarizeni. Na x86 se typ cache ridi MTRR firmwaru, priznak
    /// se ignoruje (stejne chovani jako drive).
    #[allow(dead_code, reason = "vyznam ma jen na aarch64")]
    pub device: bool,
}

pub struct PageTables {
    pml4: u64,
    memory_type: MemoryType,
    nx: bool,
}

impl PageTables {
    /// Vytvori prazdnou PML4. `nx` urcuje, zda se smi pouzit bit NX.
    pub fn new(memory_type: MemoryType, nx: bool) -> uefi::Result<Self> {
        let pml4 = alloc_table(memory_type)?;
        Ok(Self {
            pml4,
            memory_type,
            nx,
        })
    }

    /// Fyzicka adresa PML4 (hodnota pro CR3).
    pub fn root(&self) -> u64 {
        self.pml4
    }

    fn leaf_bits(&self, flags: Flags) -> u64 {
        let mut bits = PRESENT;
        if flags.writable {
            bits |= WRITABLE;
        }
        if self.nx && !flags.executable {
            bits |= NO_EXECUTE;
        }
        bits
    }

    /// Vrati ukazatel na polozku tabulky urovne `level` (4 = PML4, 1 = PT)
    /// pro adresu `virt`; chybejici mezilehle tabulky alokuje.
    fn entry(&mut self, virt: u64, level: u32) -> uefi::Result<*mut u64> {
        let mut table = self.pml4;
        for l in (level + 1..=4).rev() {
            let e = entry_ptr(table, virt, l);
            // SAFETY: `e` ukazuje do tabulky alokovane touto strukturou
            // (alloc_table), dostupne pres identicke mapovani firmwaru.
            let val = unsafe { *e };
            if val & PRESENT == 0 {
                let new = alloc_table(self.memory_type)?;
                // Mezilehle urovne jsou co nejpovolnejsi, prava urcuje list.
                // SAFETY: viz vyse.
                unsafe { *e = new | PRESENT | WRITABLE };
                table = new;
            } else {
                assert!(val & HUGE == 0, "konflikt huge a 4K stranky na {virt:#x}");
                table = val & ADDR_MASK;
            }
        }
        Ok(entry_ptr(table, virt, level))
    }

    /// Namapuje jednu 4 KiB stranku. Pokud uz je namapovana na stejnou
    /// fyzickou adresu, prava se sjednoti (povolnejsi vyhrava).
    pub fn map_4k(&mut self, virt: u64, phys: u64, flags: Flags) -> uefi::Result {
        let bits = self.leaf_bits(flags);
        let e = self.entry(virt, 1)?;
        // SAFETY: `e` je platna polozka PT vracena `entry`.
        unsafe {
            let old = *e;
            *e = if old & PRESENT != 0 {
                assert_eq!(
                    old & ADDR_MASK,
                    phys,
                    "4K stranka {virt:#x} uz je namapovana jinam"
                );
                // WRITABLE: OR, NO_EXECUTE: AND.
                ((old | bits) & !NO_EXECUTE) | (old & bits & NO_EXECUTE)
            } else {
                phys | bits
            };
        }
        Ok(())
    }

    /// Namapuje jednu 2 MiB stranku.
    pub fn map_2m(&mut self, virt: u64, phys: u64, flags: Flags) -> uefi::Result {
        let bits = self.leaf_bits(flags) | HUGE;
        let e = self.entry(virt, 2)?;
        // SAFETY: `e` je platna polozka PD vracena `entry`.
        unsafe {
            let old = *e;
            assert!(
                old & PRESENT == 0 || old & HUGE != 0,
                "konflikt huge a 4K stranky na {virt:#x}"
            );
            *e = phys | bits;
        }
        Ok(())
    }

    /// Namapuje fyzicky rozsah na `phys + virt_offset` 2 MiB strankami.
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

fn entry_ptr(table: u64, virt: u64, level: u32) -> *mut u64 {
    let index = (virt >> (12 + 9 * (level - 1))) & 0x1FF;
    (table + index * 8) as *mut u64
}

fn alloc_table(memory_type: MemoryType) -> uefi::Result<u64> {
    let ptr = boot::allocate_pages(AllocateType::AnyPages, memory_type, 1)?;
    // SAFETY: prave alokovana stranka o velikosti 4 KiB, nikdo jiny ji nepouziva.
    unsafe { ptr.as_ptr().write_bytes(0, SIZE_4K as usize) };
    Ok(ptr.as_ptr() as u64)
}
