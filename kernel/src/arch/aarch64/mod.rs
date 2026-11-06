//! Kostra implementace smlouvy architektury (`arch::api`) pro aarch64.
//!
//! ZATIM NEFUNGUJE: vsechno, co by melo neco delat, konci `todo!` s popisem.
//! Kostra existuje, aby se jadro pro aarch64 prekladalo (`cargo xtask check
//! --arch aarch64`) a aby prekladac hlidal, ze smlouva jde implementovat
//! i jinde nez na x86. Cil: QEMU `virt` (GICv3, PL011, PSCI, SMMUv3, UEFI
//! z edk2 s ACPI), pozdeji Raspberry Pi.
//!
//! Planovane moduly (obdoba `arch/x86_64`):
//!
//! | modul        | obsah                                                      |
//! |--------------|------------------------------------------------------------|
//! | `exceptions` | tabulka vektoru (VBAR_EL1), ulozeni registru, ESR -> chyba ABI |
//! | `gic`        | GICv3: distributor, redistributor na CPU, ITS pro MSI (LPI) |
//! | `timer`      | generic timer: CNTVCT_EL0 hodiny, CNTP_TVAL_EL0 jednorazove |
//! | `mmu`        | 4 urovne, granule 4 KiB, TTBR0 (proces) / TTBR1 (jadro), MAIR, TCR |
//! | `context`    | x19-x30, sp, FPU/SIMD v0-v31, FPCR/FPSR; prepinani         |
//! | `user`       | EL0 <-> EL1: SVC, ERET, SP_EL0, TPIDR_EL0 programu         |
//! | `psci`       | start CPU (CPU_ON pres HVC/SMC podle ACPI FADT)            |
//! | `smmu`       | SMMUv3: stream table, domeny (preklad DMA)                 |
//! | `acpi`       | MADT (GICC, GICD, GIC ITS), GTDT, IORT (RID -> StreamID/DeviceID), SPCR |
//! | `pl011`      | ladici seriova linka                                       |
//!
//! Dohodnuta pravidla pro port (dulezita, at se nemusi menit zbytek jadra):
//! - per-CPU ukazatel v TPIDR_EL1, `CpuLocal` na offsetu 8 v `PerCpu`,
//! - stranka 4 KiB (soucast ABI) a stejne rozlozeni adresniho prostoru jako
//!   na x86_64 (prima mapa od `0xFFFF_8000_0000_0000`, 48bitove adresy),
//! - cisla chyb pro `trap::user_fault` = cisla vektoru x86 (abi): napr.
//!   Data/Instruction Abort -> 14, nedefinovana instrukce -> 6, zarovnani
//!   (SP/PC alignment) -> 17, breakpoint (BRK) -> 3,
//! - hardwarove ID CPU = afinita z MPIDR_EL1 (Aff3..Aff0 slozene do u32),
//! - citac cyklu (CNTVCT_EL0) je architekturou synchronni: zadne mereni.

#![allow(
    dead_code,
    reason = "kostra: pole a typy pouzije az skutecna implementace (milnik 9)"
)]

use core::fmt;

use boot_info::BootInfo;

use crate::acpi::Platform;
use crate::arch::api::{
    self,
    paging::{MapError, MapFlags, PageSize},
};
use crate::error::Error;
use crate::mm::{PhysAddr, VirtAddr};

/// Typ, na kterem je smlouva pro aarch64 implementovana.
pub struct Aarch64;

impl api::Architecture for Aarch64 {}

/// Per-CPU data pro prechody EL0 <-> EL1 (cte je vstup vyjimky pres
/// TPIDR_EL1 na offsetu `CPU_LOCAL_OFFSET`).
#[repr(C)]
#[derive(Debug)]
pub struct CpuLocal {
    /// Vrchol zasobniku jadra aktualniho vlakna (SP_EL1 pri vstupu z EL0).
    kernel_sp: u64,
    /// Odlozeny SP_EL0 programu.
    user_sp: u64,
}

impl api::CpuLocalData for CpuLocal {
    fn for_this_cpu() -> Self {
        Self {
            kernel_sp: 0,
            user_sp: 0,
        }
    }
}

impl api::Cpu for Aarch64 {
    const NAME: &'static str = "aarch64";
    const HW_ID_NAME: &'static str = "MPIDR";
    const PC_NAME: &'static str = "pc";

    type CpuLocal = CpuLocal;
    const CPU_LOCAL_OFFSET: usize = 8;

    unsafe fn init_boot_cpu() {
        todo!(
            "aarch64: overit EL1, nastavit VBAR_EL1 (tabulka vektoru), SCTLR_EL1 (WXN, SPAN), \
             zapnout FPU/SIMD (CPACR_EL1.FPEN), PAN, MAIR_EL1 a TCR_EL1"
        )
    }

    unsafe fn init_ap_cpu() {
        todo!("aarch64: totez co init_boot_cpu pro dalsi CPU + redistributor GICv3 a casovac")
    }

    fn hw_id() -> u32 {
        todo!("aarch64: cist MPIDR_EL1 a slozit Aff3..Aff0 do u32")
    }

    fn clear_cpu_local() {
        todo!("aarch64: TPIDR_EL1 = adresa staticke nuly (jako EMPTY_CPU_LOCAL na x86)")
    }

    unsafe fn set_cpu_local(_ptr: u64) {
        todo!("aarch64: zapsat ptr do TPIDR_EL1")
    }

    fn cpu_local() -> u64 {
        todo!("aarch64: cist TPIDR_EL1 a z nej prvni slovo (ukazatel na PerCpu)")
    }

    fn halt_forever() -> ! {
        todo!("aarch64: msr daifset, #0xf a smycka wfi")
    }

    fn breakpoint() {
        todo!("aarch64: instrukce brk #0, obsluha posune ELR_EL1 o 4 a vrati se")
    }
}

impl api::Interrupts for Aarch64 {
    fn are_enabled() -> bool {
        todo!("aarch64: DAIF.I == 0")
    }

    fn disable() -> bool {
        todo!("aarch64: precist DAIF, msr daifset, #2, vratit puvodni stav I")
    }

    fn restore(_was_enabled: bool) {
        todo!("aarch64: pri true msr daifclr, #2")
    }

    fn enable() {
        todo!("aarch64: msr daifclr, #2")
    }

    fn enable_and_halt() {
        todo!(
            "aarch64: wfi s maskovanym IRQ a pak daifclr (cekajici preruseni WFI probudi \
             i kdyz je maskovane, takze nic neutece)"
        )
    }
}

/// Ulozeny kontext vlakna: SP, x19-x30 (na zasobniku nebo zde), FPU/SIMD
/// v0-v31 + FPCR/FPSR (512 + 8 B).
#[derive(Debug, Default)]
pub struct Context {
    sp: u64,
}

impl api::ThreadContext for Context {
    unsafe fn new(_stack_top: u64, _start: extern "C" fn() -> !) -> Self {
        todo!("aarch64: ramec pro switch: x30 (lr) = start, x29 = 0 (konec retezu ramcu)")
    }
}

impl api::Threads for Aarch64 {
    type Context = Context;

    unsafe fn switch(_from: *mut Context, _to: *const Context) {
        todo!("aarch64: naked fn - ulozit x19-x30, sp, v8-v15 a FPU stav, nacist z `to`, ret")
    }

    unsafe fn switch_stack(_top: u64, _f: extern "C" fn(u64) -> !, _arg: u64) -> ! {
        todo!("aarch64: mov sp, top; mov x29, #0; mov x30, #0; x0 = arg; br f")
    }

    unsafe fn set_kernel_stack(_local: &CpuLocal, _top: u64) {
        todo!("aarch64: local.kernel_sp = top (vstup z EL0 prepne SP_EL1 sem)")
    }

    unsafe fn enter_user(_entry: u64, _stack: u64, _arg0: u64, _arg1: u64) -> ! {
        todo!(
            "aarch64: SP_EL0 = stack, ELR_EL1 = entry, SPSR_EL1 = EL0t s povolenymi IRQ, \
             x0/x1 = argumenty, ostatni registry vynulovat, eret"
        )
    }
}

/// Vlastnosti casovace (do logu).
pub struct TimerInfo {
    /// Frekvence generic timeru (CNTFRQ_EL0).
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

    unsafe fn init_platform(_platform: &Platform) -> TimerInfo {
        todo!(
            "aarch64: CNTFRQ_EL0 (bez kalibrace), GICv3 distributor a redistributor BSP \
             z MADT, PPI casovace (z GTDT) povolit"
        )
    }

    fn now_ns() -> u64 {
        todo!("aarch64: CNTVCT_EL0 (po isb) prevest na ns podle CNTFRQ_EL0")
    }

    fn cycle_counter() -> u64 {
        todo!("aarch64: CNTVCT_EL0")
    }

    fn adjust_cycle_counter(_delta: i64) -> bool {
        // Citac je synchronni, jadro tuto funkci na aarch64 nevola.
        false
    }

    fn set_timer(_ns: Option<u64>) {
        todo!("aarch64: CNTP_TVAL_EL0 + CNTP_CTL_EL0.ENABLE, pri None IMASK")
    }
}

impl api::Ipi for Aarch64 {
    fn send_reschedule(_cpu_hw_id: u32) {
        todo!("aarch64: SGI (ICC_SGI1R_EL1) s cislem pro preplanovani")
    }

    fn send_halt(_cpu_hw_id: u32) {
        todo!("aarch64: SGI pro zastaveni")
    }

    fn send_tlb_flush(_cpu_hw_id: u32) {
        // Nevola se: `flush_tlb_broadcast` vraci true.
        todo!("aarch64: neni potreba (TLBI ... IS)")
    }
}

/// Tabulky stranek procesu (TTBR0_EL1). Jadro ma vlastni tabulky
/// v TTBR1_EL1, ktere se pri prepinani procesu nemeni.
pub struct PageTable {
    root: u64,
}

impl api::PageTableOps for PageTable {
    fn new() -> Result<Self, MapError> {
        todo!("aarch64: alokovat korenovou tabulku (L0)")
    }

    fn new_sharing_kernel_half(_kernel: &Self) -> Result<Self, MapError> {
        todo!("aarch64: jen nova prazdna tabulka - jadro je v TTBR1_EL1, nic se nekopiruje")
    }

    fn preallocate_kernel_half(&mut self) -> Result<(), MapError> {
        todo!("aarch64: nic - horni polovinu sdili registr TTBR1_EL1")
    }

    unsafe fn activate_kernel(&self) {
        todo!("aarch64: TTBR1_EL1 = root; dsb ish, isb (TTBR0 nastavuje switch_user_space)")
    }

    fn map(
        &mut self,
        _virt: VirtAddr,
        _phys: PhysAddr,
        _size: PageSize,
        _flags: MapFlags,
    ) -> Result<(), MapError> {
        todo!(
            "aarch64: deskriptor stranky/bloku: AF, SH = inner, AP (EL0/RO), UXN/PXN, nG pro \
             proces, AttrIndx (MAIR: normalni / Device-nGnRE pro UNCACHED, Normal-NC pro DMA na nekoherentni platforme)"
        )
    }

    fn unmap_4k(&mut self, _virt: VirtAddr) {
        todo!("aarch64: vynulovat L3 deskriptor, dsb ishst, tlbi vaae1, dsb ish, isb")
    }

    fn translate(&self, _virt: VirtAddr) -> Option<PhysAddr> {
        todo!("aarch64: prochazka L0-L3 (nebo instrukce AT S1E1R + PAR_EL1)")
    }

    fn root(&self) -> u64 {
        self.root
    }

    unsafe fn destroy(self) {
        todo!("aarch64: uvolnit tabulky spodni poloviny a korenovou tabulku")
    }
}

impl api::Memory for Aarch64 {
    type PageTable = PageTable;

    fn supports_1g_pages() -> bool {
        // Granule 4 KiB ma 1 GiB bloky na urovni 1 vzdy.
        true
    }

    unsafe fn switch_user_space(_root: Option<u64>) {
        todo!(
            "aarch64: Some(root) -> TTBR0_EL1 = root a TCR_EL1.EPD0 = 0 (+ ASID az pozdeji; \
             bez ASID tlbi vmalle1 po prepnuti); None -> TCR_EL1.EPD0 = 1 (spodni \
             polovina neni mapovana); isb"
        )
    }

    fn flush_tlb_local() {
        todo!("aarch64: dsb ishst; tlbi vmalle1; dsb nsh; isb")
    }

    fn flush_tlb_broadcast() -> bool {
        todo!("aarch64: dsb ishst; tlbi vmalle1is; dsb ish; isb; vratit true")
    }

    // Koherence DMA je vlastnost platformy: QEMU virt koherentni, Raspberry
    // Pi 4 ne (ACPI _CCA / DT `dma-coherent`). Zjisti se pri startu a ulozi
    // (napr. AtomicBool); `PageTable::map` podle ni pro `MapFlags::DMA`
    // zvoli Normal WB nebo Normal Non-cacheable (MAIR).

    fn dma_sync_for_device(_phys: u64, _len: u64) {
        todo!(
            "aarch64: na koherentni platforme nic; jinak dc civac pres primou mapu po \
             radcich cache (CTR_EL0.DminLine), dsb sy"
        )
    }

    fn dma_sync_for_cpu(_phys: u64, _len: u64) {
        todo!(
            "aarch64: na koherentni platforme nic; jinak dc civac (ne ivac - nezahodit \
             cizi data v okrajovych radcich) po radcich cache, dsb sy"
        )
    }
}

impl api::DeviceIrq for Aarch64 {
    /// LPI v GICv3 zacinaji na INTID 8192.
    const DEVICE_IRQ_FIRST: u32 = 8192;
    const DEVICE_IRQ_LAST: u32 = 8192 + 1023;

    fn msi_message(_irq: u32, device: Option<(u16, u8, u8)>) -> Result<(u64, u32), Error> {
        // Bez identity zarizeni ITS nevi, komu preruseni patri.
        if device.is_none() {
            return Err(Error::InvalidArgs);
        }
        todo!(
            "aarch64: ITS - DeviceID z IORT (RID -> DeviceID), MAPD/MAPTI prikazy, \
             adresa = GITS_TRANSLATER, data = EventID"
        )
    }
}

/// Domena SMMUv3 (kontextovy deskriptor + tabulky stranek pro jeden StreamID).
#[derive(Clone)]
pub struct Domain;

impl api::IommuDomain for Domain {
    fn map(&self, _phys: u64, _pages: u64) -> Result<(), Error> {
        todo!("aarch64: SMMUv3 - identicke mapovani stranek, CMD_TLBI_NH_VA")
    }

    fn unmap(&self, _phys: u64, _pages: u64) {
        todo!("aarch64: SMMUv3 - odmapovat a zneplatnit TLB SMMU, pockat na CMD_SYNC")
    }
}

impl api::Iommu for Aarch64 {
    type Domain = Domain;

    unsafe fn init(_platform: &Platform) {
        todo!(
            "aarch64: SMMUv3 z IORT; bez SMMU (napr. Raspberry Pi 4) zalogovat varovani \
             jako na x86 bez IOMMU"
        )
    }

    fn domain(_segment: u16, _bus: u8, _devfn: u8) -> Result<Option<Domain>, Error> {
        todo!("aarch64: StreamID z IORT, polozka stream table -> nova domena")
    }
}

/// Start CPU pres PSCI CPU_ON.
pub struct Psci;

impl api::CpuStarter for Psci {
    fn start(
        &self,
        _hw_id: u32,
        _stack_top: u64,
        _entry: extern "C" fn(u64) -> !,
        _arg: u64,
        _started: impl Fn() -> bool,
    ) -> bool {
        todo!(
            "aarch64: CPU_ON(MPIDR, fyzicka adresa vstupniho stubu, kontext) - stub zapne \
             MMU s tabulkami jadra, nastavi sp a skoci do entry(arg); pak cekat na started()"
        )
    }

    fn finish(self) {}
}

impl api::Smp for Aarch64 {
    type Starter = Psci;

    unsafe fn prepare_cpu_start(_boot_info: &BootInfo) -> Option<Psci> {
        todo!("aarch64: PSCI pres HVC nebo SMC podle ACPI FADT (ARM boot flags)")
    }
}

/// Udaje o platforme aarch64 z ACPI.
#[derive(Debug, Default)]
pub struct Aarch64Platform {
    /// Fyzicka adresa distributoru GICv3 (MADT GICD).
    pub gicd_phys: u64,
    /// Fyzicke adresy ITS (MADT GIC ITS).
    pub its_phys: alloc::vec::Vec<u64>,
}

impl fmt::Display for Aarch64Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GICD {:#x}, {} ITS", self.gicd_phys, self.its_phys.len())
    }
}

impl api::PlatformInfo for Aarch64 {
    type Info = Aarch64Platform;

    fn acpi_table(_signature: &[u8; 4], _table: &[u8], _platform: &mut Platform) {
        todo!(
            "aarch64: APIC (MADT: GICC -> cpus s MPIDR, GICD, GICR, GIC ITS), GTDT (PPI \
             casovace), IORT (mapovani RID), SPCR (seriova linka), FACP (PSCI HVC/SMC)"
        )
    }

    fn acpi_check(_platform: &Platform) -> Result<(), Error> {
        todo!("aarch64: chybi MADT nebo GICD -> Error::NotFound")
    }
}

/// Seriova linka PL011 (QEMU virt: 0x0900_0000, adresa z ACPI SPCR).
pub struct Pl011;

impl fmt::Write for Pl011 {
    fn write_str(&mut self, _s: &str) -> fmt::Result {
        todo!("aarch64: cekat na UARTFR.TXFF == 0 a zapsat do UARTDR (MMIO pres primou mapu)")
    }
}

impl api::DebugSerial for Pl011 {
    fn init(&mut self) {
        todo!("aarch64: PL011 - rychlost (UARTIBRD/FBRD), 8N1 (UARTLCR_H), zapnout (UARTCR)")
    }
}

impl api::Diagnostics for Aarch64 {
    type Serial = Pl011;

    fn debug_serial() -> Pl011 {
        Pl011
    }

    fn stack_trace(_fp: u64, _f: impl FnMut(u64)) {
        todo!("aarch64: retez ramcu pres x29: [fp] = predchozi fp, [fp + 8] = navratova adresa")
    }

    fn current_stack_trace(_f: impl FnMut(u64)) {
        todo!("aarch64: stack_trace od aktualniho x29")
    }

    fn qemu_exit(_success: bool) {
        todo!(
            "aarch64: semihosting SYS_EXIT_EXTENDED (QEMU -semihosting) nebo PSCI SYSTEM_OFF; \
             xtask musi znat vysledek"
        )
    }

    const TESTS: &'static [(&'static str, fn())] = &[];
}
