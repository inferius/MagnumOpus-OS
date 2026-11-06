//! Sprava pameti jadra.
//!
//! Rozlozeni virtualniho adresniho prostoru jadra (horni polovina):
//!
//! | od                      | co                                      |
//! |-------------------------|-----------------------------------------|
//! | `0xFFFF_8000_0000_0000` | prima mapa fyzicke pameti (RW, NX)      |
//! | `0xFFFF_FE00_0000_0000` | zasobniky jadra (s ochrannymi strankami)|
//! | `0xFFFF_FFFF_8000_0000` | obraz jadra (podle sekci R-X, R--, RW-) |
//!
//! Spodni polovina patri userspace; identicke mapovani od loaderu se po
//! prepnuti na tabulky jadra zahodi.

pub mod heap;
pub mod phys;

use alloc::vec::Vec;

use boot_info::{BootInfo, MemoryKind, PHYS_MAP_BASE};
use spin::Mutex;

use crate::arch::paging::{MapFlags, PAGE_SIZE, PageSize, PageTable, supports_1g_pages};

/// Zacatek oblasti zasobniku jadra.
const KERNEL_STACKS_BASE: u64 = 0xFFFF_FE00_0000_0000;
/// Velikost zasobniku jadra (bez ochranne stranky).
pub const KERNEL_STACK_SIZE: u64 = 64 * 1024;

/// Fyzicka adresa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysAddr(u64);

impl PhysAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Adresa v prime mape fyzicke pameti.
    pub const fn to_virt(self) -> VirtAddr {
        VirtAddr(self.0 + PHYS_MAP_BASE)
    }

    /// Opak `to_virt`; `virt` musi lezet v prime mape.
    pub fn from_direct_map(virt: VirtAddr) -> Self {
        debug_assert!(virt.0 >= PHYS_MAP_BASE);
        Self(virt.0 - PHYS_MAP_BASE)
    }
}

/// Virtualni adresa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtAddr(u64);

impl VirtAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub const fn as_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }
}

/// Adresni prostor jadra. Pozdeji z nej budou procesy kopirovat horni polovinu.
static KERNEL_SPACE: Mutex<Option<PageTable>> = Mutex::new(None);

/// Hranice sekci obrazu jadra z linker.ld.
mod sections {
    unsafe extern "C" {
        pub static __text_start: u8;
        pub static __text_end: u8;
        pub static __rodata_start: u8;
        pub static __rodata_end: u8;
        pub static __data_start: u8;
        pub static __data_end: u8;
    }
}

/// Inicializuje spravu pameti: alokator fyzicke pameti, vlastni tabulky
/// stranek jadra a heap. Vraci vrchol noveho zasobniku jadra (s ochrannou
/// strankou), na ktery se ma volajici prepnout.
///
/// # Safety
///
/// Volat jednou, po `arch::init_boot_cpu`. `boot_info` musi byt platny.
pub unsafe fn init(boot_info: &BootInfo) -> u64 {
    assert_eq!(
        boot_info.phys_offset, PHYS_MAP_BASE,
        "loader pouziva jinou primou mapu"
    );
    // SAFETY: BootInfo je platny podle kontraktu.
    let regions = unsafe { boot_info.memory_map() };

    phys::init(regions);

    let mut space = PageTable::new().expect("nedostatek pameti pro PML4");
    space
        .preallocate_kernel_half()
        .expect("nedostatek pameti pro tabulky jadra");
    map_direct(&mut space, boot_info);
    // SAFETY: symboly sekci pochazi z linker.ld, loader nahral obraz jadra
    // souvisle od `kernel_phys_start` (odpovida KERNEL_BASE).
    unsafe { map_kernel_image(&mut space, boot_info.kernel_phys_start) };

    // SAFETY: nove tabulky mapuji obraz jadra (bezici kod a statiky), primou
    // mapu (aktualni zasobnik, BootInfo, framebuffer a tabulky samotne).
    unsafe { space.activate() };

    // Identicke mapovani ani kod loaderu uz nikdo nepotrebuje. GDT a IDT
    // firmwaru jsou nahrazene vlastnimi, takze i pamet boot services je volna.
    phys::add_regions(regions, MemoryKind::Reclaimable);

    let stack_top = alloc_kernel_stack(&mut space, 0).expect("nedostatek pameti pro zasobnik");
    *KERNEL_SPACE.lock() = Some(space);
    stack_top
}

/// Namapuje primou mapu: vsechny oblasti z mapy pameti a framebuffer.
/// Oblasti se zarovnaji na 2 MiB a spoji, pak se mapuji co nejvetsimi
/// strankami (1 GiB, pokud je CPU podporuje, jinak 2 MiB).
fn map_direct(space: &mut PageTable, boot_info: &BootInfo) {
    const SIZE_2M: u64 = PageSize::Size2M.bytes();
    // SAFETY: BootInfo overeny v `init`.
    let regions = unsafe { boot_info.memory_map() };

    let mut ranges: Vec<(u64, u64)> = regions
        .iter()
        .map(|r| (r.start, r.end()))
        .chain(boot_info.framebuffer.is_present().then(|| {
            let fb = &boot_info.framebuffer;
            (fb.phys_addr, fb.phys_addr + fb.len)
        }))
        .map(|(s, e)| (s & !(SIZE_2M - 1), e.next_multiple_of(SIZE_2M)))
        .collect();
    ranges.sort_unstable();

    let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        match merged.last_mut() {
            Some(last) if s <= last.1 => last.1 = last.1.max(e),
            _ => merged.push((s, e)),
        }
    }

    // MMIO (framebuffer) se mapuje se stejnym typem cache jako RAM; skutecny
    // typ (UC/WC) zatim urcuji MTRR nastavene firmwarem.
    // TODO: PAT a write-combining pro framebuffer.
    let flags = MapFlags::WRITE | MapFlags::GLOBAL;
    let gig = supports_1g_pages();
    for (start, end) in merged {
        let mut p = start;
        while p < end {
            let size = if gig
                && p % PageSize::Size1G.bytes() == 0
                && p + PageSize::Size1G.bytes() <= end
            {
                PageSize::Size1G
            } else {
                PageSize::Size2M
            };
            space
                .map(PhysAddr(p).to_virt(), PhysAddr(p), size, flags)
                .expect("mapovani prime mapy selhalo");
            p += size.bytes();
        }
    }
}

/// Namapuje obraz jadra 4 KiB strankami s pravy podle sekci.
///
/// # Safety
///
/// Obraz jadra musi lezet fyzicky souvisle od `phys_start` a zacinat na
/// adrese `__text_start`.
unsafe fn map_kernel_image(space: &mut PageTable, phys_start: u64) {
    use sections::*;
    let image_start = &raw const __text_start as u64;
    let parts = [
        (
            &raw const __text_start,
            &raw const __text_end,
            MapFlags::EXEC,
        ),
        (
            &raw const __rodata_start,
            &raw const __rodata_end,
            MapFlags::READ,
        ),
        (
            &raw const __data_start,
            &raw const __data_end,
            MapFlags::WRITE,
        ),
    ];
    for (start, end, flags) in parts {
        let (start, end) = (start as u64, (end as u64).next_multiple_of(PAGE_SIZE));
        for virt in (start..end).step_by(PAGE_SIZE as usize) {
            let phys = PhysAddr(phys_start + (virt - image_start));
            space
                .map(
                    VirtAddr(virt),
                    phys,
                    PageSize::Size4K,
                    flags | MapFlags::GLOBAL,
                )
                .expect("mapovani obrazu jadra selhalo");
        }
    }
}

/// Alokuje zasobnik jadra cislo `index` v oblasti zasobniku. Pod kazdym
/// zasobnikem zustava nenamapovana ochranna stranka, takze preteceni
/// skonci page faultem (a double faultem na IST), ne tichym prepsanim pameti.
fn alloc_kernel_stack(space: &mut PageTable, index: u64) -> Option<u64> {
    let slot = KERNEL_STACK_SIZE + PAGE_SIZE;
    let bottom = KERNEL_STACKS_BASE + index * slot + PAGE_SIZE;
    for virt in (bottom..bottom + KERNEL_STACK_SIZE).step_by(PAGE_SIZE as usize) {
        let frame = phys::alloc_frame_zeroed()?;
        space
            .map(
                VirtAddr(virt),
                frame,
                PageSize::Size4K,
                MapFlags::WRITE | MapFlags::GLOBAL,
            )
            .ok()?;
    }
    Some(bottom + KERNEL_STACK_SIZE)
}

/// Prelozi adresu v adresnim prostoru jadra (pro testy a ladeni).
pub fn translate(virt: VirtAddr) -> Option<PhysAddr> {
    KERNEL_SPACE.lock().as_ref()?.translate(virt)
}
