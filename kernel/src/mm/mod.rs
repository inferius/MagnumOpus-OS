//! Sprava pameti jadra.
//!
//! Rozlozeni virtualniho adresniho prostoru jadra (horni polovina):
//!
//! | od                      | co                                      |
//! |-------------------------|-----------------------------------------|
//! | `0xFFFF_8000_0000_0000` | prima mapa fyzicke pameti (RW, NX)      |
//! | `0xFFFF_FD00_0000_0000` | registry zarizeni (MMIO, bez cache)     |
//! | `0xFFFF_FE00_0000_0000` | zasobniky jadra (s ochrannymi strankami)|
//! | `0xFFFF_FFFF_8000_0000` | obraz jadra (podle sekci R-X, R--, RW-) |
//!
//! Spodni polovina patri userspace; identicke mapovani od loaderu se po
//! prepnuti na tabulky jadra zahodi.

mod address_space;
pub mod heap;
pub mod phys;
pub mod tlb;

pub use address_space::AddressSpace;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::sync::SpinLock;
use boot_info::{BootInfo, MemoryKind, PHYS_MAP_BASE};

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
static KERNEL_SPACE: SpinLock<Option<PageTable>> = SpinLock::new(None);
/// Fyzicka adresa PML4 jadra (rychly pristup pri prepinani vlaken).
static KERNEL_ROOT: AtomicU64 = AtomicU64::new(0);

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
/// stranek jadra a heap.
///
/// # Safety
///
/// Volat jednou, po `arch::init_boot_cpu`. `boot_info` musi byt platny.
pub unsafe fn init(boot_info: &BootInfo) {
    assert_eq!(
        boot_info.phys_offset, PHYS_MAP_BASE,
        "loader pouziva jinou primou mapu"
    );
    // SAFETY: BootInfo je platny podle kontraktu.
    let regions = unsafe { boot_info.memory_map() };

    phys::init(regions);
    *RAM_RANGES.lock() = regions
        .iter()
        .filter(|r| r.kind != MemoryKind::Mmio && r.kind != MemoryKind::Reserved)
        .map(|r| (r.start, r.end()))
        .collect();

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
    KERNEL_ROOT.store(space.root(), Ordering::Release);
    *KERNEL_SPACE.lock() = Some(space);
}

/// Rozsahy fyzicke pameti, ktere jsou RAM (nebo je pouziva firmware),
/// pro kontrolu fyzickych objektu pameti ovladacu.
static RAM_RANGES: SpinLock<Vec<(u64, u64)>> = SpinLock::new(Vec::new());

/// Je rozsah `start..start+len` mimo RAM a pamet firmwaru (tj. registry
/// zarizeni)? Fyzicke objekty pameti smi ukazovat jen sem - jinak by
/// ovladac mohl namapovat pamet jadra nebo jinych procesu.
pub fn is_device_memory(start: u64, len: u64) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    !RAM_RANGES.lock().iter().any(|&(s, e)| start < e && s < end)
}

/// Vrati alokatoru pamet loaderu a boot services. Volat az kdyz z ni jadro
/// nic necte: po precteni ACPI tabulek (nektere firmwary maji RSDP v pameti
/// boot services) a po nahrani vlastnich GDT a IDT.
pub fn reclaim_boot_memory(boot_info: &BootInfo) {
    // SAFETY: BootInfo overeny v `init`.
    let regions = unsafe { boot_info.memory_map() };
    phys::add_regions(regions, MemoryKind::Reclaimable);
}

fn with_kernel_space<R>(f: impl FnOnce(&mut PageTable) -> R) -> R {
    f(KERNEL_SPACE
        .lock()
        .as_mut()
        .expect("mm neni inicializovana"))
}

/// Novy adresni prostor (pro proces): prazdna spodni polovina, horni
/// polovina sdilena s jadrem.
pub fn new_address_space() -> Option<PageTable> {
    with_kernel_space(|kernel| PageTable::new_sharing_kernel_half(kernel).ok())
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

/// Sprava slotu v oblasti zasobniku: kazdy slot = ochranna stranka + zasobnik.
struct StackSlots {
    /// Pocet slotu, ktere uz byly nekdy namapovane.
    used: u64,
    /// Uvolnene sloty. Zustavaji namapovane a znovu se pouziji, takze se
    /// nemusi resit zneplatneni TLB na ostatnich CPU.
    /// TODO: vracet pamet (az bude TLB shootdown).
    free: Vec<u64>,
}

static STACK_SLOTS: SpinLock<StackSlots> = SpinLock::new(StackSlots {
    used: 0,
    free: Vec::new(),
});

/// Zasobnik jadra s ochrannou strankou pod sebou: preteceni skonci page
/// faultem (a double faultem na IST), ne tichym prepsanim pameti.
pub struct KernelStack {
    slot: u64,
}

impl KernelStack {
    pub fn new() -> Option<Self> {
        if let Some(slot) = STACK_SLOTS.lock().free.pop() {
            return Some(Self { slot });
        }
        let slot = {
            let mut slots = STACK_SLOTS.lock();
            slots.used += 1;
            slots.used - 1
        };
        let bottom = Self::bottom_of(slot);
        with_kernel_space(|space| {
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
            Some(())
        })?;
        Some(Self { slot })
    }

    fn bottom_of(slot: u64) -> u64 {
        KERNEL_STACKS_BASE + slot * (KERNEL_STACK_SIZE + PAGE_SIZE) + PAGE_SIZE
    }

    /// Vrchol zasobniku (zarovnany na 16 B).
    pub fn top(&self) -> u64 {
        Self::bottom_of(self.slot) + KERNEL_STACK_SIZE
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        STACK_SLOTS.lock().free.push(self.slot);
    }
}

/// Oblast pro mapovani MMIO registru zarizeni (bez cache).
const MMIO_BASE: u64 = 0xFFFF_FD00_0000_0000;
static MMIO_NEXT: SpinLock<u64> = SpinLock::new(MMIO_BASE);

/// Namapuje fyzicky rozsah registru zarizeni bez cache a vrati jeho
/// virtualni adresu (se zachovanym posunem uvnitr stranky).
pub fn map_mmio(phys: PhysAddr, len: u64) -> VirtAddr {
    let start = phys.0 & !(PAGE_SIZE - 1);
    let end = (phys.0 + len).next_multiple_of(PAGE_SIZE);
    let virt_start = {
        let mut next = MMIO_NEXT.lock();
        let v = *next;
        *next += end - start;
        v
    };
    with_kernel_space(|space| {
        for offset in (0..end - start).step_by(PAGE_SIZE as usize) {
            space
                .map(
                    VirtAddr(virt_start + offset),
                    PhysAddr(start + offset),
                    PageSize::Size4K,
                    MapFlags::WRITE | MapFlags::GLOBAL | MapFlags::UNCACHED,
                )
                .expect("mapovani MMIO selhalo");
        }
    });
    VirtAddr(virt_start + (phys.0 - start))
}

/// Docasne identicky namapuje jednu stranku ve spodni polovine adresniho
/// prostoru jadra (start aplikacnich procesoru, ktere zacinaji bez strankovani).
pub fn map_identity_page(phys: PhysAddr) {
    with_kernel_space(|space| {
        space
            .map(
                VirtAddr(phys.0),
                phys,
                PageSize::Size4K,
                MapFlags::WRITE | MapFlags::EXEC,
            )
            .expect("identicke mapovani selhalo");
    });
}

/// Zrusi mapovani z `map_identity_page`.
pub fn unmap_identity_page(phys: PhysAddr) {
    with_kernel_space(|space| space.unmap_4k(VirtAddr(phys.0)));
}

/// Fyzicka adresa PML4 jadra.
pub fn kernel_root() -> PhysAddr {
    PhysAddr(KERNEL_ROOT.load(Ordering::Acquire))
}

/// Prelozi adresu v adresnim prostoru jadra (pro testy a ladeni).
pub fn translate(virt: VirtAddr) -> Option<PhysAddr> {
    KERNEL_SPACE.lock().as_ref()?.translate(virt)
}
