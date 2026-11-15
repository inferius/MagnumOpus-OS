//! Rozhrani zavadece `ld.so` pro behove prostredi programu (DL-3,
//! `docs/cs/dynamicke-linkovani.md`).
//!
//! Zavadec zapise adresu tabulky [`Interface`] do auxv programu (typ
//! [`AT_MO_LOADER`]) a pri skoku na vstup programu preda navic SP (auxv)
//! a [`ENTRY_MAGIC`] (x86_64 `rsi`, `rdx`; aarch64 `x1`, `x2`). Behove
//! prostredi (libos, crt) tak pozna, ze bezi pod zavadecem, a pres tabulku:
//!
//! - dostane blok TLS pro nove vlakno (staticke TLS vsech modulu),
//! - pri konci procesu zavola finalizatory modulu,
//! - projde nahrane moduly (`dl_iterate_phdr`).
//!
//! Kod v C vola funkce zavadece jmenem (`dl_iterate_phdr`, `__tls_get_addr`,
//! pozdeji `dlopen` ...): zavadec je resi jako vestavene symboly.

/// Typ auxv s adresou [`Interface`] (mimo rozsah SysV, "MO").
pub const AT_MO_LOADER: u64 = 0x4D4F_0001;

/// Typ auxv s velikosti rezervy statickeho TLS v bajtech (manifest
/// `[app] static_tls_reserve`, `libos::spawn::Options`); bez nej vychozi
/// 2 KiB, nejvys 64 KiB.
pub const AT_MO_TLS_RESERVE: u64 = 0x4D4F_0002;

/// Typ auxv s adresou tabulky rezervaci adres (Windows vrstva, T5):
/// slova `[pocet, zacatek0, delka0, zacatek1, delka1, ...]` v bloku auxv na
/// zasobniku hlavniho vlakna (zije do konce procesu). Rozsahy rezervoval
/// `spawn` v potomkovi (`VM_RESERVE` s pevnou adresou) jako prvni, pred
/// namapovanim programu, zavadece a zasobniku, takze je nezabere zadny
/// modul ani mapovani bez pevne adresy; program je prevezme (`VM_MAP`
/// s `MAP_IN_RESERVATION`). Nejvys [`RESERVE_MAX`] rozsahu.
pub const AT_MO_RESERVE: u64 = 0x4D4F_0003;

/// Nejvic rozsahu v [`AT_MO_RESERVE`].
pub const RESERVE_MAX: usize = 8;

/// Hodnota v tretim argumentu vstupu programu od zavadece ("MOLD").
pub const ENTRY_MAGIC: u64 = 0x4D4F_4C44;

/// Verze tabulky (nova pole jen na konec, `size` rika kolik jich je).
pub const VERSION: u32 = 1;

/// Informace o modulu pro `dl_iterate_phdr` (rozlozeni jako glibc
/// `struct dl_phdr_info`, aby slo predat prevzatemu kodu).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PhdrInfo {
    /// Baze modulu.
    pub addr: u64,
    /// Jmeno (C retezec; program = "").
    pub name: *const u8,
    /// Programove hlavicky v pameti.
    pub phdr: *const u8,
    pub phnum: u16,
    /// Pocet nahrani a odstraneni modulu (zmena = seznam se zmenil).
    pub adds: u64,
    pub subs: u64,
    /// Cislo modulu TLS (0 = bez TLS) a data TLS volajiciho vlakna.
    pub tls_modid: u64,
    pub tls_data: *mut u8,
}

/// Vysledek `dladdr` (rozlozeni jako glibc `Dl_info`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DlInfo {
    /// Jmeno modulu (C retezec).
    pub fname: *const u8,
    /// Baze modulu.
    pub fbase: u64,
    /// Nejblizsi symbol (C retezec) a jeho adresa; null/0 = neznamy.
    pub sname: *const u8,
    pub saddr: u64,
}

/// Priznaky `dlopen` (hodnoty jako glibc).
pub mod rtld {
    pub const LAZY: i32 = 1;
    pub const NOW: i32 = 2;
    pub const GLOBAL: i32 = 0x100;
    pub const LOCAL: i32 = 0;
    pub const NODELETE: i32 = 0x1000;
    /// Jen vratit handle uz nahraneho modulu (jinak null bez chyby).
    pub const NOLOAD: i32 = 4;
    /// `dlsym`: globalni rozsah (program, knihovny ze startu, `GLOBAL`).
    pub const DEFAULT: usize = 0;
    /// `dlsym`: dalsi vyskyt za volajicim modulem (zatim nepodporovano).
    pub const NEXT: usize = usize::MAX;
}

/// Callback `dl_iterate_phdr`: nenulova navratova hodnota zastavi pruchod.
pub type PhdrCallback = unsafe extern "C" fn(info: *mut PhdrInfo, size: usize, data: *mut u8) -> i32;

/// Tabulka funkci zavadece (ABI C, adresa v auxv `AT_MO_LOADER`).
#[repr(C)]
pub struct Interface {
    pub version: u32,
    /// Velikost tabulky v bajtech (starsi program cte jen co zna).
    pub size: u32,
    /// Novy blok TLS pro vlakno: vraci ukazatel vlakna (TP) nebo 0 (malo
    /// pameti). Blok obsahuje kopii sablon vsech modulu a rezervu.
    pub thread_tls_new: unsafe extern "C" fn() -> u64,
    /// Uvolni blok vlakna, ktere skoncilo (`tp` z `thread_tls_new`).
    pub thread_tls_free: unsafe extern "C" fn(tp: u64),
    /// Finalizatory modulu (`DT_FINI_ARRAY` pozpatku, `DT_FINI`), program
    /// prvni, zavislosti posledni; podruhe nic nedela.
    pub run_fini: unsafe extern "C" fn(),
    /// `dl_iterate_phdr` (glibc): projde nahrane moduly.
    pub dl_iterate_phdr: unsafe extern "C" fn(cb: PhdrCallback, data: *mut u8) -> i32,
    /// `dlopen(cesta, rtld::*)` -> handle, null = chyba (`dlerror`).
    /// Cesta jen `/system/lib/<jmeno>` nebo `<jmeno>` (knihovny od libd);
    /// null = program (globalni rozsah). `LAZY` = `NOW`.
    pub dlopen: unsafe extern "C" fn(path: *const u8, flags: i32) -> *mut u8,
    /// `dlsym(handle, jmeno)` -> adresa, null = nenalezeno.
    pub dlsym: unsafe extern "C" fn(handle: *mut u8, name: *const u8) -> *mut u8,
    /// `dlclose(handle)`: 0 = ok. Snizi pocitadlo odkazu; modul zustava
    /// nahrany (kod je zapeceteny, odstraneni modulu neni).
    pub dlclose: unsafe extern "C" fn(handle: *mut u8) -> i32,
    /// Posledni chyba `dl*` volajiciho vlakna jako C retezec (null =
    /// zadna); cteni ji vymaze, retezec plati do dalsiho volani `dl*`
    /// v tomtez vlakne.
    pub dlerror: unsafe extern "C" fn() -> *const u8,
    /// `dladdr(adresa, &mut DlInfo)`: nenulove = adresa lezi v modulu.
    pub dladdr: unsafe extern "C" fn(addr: *const u8, info: *mut DlInfo) -> i32,
    /// Nova relace s libd a listek k ni pro behove prostredi programu
    /// (libos pri prvnim spawnu programu se zavadecem): zavadec udela
    /// `NARROW` na svem listku. 0 = ok (dva handly zapsane), jinak kod
    /// chyby `abi::Error`. (DL-4b; starsi zavadec pole nema - `size`.)
    pub libd_session: unsafe extern "C" fn(session: *mut u32, lease: *mut u32) -> i32,
    /// Inicializatory modulu ze startu (`DT_INIT`, `DT_INIT_ARRAY`;
    /// zavislosti prvni, program posledni). Zavadec je pred skokem do
    /// programu nespousti: konstruktory (C++ staticke objekty) alokuji
    /// a registruji `__cxa_atexit`, takze smi bezet az po priprave
    /// behoveho prostredi programu, ktere tuto funkci zavola hned po sve
    /// inicializaci a pred `main`. Podruhe nic nedela. Moduly z `dlopen`
    /// inicializuje `dlopen` sam. (7a-1; starsi zavadec pole nema - `size`,
    /// spustil inicializatory sam.)
    pub run_init: unsafe extern "C" fn(),
}

/// Ma tabulka velikosti `size` pole konci na `end` (offset konce pole)?
pub const fn has_field(size: u32, end: usize) -> bool {
    size as usize >= end
}
