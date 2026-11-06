//! Priznaky mapovani pameti a rozlozeni adresniho prostoru procesu.

/// Velikost stranky.
pub const PAGE_SIZE: u64 = 4096;

/// Pristupova prava mapovani (`VM_MAP` flags).
pub const MAP_READ: u32 = 1 << 0;
pub const MAP_WRITE: u32 = 1 << 1;
pub const MAP_EXECUTE: u32 = 1 << 2;
/// Mapovat presne na zadanou adresu (jinak je adresa jen napoveda a 0 =
/// vyber jadra).
pub const MAP_FIXED: u32 = 1 << 3;

/// Zacatek pameti procesu, kterou smi mapovat (pod ni zustava nenamapovany
/// prostor, aby dereference nuloveho ukazatele vzdy skoncila chybou).
pub const USER_MIN: u64 = 0x0000_0000_0001_0000;
/// Konec pameti procesu (exkluzivne). Posledni stranka spodni poloviny
/// adresniho prostoru zustava vzdy nenamapovana: instrukce `syscall` na jejim
/// konci by jinak vratila nekanonickou adresu (0x8000_0000_0000) a navrat do
/// userspace by vyvolal vyjimku jeste v jadre.
pub const USER_MAX: u64 = 0x0000_7FFF_FFFF_F000;
/// Odkud jadro vybira adresy pro mapovani bez MAP_FIXED.
pub const USER_ALLOC_BASE: u64 = 0x0000_1000_0000_0000;

/// Maximalni velikost jednoho objektu pameti.
pub const VMO_MAX_SIZE: u64 = 1 << 32;
