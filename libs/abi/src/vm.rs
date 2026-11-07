//! Priznaky mapovani pameti a rozlozeni adresniho prostoru procesu.

/// Nejmensi velikost stranky na vsech architekturach (4 KiB).
///
/// Skutecna velikost stranky je vlastnost adresniho prostoru procesu: je to
/// mocnina dvou od `MIN_PAGE_SIZE` do `MAX_PAGE_SIZE` a program ji zjisti
/// za behu (`VM_PAGE_SIZE`, v libos `libos::vm::page_size()`). Dnes je
/// vsude 4 KiB; na aarch64 muze jadro dat nativnim procesum 16 KiB a
/// procesum Windows vrstvy 4 KiB (granule se nastavuje pro kazdy adresni
/// prostor zvlast). Proto zadna konstanta `PAGE_SIZE` v ABI neni: zarovnani
/// pro `VM_MAP`, `VM_UNMAP` a rozsahy prostredku se ridi hodnotou za behu.
pub const MIN_PAGE_SIZE: u64 = 4096;

/// Nejvetsi velikost stranky, kterou muze proces dostat (64 KiB). Na tuto
/// hodnotu se zarovnavaji segmenty programu (`user/user.ld`), aby slo
/// stejny program nahrat s jakoukoli velikosti stranky.
pub const MAX_PAGE_SIZE: u64 = 65536;

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
