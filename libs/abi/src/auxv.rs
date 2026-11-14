//! Pomocny vektor (auxv) pro zavadec `ld.so` (`docs/cs/dynamicke-linkovani.md`).
//!
//! Program s `PT_INTERP` (PIE) spousti `spawn` pres zavadec: na vrchol
//! zasobniku hlavniho vlakna zapise rozlozeni SysV (slova little endian)
//!
//! ```text
//! SP -> argc = 0 | argv: 0 | envp: 0 | (typ, hodnota)... | AT_NULL, 0 | 16 B nahody
//! ```
//!
//! a vlakno zacne na vstupu zavadece (`rdi`/`x0` = handle bootstrapu, SP
//! jako vyse). Cisla typu jsou jako v SysV ABI, takze prevzaty kod
//! (`getauxval`) funguje beze zmeny. Argumenty a prostredi jdou dal pres
//! blok parametru (`abi::params`), ne pres argv.

/// Konec vektoru.
pub const AT_NULL: u64 = 0;
/// Adresa programovych hlavicek programu (uz s bazi).
pub const AT_PHDR: u64 = 3;
/// Velikost programove hlavicky (56).
pub const AT_PHENT: u64 = 4;
/// Pocet programovych hlavicek.
pub const AT_PHNUM: u64 = 5;
/// Velikost stranky procesu.
pub const AT_PAGESZ: u64 = 6;
/// Baze zavadece.
pub const AT_BASE: u64 = 7;
/// Vstupni bod programu (uz s bazi).
pub const AT_ENTRY: u64 = 9;
/// Adresa 16 B nahody (seminko kanarku, ukazatelu).
pub const AT_RANDOM: u64 = 25;

/// Cesta zavadece v `PT_INTERP` a jeho jmeno v initfs.
pub const INTERP: &str = "/system/lib/ld.so";
/// Adresar systemovych knihoven (`/system/lib/x` = `lib/x` v initfs).
pub const SYSTEM_LIB: &str = "/system/lib/";
/// Predpona knihoven v initfs.
pub const INITFS_LIB: &str = "lib/";

/// Hledani volne baze modulu s nahodnou adresou: rozsah adres (pod
/// heapem libos na `0x6000_0000_0000`, nad oblasti, kterou jadro dava
/// mapovani bez pevne adresy) a zarovnani 64 KiB (~29 bitu nahody).
pub const ASLR_START: u64 = 0x0000_2000_0000_0000;
pub const ASLR_END: u64 = 0x0000_5000_0000_0000;
pub const ASLR_ALIGN: u64 = 0x1_0000;

/// Zavadec konci timto kodem, kdyz program nejde zavest (chybi knihovna,
/// symbol, vadny soubor); duvod je v logu (`ld.so: ...`).
pub const EXIT_LOAD_FAILED: i64 = 127;
