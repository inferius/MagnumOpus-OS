//! Strankovani aarch64: granule 4 KiB, 4 urovne (L0-L3), 48bitove adresy.
//!
//! Rozdeleni indexu je stejne jako na x86_64 (9 bitu na uroven), takze
//! jadro ma i tady jednu korenovou tabulku na adresni prostor a horni
//! polovinu sdili kopii polozek 256..512 (`new_sharing_kernel_half`).
//! Rozdil je v registrech: tabulky jadra jsou v TTBR1_EL1 (horni polovina,
//! nemeni se), tabulky procesu v TTBR0_EL1 (spodni polovina). Pruchod pres
//! TTBR0 pouziva jen indexy 0..256 korenove tabulky, takze zkopirovane
//! polozky jadra v tabulkach procesu nevadi.
//!
//! TLB: jadro pouziva jediny ASID 0. Stranky procesu jsou ne-globalni (nG),
//! stranky jadra globalni, takze pri prepnuti procesu staci zahodit
//! ne-globalni zaznamy (`tlbi aside1`) a jadro v TLB zustane.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use boot_info::aarch64::{ATTR_DEVICE, ATTR_NORMAL, ATTR_NORMAL_NC};

use super::pl011;
use super::regs::{dsb_ish, dsb_ishst, isb, mrs, msr};
use crate::arch::api::paging::{MapError, MapFlags, PageSize, PageTableOps};
use crate::mm::{self, PhysAddr, VirtAddr};

const VALID: u64 = 1 << 0;
/// Na L0-L2: tabulka (jinak blok); na L3: stranka.
const TABLE_OR_PAGE: u64 = 1 << 1;
const AP_EL0: u64 = 1 << 6;
const AP_RO: u64 = 1 << 7;
const SH_INNER: u64 = 3 << 8;
const AF: u64 = 1 << 10;
const NG: u64 = 1 << 11;
const PXN: u64 = 1 << 53;
const UXN: u64 = 1 << 54;
const ADDR_MASK: u64 = 0x0000_FFFF_FFFF_F000;

/// Pevna virtualni adresa ladici linky PL011 v tabulkach jadra (oblast pro
/// pevna mapovani architektury, viz rozlozeni v `crate::mm`).
pub const EARLY_UART_VA: u64 = 0xFFFF_FC00_0000_0000;

const TCR_EPD0: u64 = 1 << 7;

/// DMA je koherentni s cache CPU (QEMU virt). Na nekoherentni platforme
/// (Raspberry Pi 4) se nastavi podle popisu platformy na false a pamet pro
/// DMA se pak mapuje bez cache.
static DMA_COHERENT: AtomicBool = AtomicBool::new(true);

/// Uroven tabulky, ve ktere lezi list (0 = koren).
const fn level(size: PageSize) -> u32 {
    match size {
        PageSize::Size4K => 3,
        PageSize::Size2M => 2,
        PageSize::Size1G => 1,
    }
}

fn index(virt: u64, level: u32) -> usize {
    ((virt >> (39 - 9 * level)) & 0x1FF) as usize
}

fn entry_ptr(table: u64, idx: usize) -> *mut u64 {
    PhysAddr::new(table)
        .to_virt()
        .as_mut_ptr::<u64>()
        .wrapping_add(idx)
}

fn leaf_bits(flags: MapFlags, size: PageSize) -> u64 {
    let mut bits = VALID | AF;
    if size == PageSize::Size4K {
        bits |= TABLE_OR_PAGE;
    }
    let user = flags.contains(MapFlags::USER);
    if flags.contains(MapFlags::UNCACHED) {
        // Registry zarizeni: nikdy spustitelne.
        bits |= ATTR_DEVICE << 2 | PXN | UXN;
    } else {
        let attr = if flags.contains(MapFlags::DMA) && !DMA_COHERENT.load(Ordering::Relaxed) {
            ATTR_NORMAL_NC
        } else {
            ATTR_NORMAL
        };
        bits |= attr << 2 | SH_INNER;
        let exec = flags.contains(MapFlags::EXEC);
        // Jadro nikdy nespousti kod procesu (obdoba SMEP) a proces nikdy kod
        // jadra.
        if user {
            bits |= PXN;
            if !exec {
                bits |= UXN;
            }
        } else {
            bits |= UXN;
            if !exec {
                bits |= PXN;
            }
        }
    }
    if user {
        bits |= AP_EL0 | NG;
    }
    if !flags.contains(MapFlags::WRITE) {
        bits |= AP_RO;
    }
    bits
}

/// Zapne funkce CPU pro strankovani a pristup userspace: MAIR a TCR (stejne
/// jako loader), WXN, PAN (jadro nesmi cist ani zapisovat pres adresy
/// userspace, data procesu kopiruje pres primou mapu), FPU/SIMD pro EL0.
///
/// # Safety
///
/// Volat jednou pri startu kazdeho CPU, s tabulkami jadra (nebo loaderu),
/// ktere mapuji bezici kod.
pub unsafe fn init_cpu() {
    const SCTLR_A: u64 = 1 << 1;
    const SCTLR_DZE: u64 = 1 << 14;
    const SCTLR_UCT: u64 = 1 << 15;
    const SCTLR_NTWI: u64 = 1 << 16;
    const SCTLR_NTWE: u64 = 1 << 18;
    const SCTLR_WXN: u64 = 1 << 19;
    const SCTLR_SPAN: u64 = 1 << 23;
    const CPACR_FPEN_EL0: u64 = 3 << 20;
    let pan = (mrs!("id_aa64mmfr1_el1") >> 20) & 0xF != 0;
    let mut sctlr = mrs!("sctlr_el1");
    sctlr &= !SCTLR_A;
    sctlr |= SCTLR_DZE | SCTLR_UCT | SCTLR_NTWI | SCTLR_NTWE | SCTLR_WXN;
    if pan {
        // SPAN = 0: PAN se nastavi pri kazdem vstupu do EL1.
        sctlr &= !SCTLR_SPAN;
    }
    // SAFETY: MAIR odpovida hodnote loaderu (aktualni tabulky ji pouzivaji);
    // zmeny SCTLR nevypinaji MMU ani cache; FPU pro EL0 jen povoli instrukce
    // FPU v userspace.
    unsafe {
        msr!("mair_el1", boot_info::aarch64::MAIR);
        msr!("sctlr_el1", sctlr);
        msr!("cpacr_el1", mrs!("cpacr_el1") | CPACR_FPEN_EL0);
        isb();
        if pan {
            // msr pan, #1 (zakodovano, aby nebylo potreba rozsireni assembleru).
            asm!(".inst 0xd500419f", options(nostack));
        }
        asm!("tlbi vmalle1", "dsb nsh", "isb", options(nostack));
    }
}

/// Je adresa namapovana v aktualnim adresnim prostoru?
pub fn is_mapped(virt: u64) -> bool {
    let root = if virt >> 63 != 0 {
        mrs!("ttbr1_el1")
    } else {
        if mrs!("tcr_el1") & TCR_EPD0 != 0 {
            return false;
        }
        mrs!("ttbr0_el1")
    } & ADDR_MASK;
    PageTable { root }.translate(VirtAddr::new(virt)).is_some()
}

/// Zahodi TLB zaznamy userspace (ne-globalni, ASID 0) na tomto CPU.
pub fn flush_user_tlb_local() {
    // SAFETY: zneplatneni TLB nemeni data.
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi aside1, xzr",
            "dsb nsh",
            "isb",
            options(nostack)
        )
    };
}

/// Zahodi TLB zaznamy userspace na vsech CPU (broadcast v inner shareable
/// domene) a pocka na dokonceni.
pub fn flush_user_tlb_all() {
    // SAFETY: viz `flush_user_tlb_local`.
    unsafe {
        asm!(
            "dsb ishst",
            "tlbi aside1is, xzr",
            "dsb ish",
            "isb",
            options(nostack)
        )
    };
}

/// Prepne spodni polovinu (TTBR0) na tabulky procesu, nebo ji vypne.
///
/// # Safety
///
/// Viz `api::Memory::switch_user_space`.
pub unsafe fn switch_user_space(root: Option<u64>) {
    let tcr = mrs!("tcr_el1");
    let ttbr0 = mrs!("ttbr0_el1") & ADDR_MASK;
    match root {
        Some(root) if tcr & TCR_EPD0 == 0 && ttbr0 == root => {}
        // SAFETY: predano volajicim; jadro v TTBR1 se nemeni.
        Some(root) => unsafe {
            msr!("ttbr0_el1", root);
            msr!("tcr_el1", tcr & !TCR_EPD0);
            isb();
            flush_user_tlb_local();
        },
        None if tcr & TCR_EPD0 != 0 => {}
        // SAFETY: vypnuti spodni poloviny; jadro ji nepouziva.
        None => unsafe {
            msr!("tcr_el1", tcr | TCR_EPD0);
            isb();
            flush_user_tlb_local();
        },
    }
}

/// Nastavi koherenci DMA podle popisu platformy.
#[allow(
    dead_code,
    reason = "nastavi popis nekoherentni platformy (Raspberry Pi 4)"
)]
pub fn set_dma_coherent(coherent: bool) {
    DMA_COHERENT.store(coherent, Ordering::Relaxed);
}

pub fn dma_coherent() -> bool {
    DMA_COHERENT.load(Ordering::Relaxed)
}

/// Hierarchie tabulek stranek (jeden adresni prostor).
pub struct PageTable {
    root: u64,
}

impl PageTable {
    /// Projde tabulky k urovni `level` a vrati ukazatel na polozku.
    /// S `create` chybejici mezilehle tabulky alokuje.
    fn walk(&self, virt: u64, level: u32, create: bool) -> Result<Option<*mut u64>, MapError> {
        let mut table = self.root;
        for l in 0..level {
            let e = entry_ptr(table, index(virt, l));
            // SAFETY: `e` ukazuje do tabulky tohoto adresniho prostoru,
            // dostupne pres primou mapu; tabulky vlastni tato struktura.
            let val = unsafe { e.read_volatile() };
            if val & VALID == 0 {
                if !create {
                    return Ok(None);
                }
                let new = mm::phys::alloc_frame_zeroed().ok_or(MapError::OutOfMemory)?;
                // Nova tabulka musi byt v pameti (nuly) driv, nez na ni
                // ukaze polozka, kterou muze cist MMU jineho CPU.
                dsb_ishst();
                // SAFETY: viz vyse.
                unsafe { e.write_volatile(new.as_u64() | VALID | TABLE_OR_PAGE) };
                table = new.as_u64();
            } else if val & TABLE_OR_PAGE == 0 {
                return Err(MapError::HugePageConflict);
            } else {
                table = val & ADDR_MASK;
            }
        }
        Ok(Some(entry_ptr(table, index(virt, level))))
    }

    /// Namapuje stranku i pres sdileny odkaz (tabulky jsou ve fyzicke
    /// pameti, struktura drzi jen adresu korene).
    fn map_inner(
        &self,
        virt: u64,
        phys: u64,
        size: PageSize,
        flags: MapFlags,
    ) -> Result<(), MapError> {
        if !virt.is_multiple_of(size.bytes()) || !phys.is_multiple_of(size.bytes()) {
            return Err(MapError::Misaligned);
        }
        let e = self
            .walk(virt, level(size), true)?
            .expect("walk s create vzdy vraci polozku");
        // SAFETY: `e` je platna polozka vracena `walk`.
        unsafe {
            if e.read_volatile() & VALID != 0 {
                return Err(MapError::AlreadyMapped);
            }
            e.write_volatile(phys | leaf_bits(flags, size));
        }
        // Nova platna polozka: staci, aby ji MMU videla (neplatne polozky se
        // v TLB neuchovavaji).
        dsb_ishst();
        isb();
        Ok(())
    }
}

impl PageTableOps for PageTable {
    fn new() -> Result<Self, MapError> {
        let root = mm::phys::alloc_frame_zeroed().ok_or(MapError::OutOfMemory)?;
        Ok(Self {
            root: root.as_u64(),
        })
    }

    /// Nahraje tabulky do TTBR1 (jadro), presune ladici linku na pevnou
    /// adresu v nich a vypne spodni polovinu (identitu od loaderu).
    unsafe fn activate_kernel(&self) {
        let uart = pl011::phys();
        if uart != 0 {
            self.map_inner(
                EARLY_UART_VA,
                uart & !0xFFF,
                PageSize::Size4K,
                MapFlags::WRITE | MapFlags::GLOBAL | MapFlags::UNCACHED,
            )
            .expect("mapovani ladici linky selhalo");
        }
        dsb_ish();
        // SAFETY: predano volajicim (tabulky mapuji bezici kod, zasobnik
        // a data v horni polovine); spodni polovina od loaderu plati, dokud
        // ji nevypneme, takze linka funguje po celou dobu.
        unsafe {
            msr!("ttbr1_el1", self.root);
            isb();
            asm!("tlbi vmalle1", "dsb nsh", "isb", options(nostack));
        }
        if uart != 0 {
            pl011::set_virt(EARLY_UART_VA + (uart & 0xFFF));
        }
        // SAFETY: jadro uz nic ze spodni poloviny nepouziva.
        unsafe { switch_user_space(None) };
    }

    fn map(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        size: PageSize,
        flags: MapFlags,
    ) -> Result<(), MapError> {
        self.map_inner(virt.as_u64(), phys.as_u64(), size, flags)
    }

    fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        let v = virt.as_u64();
        let mut table = self.root;
        for level in 0..=3 {
            // SAFETY: cteme jen polozky tabulek dostupnych pres primou mapu.
            let val = unsafe { entry_ptr(table, index(v, level)).read_volatile() };
            if val & VALID == 0 {
                return None;
            }
            if level == 3 || val & TABLE_OR_PAGE == 0 {
                let page_size = 1u64 << (39 - 9 * level);
                return Some(PhysAddr::new(
                    (val & ADDR_MASK & !(page_size - 1)) | (v & (page_size - 1)),
                ));
            }
            table = val & ADDR_MASK;
        }
        None
    }

    fn new_sharing_kernel_half(kernel: &Self) -> Result<Self, MapError> {
        let new = Self::new()?;
        for idx in 256..512 {
            // SAFETY: obe korenove tabulky jsou platne a dostupne pres primou
            // mapu. Pres TTBR0 se polozky 256.. nepouziji (jen pro shodu
            // s ostatnimi architekturami a pro `translate`).
            unsafe { *entry_ptr(new.root, idx) = *entry_ptr(kernel.root, idx) };
        }
        Ok(new)
    }

    fn root(&self) -> u64 {
        self.root
    }

    unsafe fn destroy(self) {
        fn free_table(table: u64, level: u32) {
            if level < 3 {
                for idx in 0..512 {
                    // SAFETY: polozka tabulky tohoto adresniho prostoru.
                    let e = unsafe { *entry_ptr(table, idx) };
                    if e & VALID != 0 && e & TABLE_OR_PAGE != 0 {
                        free_table(e & ADDR_MASK, level + 1);
                    }
                }
            }
            mm::phys::free_pages(PhysAddr::new(table), 0);
        }
        for idx in 0..256 {
            // SAFETY: polozka korenove tabulky tohoto adresniho prostoru.
            let e = unsafe { *entry_ptr(self.root, idx) };
            if e & VALID != 0 {
                free_table(e & ADDR_MASK, 1);
            }
        }
        mm::phys::free_pages(PhysAddr::new(self.root), 0);
    }

    fn unmap_4k(&mut self, virt: VirtAddr) {
        if let Ok(Some(e)) = self.walk(virt.as_u64(), 3, false) {
            // SAFETY: platna polozka L3 vracena `walk`; po vynulovani se
            // zneplatni zaznam TLB tohoto CPU pro vsechny ASID.
            unsafe {
                e.write_volatile(0);
                asm!(
                    "dsb ishst",
                    "tlbi vaale1, {}",
                    "dsb nsh",
                    "isb",
                    in(reg) (virt.as_u64() >> 12) & ((1 << 44) - 1),
                    options(nostack)
                );
            }
        }
    }

    /// Predalokuje tabulky L1 pro celou horni polovinu: kazdy novy adresni
    /// prostor pak sdili jadro zkopirovanim 256 polozek korene.
    fn preallocate_kernel_half(&mut self) -> Result<(), MapError> {
        for idx in 256..512 {
            let e = entry_ptr(self.root, idx);
            // SAFETY: polozka korenove tabulky tohoto adresniho prostoru.
            if unsafe { *e } & VALID == 0 {
                let new = mm::phys::alloc_frame_zeroed().ok_or(MapError::OutOfMemory)?;
                // SAFETY: viz vyse.
                unsafe { *e = new.as_u64() | VALID | TABLE_OR_PAGE };
            }
        }
        dsb_ishst();
        Ok(())
    }
}
