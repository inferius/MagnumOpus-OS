//! Implementace smlouvy architektury (`arch::api`) pro aarch64.
//!
//! Cil: QEMU `virt` (GICv3 s ITS, PL011, PSCI, UEFI z edk2 s ACPI), pozdeji
//! Raspberry Pi. Jadro bezi v EL1, programy v EL0.
//!
//! Prehled (podrobnosti v modulech):
//! - CPU: tabulka vyjimek (`exceptions`), per-CPU data pres TPIDR_EL1,
//!   FPU/SIMD jen pro programy (`context`),
//! - preruseni: GICv3 (`gic`), MSI zarizeni pres ITS (`its`), IPI = SGI,
//! - cas: generic timer (`timer`), citac je synchronni na vsech CPU,
//! - pamet: 4 urovne, granule 4 KiB, TTBR1 jadro / TTBR0 proces (`mmu`),
//! - start dalsich CPU: PSCI CPU_ON (`smp`),
//! - platforma: ACPI MADT a FADT (`acpi`),
//! - ladeni: PL011 (`pl011`), konec testu pres semihosting (`semihosting`).
//!
//! Dohodnuta pravidla pro port (dulezita, at se nemusi menit zbytek jadra):
//! - per-CPU ukazatel v TPIDR_EL1, `CpuLocal` na offsetu 8 v `PerCpu`,
//! - stejne rozlozeni adresniho prostoru jako na x86_64 (prima mapa od
//!   `0xFFFF_8000_0000_0000`, 48bitove adresy), ramec jadra 4 KiB,
//! - cisla chyb pro `trap::user_fault` = cisla vektoru x86 (abi):
//!   Data/Instruction Abort -> 14, nedefinovana instrukce -> 6, zarovnani
//!   (SP/PC) -> 17, BRK -> 3, ostatni -> 13,
//! - hardwarove ID CPU = afinita z MPIDR_EL1 (Aff3..Aff0 slozene do u32),
//! - syscall: `svc #0`, cislo v x8, argumenty x0-x5, vysledek v x0.
//!
//! - IOMMU: SMMUv3 (`smmu`, popis v IORT).
//!
//! Zatim chybi: RTC (cas jen z UEFI pri startu).

pub mod acpi;
mod context;
mod exceptions;
mod gic;
mod its;
mod mmio;
mod mmu;
mod pl011;
mod regs;
mod semihosting;
mod smmu;
mod smp;
mod tests;
mod timer;

use core::arch::asm;
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use boot_info::{BootInfo, SerialKind};

use crate::acpi::Platform;
use crate::arch::api;
use crate::error::Error;
use crate::log;
use regs::{mpidr_to_hw_id, mrs, msr};

/// Typ, na kterem je smlouva pro aarch64 implementovana.
pub struct Aarch64;

impl api::Architecture for Aarch64 {}

/// Posledni cislo preruseni zarizeni (LPI 8192..=9215, 1024 preruseni).
pub const DEVICE_IRQ_LAST: u32 = gic::LPI_FIRST + 1023;

/// Per-CPU data pro prechody EL0 <-> EL1 (v `PerCpu` na offsetu
/// `CPU_LOCAL_OFFSET`).
#[repr(C)]
#[derive(Debug)]
pub struct CpuLocal {
    /// Vrchol zasobniku jadra aktualniho vlakna (pro prvni vstup do EL0).
    kernel_sp: AtomicU64,
}

impl api::CpuLocalData for CpuLocal {
    fn for_this_cpu() -> Self {
        Self {
            kernel_sp: AtomicU64::new(0),
        }
    }
}

/// Prazdna per-CPU data: prvni slovo 0 = "CPU jeste neni zaregistrovane".
static EMPTY_CPU_LOCAL: u64 = 0;

/// PSCI pres HVC (z FADT), pro start CPU.
static PSCI_HVC: AtomicBool = AtomicBool::new(true);
static PSCI_PRESENT: AtomicBool = AtomicBool::new(false);

impl api::Cpu for Aarch64 {
    const NAME: &'static str = "aarch64";
    const HW_ID_NAME: &'static str = "MPIDR";
    const PC_NAME: &'static str = "pc";

    type CpuLocal = CpuLocal;
    const CPU_LOCAL_OFFSET: usize = 8;

    /// Tabulka vyjimek, funkce MMU (MAIR, WXN, PAN), FPU pro programy,
    /// hodiny.
    unsafe fn init_boot_cpu() {
        // SAFETY: predano volajicim (jednou, bootovaci CPU, bez preruseni).
        unsafe {
            exceptions::init_cpu();
            mmu::init_cpu();
        }
        timer::init();
    }

    unsafe fn init_ap_cpu() {
        // SAFETY: predano volajicim.
        unsafe {
            exceptions::init_cpu();
            mmu::init_cpu();
        }
        gic::init_cpu(timer::INTID);
    }

    fn hw_id() -> u32 {
        mpidr_to_hw_id(mrs!("mpidr_el1"))
    }

    fn core_kind() -> u8 {
        schedcore::topo::arm_core_kind(mrs!("midr_el1"))
    }

    fn clear_cpu_local() {
        // SAFETY: staticka promenna zije navzdy a jeji prvni slovo je 0.
        unsafe { msr!("tpidr_el1", &raw const EMPTY_CPU_LOCAL as u64) };
    }

    unsafe fn set_cpu_local(ptr: u64) {
        // SAFETY: predano volajicim.
        unsafe { msr!("tpidr_el1", ptr) };
    }

    #[inline]
    fn cpu_local() -> u64 {
        let base = mrs!("tpidr_el1");
        // SAFETY: TPIDR_EL1 ukazuje na per-CPU data, jejichz prvni slovo je
        // ukazatel na ne same, nebo na nulove EMPTY_CPU_LOCAL.
        unsafe { (base as *const u64).read() }
    }

    fn halt_forever() -> ! {
        halt_forever()
    }

    fn breakpoint() {
        // SAFETY: brk vyvola vyjimku, obsluha ji zaloguje a vrati se za ni.
        unsafe { asm!("brk #0", options(nomem, nostack)) };
    }
}

/// Zakaze preruseni a navzdy zastavi aktualni CPU.
fn halt_forever() -> ! {
    loop {
        // SAFETY: maskovani preruseni a cekani nemeni pamet.
        unsafe { asm!("msr daifset, #0xf", "wfi", options(nomem, nostack)) };
    }
}

/// Povoleni a zakazani preruseni (bit I v DAIF).
mod interrupts {
    use core::arch::asm;

    const DAIF_I: u64 = 1 << 7;

    pub fn are_enabled() -> bool {
        super::regs::mrs!("daif") & DAIF_I == 0
    }

    pub fn disable() -> bool {
        let was = are_enabled();
        // SAFETY: zamaskuje IRQ na tomto CPU.
        unsafe { asm!("msr daifset, #2", options(nomem, nostack)) };
        was
    }

    pub fn enable() {
        // SAFETY: povoli IRQ; tabulka vyjimek je nahrana pri startu CPU.
        unsafe { asm!("msr daifclr, #2", options(nomem, nostack)) };
    }

    /// WFI probudi i zamaskovane cekajici preruseni, takze nejdriv uspat,
    /// pak povolit: preruseni mezi tim neutece.
    pub fn enable_and_halt() {
        // SAFETY: viz `enable`; wfi jen ceka.
        unsafe { asm!("wfi", "msr daifclr, #2", options(nomem, nostack)) };
    }
}

impl api::Interrupts for Aarch64 {
    #[inline]
    fn are_enabled() -> bool {
        interrupts::are_enabled()
    }

    #[inline]
    fn disable() -> bool {
        interrupts::disable()
    }

    #[inline]
    fn restore(was_enabled: bool) {
        if was_enabled {
            interrupts::enable();
        }
    }

    #[inline]
    fn enable() {
        interrupts::enable()
    }

    #[inline]
    fn enable_and_halt() {
        interrupts::enable_and_halt()
    }
}

impl api::Threads for Aarch64 {
    type Context = context::Context;

    #[inline(always)]
    unsafe fn switch(from: *mut Self::Context, to: *const Self::Context) {
        // SAFETY: predano volajicim.
        unsafe { context::switch(from, to) }
    }

    unsafe fn switch_stack(top: u64, f: extern "C" fn(u64) -> !, arg: u64) -> ! {
        // SAFETY: predano volajicim.
        unsafe { context::switch_stack(top, f, arg) }
    }

    #[inline]
    unsafe fn set_kernel_stack(local: &CpuLocal, top: u64) {
        local.kernel_sp.store(top, Ordering::Relaxed);
    }

    /// TPIDR_EL0 smi program zapsat sam (`msr tpidr_el0`), proto se pred
    /// prepnutim ulozi. Jadro ho nepouziva (per-CPU data jsou v TPIDR_EL1).
    #[inline]
    unsafe fn switch_thread_pointer(save: &core::sync::atomic::AtomicU64, next: u64) {
        save.store(mrs!("tpidr_el0"), Ordering::Relaxed);
        // SAFETY: TPIDR_EL0 je jen registr pro program, jadro ho necte.
        unsafe { msr!("tpidr_el0", next) };
    }

    #[inline]
    unsafe fn set_thread_pointer(value: u64) {
        // SAFETY: viz `switch_thread_pointer`.
        unsafe { msr!("tpidr_el0", value) };
    }

    unsafe fn enter_user(entry: u64, stack: u64, arg0: u64, arg1: u64) -> ! {
        let local = <Self as api::Cpu>::cpu_local() + <Self as api::Cpu>::CPU_LOCAL_OFFSET as u64;
        // SAFETY: per-CPU data aktualniho CPU (kontrakt `set_cpu_local`).
        let top = unsafe {
            (*(local as *const CpuLocal))
                .kernel_sp
                .load(Ordering::Relaxed)
        };
        // SAFETY: predano volajicim; `top` nastavil `set_kernel_stack`.
        unsafe { context::enter_user(entry, stack, arg0, arg1, top) }
    }
}

/// Vlastnosti casovace (do logu).
pub struct TimerInfo {
    pub hz: u64,
}

impl fmt::Display for TimerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "generic timer {} MHz", self.hz / 1_000_000)
    }
}

impl api::Time for Aarch64 {
    type TimerInfo = TimerInfo;

    /// CNTVCT_EL0 je podle architektury stejny na vsech CPU.
    const CYCLE_COUNTER_SYNCED: bool = true;

    /// GICv3 (distributor, ITS, redistributor bootovaciho CPU) a casovac.
    unsafe fn init_platform(platform: &Platform) -> TimerInfo {
        let a = &platform.arch;
        let regions: alloc::vec::Vec<(u64, u64)> = if a.gicr.is_empty() {
            a.gicc_gicr.iter().map(|&b| (b, 0x2_0000)).collect()
        } else {
            a.gicr.clone()
        };
        gic::init_distributor(a.gicd, &regions);
        gic::init_cpu(timer::INTID);
        match a.its.first() {
            Some(&its) => its::init(its),
            None => log::warn!("platforma nema ITS: zarizeni nebudou mit preruseni (MSI)"),
        }
        PSCI_HVC.store(a.psci_hvc, Ordering::Relaxed);
        PSCI_PRESENT.store(a.psci, Ordering::Relaxed);
        TimerInfo { hz: timer::hz() }
    }

    #[inline]
    fn now_ns() -> u64 {
        timer::now_ns()
    }

    #[inline]
    fn cycle_counter() -> u64 {
        timer::counter()
    }

    fn adjust_cycle_counter(_delta: i64) -> bool {
        // Citac je synchronni, jadro tuto funkci na aarch64 nevola.
        false
    }

    fn set_timer(ns: Option<u64>) {
        timer::set_oneshot(ns);
    }

    fn ns_from_counter_to_start(counter: u64) -> i128 {
        timer::ns_from_counter_to_start(counter)
    }

    fn hardware_clock(_platform: &Platform) -> Option<abi::time::WallTime> {
        // RTC (PL031, EFI RTC za behu) zatim neni: jen cas z UEFI.
        None
    }

    const HARDWARE_CLOCK_PHASE_NS: u32 = 0;

    fn set_hardware_clock(_platform: &Platform, _t: abi::time::WallTime) -> bool {
        false
    }
}

impl api::Ipi for Aarch64 {
    fn send_reschedule(cpu_hw_id: u32) {
        gic::send_sgi(cpu_hw_id, gic::SGI_RESCHEDULE);
    }

    fn send_halt(cpu_hw_id: u32) {
        gic::send_sgi(cpu_hw_id, gic::SGI_HALT);
    }

    fn send_tlb_flush(cpu_hw_id: u32) {
        gic::send_sgi(cpu_hw_id, gic::SGI_TLB_FLUSH);
    }
}

impl api::Memory for Aarch64 {
    type PageTable = mmu::PageTable;

    fn supports_1g_pages() -> bool {
        // Granule 4 KiB ma 1 GiB bloky na urovni 1 vzdy.
        true
    }

    unsafe fn switch_user_space(root: Option<u64>) {
        // SAFETY: predano volajicim.
        unsafe { mmu::switch_user_space(root) }
    }

    fn flush_tlb_local() {
        mmu::flush_user_tlb_local();
    }

    /// TLBI ... IS zneplatni TLB vsech CPU bez IPI.
    fn flush_tlb_broadcast() -> bool {
        mmu::flush_user_tlb_all();
        true
    }

    fn dma_sync_for_device(phys: u64, len: u64) {
        if !mmu::dma_coherent() {
            let virt = crate::mm::PhysAddr::new(phys).to_virt().as_u64();
            regs::dcache_clean_invalidate(virt, len);
        }
    }

    fn dma_sync_for_cpu(phys: u64, len: u64) {
        if !mmu::dma_coherent() {
            let virt = crate::mm::PhysAddr::new(phys).to_virt().as_u64();
            regs::dcache_clean_invalidate(virt, len);
        }
    }
}

/// aarch64 nema I/O porty. Pevne linky jsou SPI pres GIC (zarizeni na
/// desce bez MSI, napr. Generic Event Device z ACPI): jadro si pamatuje,
/// ktere cislo preruseni objektu (z rozsahu zarizeni) k SPI patri.
impl api::LegacyIo for Aarch64 {
    fn port_read(_port: u16, _width: u8) -> Result<u32, Error> {
        Err(Error::NotSupported)
    }

    fn port_write(_port: u16, _width: u8, _value: u32) -> Result<(), Error> {
        Err(Error::NotSupported)
    }

    fn line_route(line: u32, irq: u32, edge: bool) -> Result<bool, Error> {
        if !(gic::SPI_FIRST..=gic::spi_last_supported()).contains(&line) {
            return Err(Error::InvalidArgs);
        }
        let slot = &SPI_IRQ[line as usize];
        if slot.compare_exchange(0, irq, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return Err(Error::AlreadyExists);
        }
        gic::configure_spi(line, edge, crate::trap::irq_target_hw_id());
        gic::set_spi_enabled(line, true);
        Ok(!edge)
    }

    fn line_mask(line: u32, masked: bool) {
        if (gic::SPI_FIRST..=gic::SPI_LAST).contains(&line) {
            gic::set_spi_enabled(line, !masked);
        }
    }

    fn line_release(line: u32) {
        if (gic::SPI_FIRST..=gic::SPI_LAST).contains(&line) {
            gic::set_spi_enabled(line, false);
            SPI_IRQ[line as usize].store(0, Ordering::Release);
        }
    }
}

/// Cislo preruseni objektu (z rozsahu zarizeni) pro kazde SPI; 0 = linka
/// nema objekt.
static SPI_IRQ: [core::sync::atomic::AtomicU32; gic::SPI_LAST as usize + 1] =
    [const { core::sync::atomic::AtomicU32::new(0) }; gic::SPI_LAST as usize + 1];

/// Cislo preruseni objektu pro SPI `intid` (obsluha preruseni).
pub(crate) fn spi_irq(intid: u32) -> Option<u32> {
    SPI_IRQ.get(intid as usize).map(|a| a.load(Ordering::Acquire)).filter(|&n| n != 0)
}

/// Vypnuti a restart pres PSCI (SYSTEM_OFF, SYSTEM_RESET).
impl api::Power for Aarch64 {
    fn system_power(off: bool) -> Result<(), Error> {
        if !PSCI_PRESENT.load(Ordering::Relaxed) {
            return Err(Error::NotSupported);
        }
        smp::psci_system(PSCI_HVC.load(Ordering::Relaxed), off);
        Err(Error::NotSupported)
    }
}

impl api::DeviceIrq for Aarch64 {
    const DEVICE_IRQ_FIRST: u32 = gic::LPI_FIRST;
    const DEVICE_IRQ_LAST: u32 = DEVICE_IRQ_LAST;

    fn msi_message(irq: u32, device: Option<(u16, u8, u8)>) -> Result<(u64, u32), Error> {
        its::msi_message(irq, device)
    }

    fn release(irq: u32) {
        its::release(irq);
    }
}

impl api::Iommu for Aarch64 {
    type Domain = alloc::sync::Arc<smmu::SmmuDomain>;

    unsafe fn init(platform: &Platform) {
        if let Some((base, rid, sid)) = platform.arch.smmu {
            smmu::init(base, (rid, sid));
        }
        if !smmu::active() {
            log::warn!(
                "IOMMU neni: zarizeni smi pres DMA zapisovat kamkoli do pameti (chyba ovladace muze poskodit system)"
            );
        }
    }

    fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Self::Domain>, Error> {
        smmu::domain(segment, bus, devfn)
    }
}

impl api::Smp for Aarch64 {
    type Starter = smp::Psci;

    unsafe fn prepare_cpu_start(_boot_info: &BootInfo) -> Option<smp::Psci> {
        if !PSCI_PRESENT.load(Ordering::Relaxed) {
            log::warn!("firmware nehlasi PSCI, bezim jen na jednom CPU");
            return None;
        }
        smp::prepare(PSCI_HVC.load(Ordering::Relaxed))
    }
}

impl api::PlatformInfo for Aarch64 {
    type Info = acpi::Aarch64Platform;

    fn acpi_table(signature: &[u8; 4], table: &[u8], platform: &mut Platform) {
        acpi::table(signature, table, platform);
    }

    fn acpi_check(platform: &Platform) -> Result<(), Error> {
        acpi::check(platform)
    }
}

impl api::Diagnostics for Aarch64 {
    type Serial = pl011::Pl011;

    /// Adresa PL011 od loaderu (z ACPI SPCR), identicky namapovana.
    fn init_early(boot_info: &BootInfo) {
        if boot_info.magic == boot_info::BOOT_INFO_MAGIC
            && boot_info.version == boot_info::BOOT_INFO_VERSION
            && boot_info.debug_serial.kind == SerialKind::Pl011
        {
            pl011::set_phys(boot_info.debug_serial.base);
        }
    }

    fn debug_serial() -> pl011::Pl011 {
        pl011::Pl011
    }

    fn stack_trace(fp: u64, f: impl FnMut(u64)) {
        stack_trace(fp, f);
    }

    #[inline(always)]
    fn current_stack_trace(f: impl FnMut(u64)) {
        let fp: u64;
        // SAFETY: cteni x29 (ukazatel ramce).
        unsafe { asm!("mov {}, x29", out(reg) fp, options(nomem, nostack, preserves_flags)) };
        stack_trace(fp, f);
    }

    /// Semihosting SYS_EXIT (QEMU s `-semihosting-config enable=on`):
    /// kod 33 = uspech, 35 = selhani (stejne jako isa-debug-exit na x86).
    /// Nula se nepouziva: tu vrati QEMU i pri obycejnem vypnuti, test by
    /// pak "prosel" i bez vysledku od jadra. Mimo testovaci build se nevola
    /// (bez semihostingu by instrukce vyvolala vyjimku).
    fn qemu_exit(success: bool) {
        if cfg!(feature = "qemu-test") {
            semihosting::exit(if success { 33 } else { 35 });
        }
    }

    const TESTS: &'static [(&'static str, fn())] = tests::TESTS;
}

/// Projde retez ramcu (x29: [fp] = predchozi fp, [fp + 8] = navratova
/// adresa) a pro kazdou navratovou adresu zavola `f`. Cte jen namapovanou
/// pamet jadra.
fn stack_trace(mut fp: u64, mut f: impl FnMut(u64)) {
    const MAX_DEPTH: usize = 32;
    for _ in 0..MAX_DEPTH {
        if fp == 0
            || fp % 8 != 0
            || fp < 0xFFFF_8000_0000_0000
            || !mmu::is_mapped(fp)
            || !mmu::is_mapped(fp + 8)
        {
            return;
        }
        // SAFETY: obe slova ramce jsou zarovnana a namapovana (overeno vyse).
        let (next, ret) = unsafe { (*(fp as *const u64), *((fp + 8) as *const u64)) };
        if ret == 0 {
            return;
        }
        f(ret);
        fp = next;
    }
}
