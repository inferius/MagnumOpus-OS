//! 4urovnove strankovani x86_64.
//!
//! Tabulky se alokuji z alokatoru fyzicke pameti a zapisuji pres primou
//! mapu fyzicke pameti, takze kod nepotrebuje zadne docasne mapovani.

use core::sync::atomic::{AtomicBool, Ordering};

use super::cpu;
use crate::arch::api::paging::{MapError, MapFlags, PageSize, PageTableOps};
use crate::mm::{self, PhysAddr, VirtAddr};

const PRESENT: u64 = 1 << 0;
const WRITABLE: u64 = 1 << 1;
const USER: u64 = 1 << 2;
const HUGE: u64 = 1 << 7;
const WRITE_THROUGH: u64 = 1 << 3;
const CACHE_DISABLE: u64 = 1 << 4;
const GLOBAL: u64 = 1 << 8;
const NO_EXECUTE: u64 = 1 << 63;
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

const MSR_PAT: u32 = 0x277;
/// Vychozi PAT (WB, WT, UC-, UC dvakrat) s polozkou 1 = WC (0x01).
const PAT_VALUE: u64 = 0x0007_0406_0007_0106;

static NX_ENABLED: AtomicBool = AtomicBool::new(false);
static HUGE_1G: AtomicBool = AtomicBool::new(false);

/// Uroven tabulky, ve ktere lezi list (1 = PT, 2 = PD, 3 = PDPT).
const fn level(size: PageSize) -> u32 {
    match size {
        PageSize::Size4K => 1,
        PageSize::Size2M => 2,
        PageSize::Size1G => 3,
    }
}

/// Zapne funkce CPU, na kterych strankovani jadra stavi: ochranu zapisu
/// i pro ring 0 (CR0.WP), globalni stranky (CR4.PGE), NX (EFER.NXE) a PAT
/// s polozkou WC.
///
/// # Safety
///
/// Volat jednou pri startu kazdeho CPU, pred `PageTable::activate_kernel`.
pub unsafe fn init_cpu() {
    let features = cpu::features();
    // SAFETY: WP a PGE podporuje kazde x86_64 CPU; NXE se zapina jen
    // pokud ho CPU podle CPUID podporuje. Aktualni tabulky (od loaderu)
    // jsou platne i se zapnutymi funkcemi.
    unsafe {
        cpu::write_cr0(cpu::read_cr0() | cpu::CR0_WP);
        cpu::write_cr4(cpu::read_cr4() | cpu::CR4_PGE);
        if features.nx {
            cpu::wrmsr(cpu::MSR_EFER, cpu::rdmsr(cpu::MSR_EFER) | cpu::EFER_NXE);
        }
        // SMEP: jadro nikdy nespusti kod ze stranek userspace. SMAP: jadro
        // nikdy necte ani nezapisuje pres adresy userspace (data procesu
        // kopiruje pres primou mapu fyzicke pameti), takze stac/clac neni
        // potreba a kazdy omylem vznikly pristup skonci vyjimkou.
        let mut cr4 = cpu::read_cr4();
        if features.smep {
            cr4 |= cpu::CR4_SMEP;
        }
        if features.smap {
            cr4 |= cpu::CR4_SMAP;
        }
        cpu::write_cr4(cr4);
        // PAT: polozka 1 (PWT bez PCD) misto Write-Through na
        // Write-Combining pro framebuffer (`MapFlags::WRITE_COMBINE`).
        // Ostatni polozky zustavaji vychozi (WB, UC-, UC); PWT bez PCD
        // nikdo jiny nepouziva, takze zmena nema vliv na existujici
        // mapovani. PAT ma kazde x86_64 CPU.
        cpu::wrmsr(MSR_PAT, PAT_VALUE);
    }
    NX_ENABLED.store(features.nx, Ordering::Relaxed);
    HUGE_1G.store(features.huge_1g, Ordering::Relaxed);
}

/// Podporuje CPU 1 GiB stranky?
pub fn supports_1g_pages() -> bool {
    HUGE_1G.load(Ordering::Relaxed)
}

/// `MapFlags::DMA` nema na x86 vliv: DMA je koherentni s cache.
fn leaf_bits(flags: MapFlags, size: PageSize) -> u64 {
    let mut bits = PRESENT;
    if flags.contains(MapFlags::WRITE) {
        bits |= WRITABLE;
    }
    if flags.contains(MapFlags::USER) {
        bits |= USER;
    }
    if flags.contains(MapFlags::GLOBAL) {
        bits |= GLOBAL;
    }
    if flags.contains(MapFlags::UNCACHED) {
        // PAT index 3 (PCD|PWT) = UC.
        bits |= CACHE_DISABLE | WRITE_THROUGH;
    } else if flags.contains(MapFlags::WRITE_COMBINE) {
        // PAT polozka 1 = WC (viz `init_cpu`); bit PAT (7 / 12) je 0.
        bits |= WRITE_THROUGH;
    }
    if !flags.contains(MapFlags::EXEC) && NX_ENABLED.load(Ordering::Relaxed) {
        bits |= NO_EXECUTE;
    }
    if size != PageSize::Size4K {
        bits |= HUGE;
    }
    bits
}

fn index(virt: u64, level: u32) -> usize {
    ((virt >> (12 + 9 * (level - 1))) & 0x1FF) as usize
}

fn entry_ptr(table: u64, idx: usize) -> *mut u64 {
    PhysAddr::new(table)
        .to_virt()
        .as_mut_ptr::<u64>()
        .wrapping_add(idx)
}

/// Hierarchie tabulek stranek (jeden adresni prostor).
pub struct PageTable {
    root: u64,
}

impl PageTable {
    /// Adresni prostor, ktery je prave aktivni na tomto CPU.
    pub fn current() -> Self {
        Self {
            root: cpu::read_cr3() & ADDR_MASK,
        }
    }

    /// Projde tabulky k urovni `level` a vrati ukazatel na polozku.
    /// S `create` chybejici mezilehle tabulky alokuje.
    fn walk(&mut self, virt: u64, level: u32, create: bool) -> Result<Option<*mut u64>, MapError> {
        let mut table = self.root;
        for l in (level + 1..=4).rev() {
            let e = entry_ptr(table, index(virt, l));
            // SAFETY: `e` ukazuje do tabulky tohoto adresniho prostoru,
            // dostupne pres primou mapu; tabulky vlastni tato struktura.
            let val = unsafe { *e };
            if val & PRESENT == 0 {
                if !create {
                    return Ok(None);
                }
                let new = mm::phys::alloc_frame_zeroed().ok_or(MapError::OutOfMemory)?;
                // Mezilehle urovne jsou povolne, prava urcuje list. USER
                // tu nevadi, dokud ho list nema.
                // SAFETY: viz vyse.
                unsafe { *e = new.as_u64() | PRESENT | WRITABLE | USER };
                table = new.as_u64();
            } else if val & HUGE != 0 {
                return Err(MapError::HugePageConflict);
            } else {
                table = val & ADDR_MASK;
            }
        }
        Ok(Some(entry_ptr(table, index(virt, level))))
    }
}

impl PageTableOps for PageTable {
    /// Vytvori prazdny adresni prostor.
    fn new() -> Result<Self, MapError> {
        let root = mm::phys::alloc_frame_zeroed().ok_or(MapError::OutOfMemory)?;
        Ok(Self {
            root: root.as_u64(),
        })
    }

    /// Prepne CPU na tento adresni prostor.
    ///
    /// # Safety
    ///
    /// Tabulky musi mapovat prave bezici kod, zasobnik a vsechna data,
    /// ktera se po prepnuti pouzivaji.
    unsafe fn activate_kernel(&self) {
        // SAFETY: predano volajicim.
        unsafe { cpu::write_cr3(self.root) };
    }

    /// Namapuje jednu stranku.
    fn map(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        size: PageSize,
        flags: MapFlags,
    ) -> Result<(), MapError> {
        let (v, p) = (virt.as_u64(), phys.as_u64());
        if v % size.bytes() != 0 || p % size.bytes() != 0 {
            return Err(MapError::Misaligned);
        }
        let e = self
            .walk(v, level(size), true)?
            .expect("walk s create vzdy vraci polozku");
        // SAFETY: `e` je platna polozka vracena `walk`.
        unsafe {
            if *e & PRESENT != 0 {
                return Err(MapError::AlreadyMapped);
            }
            *e = p | leaf_bits(flags, size);
        }
        Ok(())
    }

    /// Prelozi virtualni adresu na fyzickou, `None` pokud neni namapovana.
    fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        let v = virt.as_u64();
        let mut table = self.root;
        for level in (1..=4).rev() {
            // SAFETY: cteme jen polozky tabulek dostupnych pres primou mapu.
            let val = unsafe { *entry_ptr(table, index(v, level)) };
            if val & PRESENT == 0 {
                return None;
            }
            if level == 1 || val & HUGE != 0 {
                let page_size = 1u64 << (12 + 9 * (level - 1));
                return Some(PhysAddr::new(
                    (val & ADDR_MASK & !(page_size - 1)) | (v & (page_size - 1)),
                ));
            }
            table = val & ADDR_MASK;
        }
        unreachable!()
    }

    /// Novy adresni prostor se sdilenou horni polovinou (jadrem) z `kernel`.
    fn new_sharing_kernel_half(kernel: &Self) -> Result<Self, MapError> {
        let new = Self::new()?;
        for idx in 256..512 {
            // SAFETY: obe PML4 jsou platne tabulky dostupne pres primou mapu.
            unsafe { *entry_ptr(new.root, idx) = *entry_ptr(kernel.root, idx) };
        }
        Ok(new)
    }

    /// Fyzicka adresa korenove tabulky.
    fn root(&self) -> u64 {
        self.root
    }

    /// Uvolni tabulky spodni poloviny (userspace) a korenovou tabulku.
    /// Namapovane stranky samotne neuvolnuje - patri objektum pameti.
    ///
    /// # Safety
    ///
    /// Adresni prostor nesmi byt aktivni na zadnem CPU.
    unsafe fn destroy(self) {
        fn free_table(table: u64, level: u32) {
            if level > 1 {
                for idx in 0..512 {
                    // SAFETY: polozka tabulky tohoto adresniho prostoru.
                    let e = unsafe { *entry_ptr(table, idx) };
                    if e & PRESENT != 0 && e & HUGE == 0 {
                        free_table(e & ADDR_MASK, level - 1);
                    }
                }
            }
            mm::phys::free_pages(PhysAddr::new(table), 0);
        }
        for idx in 0..256 {
            // SAFETY: polozka PML4 tohoto adresniho prostoru.
            let e = unsafe { *entry_ptr(self.root, idx) };
            if e & PRESENT != 0 {
                free_table(e & ADDR_MASK, 3);
            }
        }
        mm::phys::free_pages(PhysAddr::new(self.root), 0);
    }

    /// Zrusi mapovani jedne 4 KiB stranky na tomto CPU (TLB ostatnich CPU
    /// se nezneplatni - volajici ruci, ze tam stranka neni pouzivana).
    /// Tabulky nizsich urovni zustavaji.
    fn unmap_4k(&mut self, virt: VirtAddr) {
        if let Ok(Some(e)) = self.walk(virt.as_u64(), 1, false) {
            // SAFETY: platna polozka PT vracena `walk`.
            unsafe { *e = 0 };
            cpu::invlpg(virt.as_u64());
        }
    }

    /// Predalokuje tabulky druhe urovne pro celou horni polovinu adresniho
    /// prostoru. Kazdy novy adresni prostor pak jadro sdili prostym
    /// zkopirovanim 256 polozek PML4 a zmena mapovani jadra je videt vsude.
    fn preallocate_kernel_half(&mut self) -> Result<(), MapError> {
        for idx in 256..512 {
            let e = entry_ptr(self.root, idx);
            // SAFETY: polozka PML4 tohoto adresniho prostoru.
            if unsafe { *e } & PRESENT == 0 {
                let new = mm::phys::alloc_frame_zeroed().ok_or(MapError::OutOfMemory)?;
                // SAFETY: viz vyse.
                unsafe { *e = new.as_u64() | PRESENT | WRITABLE };
            }
        }
        Ok(())
    }
}

/// Je adresa namapovana v aktualnim adresnim prostoru?
pub fn is_mapped(virt: u64) -> bool {
    PageTable::current()
        .translate(VirtAddr::new(virt))
        .is_some()
}
