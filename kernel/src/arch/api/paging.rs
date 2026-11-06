//! Cast smlouvy pro strankovani: typy spolecne vsem architekturam
//! a operace nad tabulkami stranek jednoho adresniho prostoru.
//!
//! Velikost stranky je pevne 4 KiB na vsech architekturach (je soucasti
//! ABI, viz `abi::vm::PAGE_SIZE`). Na aarch64 to znamena granuli 4 KiB:
//! velke stranky jsou pak stejne jako na x86_64 (2 MiB a 1 GiB bloky).

use crate::mm::{PhysAddr, VirtAddr};

/// Velikost zakladni stranky (4 KiB na vsech architekturach).
pub const PAGE_SIZE: u64 = 4096;

const _: () = assert!(PAGE_SIZE == abi::vm::PAGE_SIZE);

/// Prava a vlastnosti mapovani, nezavisle na architekture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MapFlags(u8);

impl MapFlags {
    pub const READ: Self = Self(0);
    pub const WRITE: Self = Self(1 << 0);
    pub const EXEC: Self = Self(1 << 1);
    #[allow(dead_code)]
    pub const USER: Self = Self(1 << 2);
    /// Mapovani je stejne ve vsech adresnich prostorech (cast jadra),
    /// TLB ho pri prepnuti adresniho prostoru nezahodi.
    pub const GLOBAL: Self = Self(1 << 3);
    /// Bez cache (registry zarizeni).
    pub const UNCACHED: Self = Self(1 << 4);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl core::ops::BitOr for MapFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Velikost listu mapovani.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageSize {
    Size4K,
    Size2M,
    Size1G,
}

impl PageSize {
    pub const fn bytes(self) -> u64 {
        match self {
            Self::Size4K => 0x1000,
            Self::Size2M => 0x20_0000,
            Self::Size1G => 0x4000_0000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapError {
    OutOfMemory,
    AlreadyMapped,
    HugePageConflict,
    Misaligned,
}

/// Tabulky stranek jednoho adresniho prostoru.
///
/// Rozdeleni adresniho prostoru je na vsech architekturach stejne: spodni
/// polovina patri procesu, horni jadru (viz `crate::mm`). Jak se horni
/// polovina sdili, je vec architektury: x86_64 kopiruje polozky korenove
/// tabulky (proto `preallocate_kernel_half`), aarch64 ma na jadro vlastni
/// registr (TTBR1_EL1), takze tabulky procesu horni polovinu vubec nemaji.
pub trait PageTableOps: Sized + Send {
    /// Vytvori prazdny adresni prostor.
    fn new() -> Result<Self, MapError>;

    /// Novy adresni prostor procesu, ktery sdili horni polovinu (jadro)
    /// s `kernel`.
    fn new_sharing_kernel_half(kernel: &Self) -> Result<Self, MapError>;

    /// Pripravi tabulky horni poloviny tak, aby zmena mapovani jadra byla
    /// videt ve vsech adresnich prostorech (volat jednou na tabulkach jadra).
    fn preallocate_kernel_half(&mut self) -> Result<(), MapError>;

    /// Prepne aktualni CPU na tento adresni prostor.
    ///
    /// # Safety
    ///
    /// Tabulky musi mapovat prave bezici kod, zasobnik a vsechna data,
    /// ktera se po prepnuti pouzivaji.
    unsafe fn activate(&self);

    /// Namapuje jednu stranku.
    fn map(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        size: PageSize,
        flags: MapFlags,
    ) -> Result<(), MapError>;

    /// Zrusi mapovani jedne 4 KiB stranky na tomto CPU (TLB ostatnich CPU
    /// se nezneplatni - volajici ruci, ze tam stranka neni pouzivana).
    fn unmap_4k(&mut self, virt: VirtAddr);

    /// Prelozi virtualni adresu na fyzickou, `None` pokud neni namapovana.
    fn translate(&self, virt: VirtAddr) -> Option<PhysAddr>;

    /// Fyzicka adresa korenove tabulky (hodnota pro `switch_page_table`).
    fn root(&self) -> u64;

    /// Uvolni tabulky spodni poloviny (userspace) a korenovou tabulku.
    /// Namapovane stranky samotne neuvolnuje - patri objektum pameti.
    ///
    /// # Safety
    ///
    /// Adresni prostor nesmi byt aktivni na zadnem CPU.
    unsafe fn destroy(self);
}
