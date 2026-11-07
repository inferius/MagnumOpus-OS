//! Rozhrani mezi UEFI loaderem a kernelem.
//!
//! Loader naplni [`BootInfo`] a preda kernelu jeho adresu (virtualni adresu
//! v prime mape fyzicke pameti, viz [`BootInfo::phys_offset`]). Vsechny
//! struktury jsou `#[repr(C)]` a obsahuji jen prosta data, aby jejich layout
//! nezavisel na verzi kompilatoru loaderu a kernelu.
//!
//! Virtualni adresni prostor, ktery loader pripravi pred skokem do kernelu:
//!
//! - `0x0000_0000_0000_0000 + phys` - identita (jen pro prechod z loaderu,
//!   kernel ji pozdeji zrusi)
//! - [`PHYS_MAP_BASE`] `+ phys` - prima mapa fyzicke pameti a framebufferu
//! - [`KERNEL_BASE`] a vys - obraz kernelu podle jeho ELF segmentu

#![no_std]

/// Magicka hodnota na zacatku [`BootInfo`] ("OSBOOTIF" v ASCII).
pub const BOOT_INFO_MAGIC: u64 = u64::from_le_bytes(*b"OSBOOTIF");

/// Verze rozhrani. Zvysit pri kazde nekompatibilni zmene struktur.
pub const BOOT_INFO_VERSION: u32 = 3;

/// Zacatek prime mapy fyzicke pameti (zacatek horni poloviny adresniho prostoru).
pub const PHYS_MAP_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Adresa, na kterou je kernel linkovany (poslednich 2 GiB adresniho prostoru).
/// Musi odpovidat `kernel/linker.ld`.
pub const KERNEL_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// Velikost zasobniku, ktery loader pripravi pro kernel.
pub const KERNEL_STACK_SIZE: u64 = 64 * 1024;

/// Informace predane kernelu pri startu.
#[repr(C)]
#[derive(Debug)]
pub struct BootInfo {
    /// Vzdy [`BOOT_INFO_MAGIC`].
    pub magic: u64,
    /// Vzdy [`BOOT_INFO_VERSION`].
    pub version: u32,
    pub _reserved: u32,
    /// Posun prime mapy fyzicke pameti: `virt = phys + phys_offset`.
    pub phys_offset: u64,
    /// Mapa fyzicke pameti (virtualni adresa pole [`MemoryRegion`]).
    pub memory_map: u64,
    /// Pocet polozek v `memory_map`.
    pub memory_map_len: u64,
    /// Framebuffer, pokud ho firmware poskytl.
    pub framebuffer: Framebuffer,
    /// Fyzicka adresa ACPI RSDP (verze 2+), 0 pokud chybi.
    pub acpi_rsdp: u64,
    /// Fyzicky rozsah, kam loader nahral segmenty kernelu.
    pub kernel_phys_start: u64,
    pub kernel_phys_len: u64,
    /// Obraz initfs v pameti (fyzicka adresa a delka), 0 pokud chybi.
    pub initfs_phys: u64,
    pub initfs_len: u64,
    /// Realny cas z firmwaru (UEFI `GetTime`) tesne pred ukoncenim boot services.
    pub firmware_time: FirmwareTime,
}

/// Cas z hodin firmwaru prevedeny loaderem na sekundy od 1970.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FirmwareTime {
    /// 1 = platne; 0 = firmware cas neposkytl nebo byl neplatny.
    pub valid: u32,
    /// Posun pasma v minutach (mistni = UTC + pasmo), nebo
    /// [`FirmwareTime::ZONE_UNSPECIFIED`] (firmware pasmo nezna, cas je
    /// "mistni" - loader ho bere jako UTC).
    pub zone_minutes: i16,
    pub _reserved: u16,
    /// Sekundy od 1970-01-01 UTC (se znamenkem).
    pub secs: i64,
    /// Nanosekundy uvnitr sekundy (vetsina firmwaru hlasi 0).
    pub nanos: u32,
    pub _reserved2: u32,
    /// Citac cyklu CPU v okamziku cteni (x86: TSC, aarch64: CNTVCT_EL0);
    /// jadro z nej dopocita posun do startu svych hodin.
    pub counter: u64,
}

impl FirmwareTime {
    /// Hodnota UEFI `EFI_UNSPECIFIED_TIMEZONE`.
    pub const ZONE_UNSPECIFIED: i16 = 0x07FF;

    pub const fn none() -> Self {
        Self {
            valid: 0,
            zone_minutes: Self::ZONE_UNSPECIFIED,
            _reserved: 0,
            secs: 0,
            nanos: 0,
            _reserved2: 0,
            counter: 0,
        }
    }
}

impl BootInfo {
    /// Vrati mapu pameti jako slice.
    ///
    /// # Safety
    ///
    /// `self` musi byt struktura vyplnena loaderem a prima mapa fyzicke
    /// pameti musi byt stale namapovana.
    pub unsafe fn memory_map(&self) -> &[MemoryRegion] {
        // SAFETY: loader alokoval `memory_map_len` souvislych, inicializovanych
        // polozek na adrese `memory_map`; pamet je oznacena jako
        // `MemoryKind::BootData`, takze ji nikdo neprepise. Volajici ruci za
        // platnost mapovani.
        unsafe {
            core::slice::from_raw_parts(
                self.memory_map as *const MemoryRegion,
                self.memory_map_len as usize,
            )
        }
    }
}

/// Jedna souvisla oblast fyzicke pameti.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    /// Fyzicka adresa zacatku (zarovnana na 4 KiB).
    pub start: u64,
    /// Velikost v bajtech (nasobek 4 KiB).
    pub len: u64,
    pub kind: MemoryKind,
    pub _reserved: u32,
}

impl MemoryRegion {
    pub const fn end(&self) -> u64 {
        self.start + self.len
    }
}

/// Druh oblasti fyzicke pameti, uz prelozeny z UEFI typu do vlastnich pojmu.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    /// Volna RAM, kernel ji muze hned pouzit.
    Usable = 0,
    /// RAM, kterou pouzival firmware nebo loader; po prevzeti vsech dat
    /// z [`BootInfo`] a nastaveni vlastni GDT/IDT se z ni stane `Usable`.
    Reclaimable = 1,
    /// Obraz kernelu (kod a data).
    Kernel = 2,
    /// Data pripravena loaderem pro kernel: tabulky stranek, zasobnik,
    /// `BootInfo`, mapa pameti. Uvolnit az kdyz je kernel nepotrebuje.
    BootData = 3,
    /// ACPI tabulky, po jejich precteni lze uvolnit.
    AcpiReclaimable = 4,
    /// ACPI NVS, nesahat.
    AcpiNvs = 5,
    /// Pamet UEFI runtime sluzeb.
    FirmwareRuntime = 6,
    /// Pametove mapovane I/O.
    Mmio = 7,
    /// Vadna pamet.
    Unusable = 8,
    /// Vse ostatni, nesahat.
    Reserved = 9,
    /// Obraz initfs od loaderu; po prevzeti jadrem lze uvolnit.
    Initfs = 10,
}

/// Format jednoho pixelu (vzdy 32 bitu na pixel).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// Framebuffer neni k dispozici.
    None = 0,
    /// Bajty v pameti: R, G, B, rezerva.
    Rgb = 1,
    /// Bajty v pameti: B, G, R, rezerva.
    Bgr = 2,
}

/// Linearni framebuffer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Framebuffer {
    /// Fyzicka adresa.
    pub phys_addr: u64,
    /// Virtualni adresa (v prime mape fyzicke pameti).
    pub virt_addr: u64,
    /// Velikost v bajtech.
    pub len: u64,
    /// Sirka v pixelech.
    pub width: u32,
    /// Vyska v pixelech.
    pub height: u32,
    /// Pocet pixelu na radek (muze byt vetsi nez `width`).
    pub stride: u32,
    pub format: PixelFormat,
}

impl Framebuffer {
    pub const fn none() -> Self {
        Self {
            phys_addr: 0,
            virt_addr: 0,
            len: 0,
            width: 0,
            height: 0,
            stride: 0,
            format: PixelFormat::None,
        }
    }

    pub fn is_present(&self) -> bool {
        self.format != PixelFormat::None
    }
}
