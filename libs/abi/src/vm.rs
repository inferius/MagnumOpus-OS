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
/// `VM_MAP` jen dovnitr rezervaci (`VM_RESERVE`): rozsah musi lezet cely
/// v rezervacich a mapovani je nahradi (jako `MEM_COMMIT` ve Windows).
/// Jen spolu s `MAP_FIXED`. Bez nej je rezervace obsazene misto.
pub const MAP_IN_RESERVATION: u32 = 1 << 4;
/// `VM_RESERVE`: ochranna oblast (napr. pod zasobnikem). Pristup do ni
/// ukonci proces s hlaskou o preteceni.
pub const RESERVE_GUARD: u32 = 1 << 5;
/// `VM_PROTECT`: oblasti v rozsahu navic **zapecetit** (jako Linux
/// `mseal`): uz nejde zmenit prava (`VM_PROTECT`), odmapovat ani uvolnit
/// pamet (`VM_UNMAP`, i `UNMAP_DECOMMIT`) ani nahradit rezervaci mapovanim;
/// mapovat pres ne nejde nikdy. Nevratne do konce procesu. Prava mapovani
/// se nastavi na `prot` jako bez pecete; rezervace v rozsahu se jen
/// zapecetí (`prot` se jich netyka). Pouziti: RELRO po relokaci, kod
/// knihoven, zasobnik zavadece (`docs/cs/dynamicke-linkovani.md`).
pub const PROT_SEAL: u32 = 1 << 8;
/// `VM_UNMAP`: mapovani v rozsahu zmenit na rezervaci (adresy zustanou
/// zabrane, pamet se vrati; `MEM_DECOMMIT`). Rezervace zustanou.
pub const UNMAP_DECOMMIT: u32 = 1 << 0;

/// Druh oblasti a atributy ve vystupu `VM_QUERY`.
pub mod region {
    /// Volne misto (mezi oblastmi).
    pub const FREE: u32 = 0;
    /// Rezervace bez pameti.
    pub const RESERVED: u32 = 1;
    /// Mapovani objektu pameti.
    pub const MAPPED: u32 = 2;
    /// Atribut: ochranna rezervace (`RESERVE_GUARD`).
    pub const ATTR_GUARD: u32 = 1 << 0;
    /// Atribut: mapovani vzniklo v rezervaci (`MAP_IN_RESERVATION`).
    pub const ATTR_IN_RESERVATION: u32 = 1 << 1;
    /// Atribut: oblast je zapecetena (`PROT_SEAL`).
    pub const ATTR_SEALED: u32 = 1 << 2;
}

/// Vystup `VM_QUERY` (40 B, little endian: `start`, `len` u64, `kind`,
/// `flags`, `max_flags`, `attrs` u32, `vmo_offset` u64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegionInfo {
    /// Zacatek oblasti (u volneho mista konec predchozi oblasti).
    pub start: u64,
    pub len: u64,
    /// `region::FREE` / `RESERVED` / `MAPPED`.
    pub kind: u32,
    /// Aktualni prava (`MAP_READ | MAP_WRITE | MAP_EXECUTE`).
    pub flags: u32,
    /// Nejvyssi prava, na ktera smi `VM_PROTECT` vratit.
    pub max_flags: u32,
    /// `region::ATTR_*`.
    pub attrs: u32,
    /// Offset zacatku oblasti v objektu pameti (jen mapovani).
    pub vmo_offset: u64,
}

impl RegionInfo {
    pub const SIZE: usize = 40;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..8].copy_from_slice(&self.start.to_le_bytes());
        b[8..16].copy_from_slice(&self.len.to_le_bytes());
        b[16..20].copy_from_slice(&self.kind.to_le_bytes());
        b[20..24].copy_from_slice(&self.flags.to_le_bytes());
        b[24..28].copy_from_slice(&self.max_flags.to_le_bytes());
        b[28..32].copy_from_slice(&self.attrs.to_le_bytes());
        b[32..40].copy_from_slice(&self.vmo_offset.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let u64_at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        Self {
            start: u64_at(0),
            len: u64_at(8),
            kind: u32_at(16),
            flags: u32_at(20),
            max_flags: u32_at(24),
            attrs: u32_at(28),
            vmo_offset: u64_at(32),
        }
    }
}

/// Typ cache fyzickeho objektu pameti (`VMO_CREATE_PHYSICAL`). Urcuje ho
/// drzitel prostredku pri vytvoreni; kazde mapovani objektu ho zdedi.
pub mod cache {
    /// Bez cache (registry zarizeni). Vychozi.
    pub const UNCACHED: u64 = 0;
    /// Zapisy se slucuji, cteni bez cache (framebuffer, pamet GPU).
    pub const WRITE_COMBINING: u64 = 1;
    /// Bezna pamet s cache (pamet hostitele namapovana zarizenim, napr.
    /// sdilena pamet virtio-gpu s `MAP_CACHE_CACHED`).
    pub const CACHED: u64 = 2;
}

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

/// Nejvetsi objekt pameti (1 TiB). Strukturalni mez: seznam stranek objektu
/// je dvouurovnove pole v blocich po 4 KiB. Skutecny strop urcuje rozpocet
/// uzivatelske pameti jobu a fyzicka pamet.
pub const VMO_MAX_SIZE: u64 = 1 << 40;

/// Volba `VMO_CREATE`: vsechny stranky hned, pripnute po celou dobu zivota
/// objektu (nesouvisle; zarizeni k nim pristupuje pres `DMA_MAP`). Strhava
/// se z rozpoctu uzivatelske pameti i z rozpoctu `DMA` jobu
/// (`job::pool::DMA`). Pripnuty objekt nejde zapecetit.
pub const VMO_PINNED: u64 = 1 << 0;
/// Vsechny zname volby `VMO_CREATE`.
pub const VMO_CREATE_FLAGS: u64 = VMO_PINNED;
