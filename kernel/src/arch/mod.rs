//! Architekturne specificky kod kernelu.
//!
//! - `api` - smlouva: traity, ktere kazda architektura implementuje.
//! - `x86_64`, `aarch64` - implementace (prelozi se jen ta pro cil buildu).
//! - tento modul - fasada: volne funkce a typove aliasy, ktere zbytek
//!   jadra pouziva. Volani jdou staticky na [`Current`].
//!
//! Zbytek jadra nesmi pouzivat nic z `arch::x86_64` ani `arch::aarch64`
//! primo, jen tuto fasadu. Port na novou architekturu = novy podadresar
//! s implementaci `api::Architecture`; fasada ani zbytek jadra se nemeni.

pub mod api;

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
pub use self::x86_64::X86_64 as Current;

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
pub use self::aarch64::Aarch64 as Current;

use api::{
    Cpu, DebugSerial, DeviceIrq, Diagnostics, Ipi, LegacyIo, Memory, Power, Smp, Threads, Time,
};
use boot_info::BootInfo;

use crate::acpi::Platform;

// Kontrola uplnosti: aktualni architektura implementuje celou smlouvu.
const _: () = {
    const fn complete<A: api::Architecture>() {}
    complete::<Current>()
};

/// Nazev architektury (pro log).
pub const NAME: &str = <Current as Cpu>::NAME;
/// Jak se jmenuje hardwarove ID CPU (pro log).
pub const HW_ID_NAME: &str = <Current as Cpu>::HW_ID_NAME;
/// Jak se jmenuje registr s adresou instrukce (pro log).
pub const PC_NAME: &str = <Current as Cpu>::PC_NAME;

/// Per-CPU data architektury (soucast `sched::percpu::PerCpu`).
pub type CpuLocal = <Current as Cpu>::CpuLocal;
/// Offset `CpuLocal` v `PerCpu`.
pub const CPU_LOCAL_OFFSET: usize = <Current as Cpu>::CPU_LOCAL_OFFSET;
/// Ladici seriova linka.
pub type SerialPort = <Current as Diagnostics>::Serial;
/// Vlastnosti casovacu (pro log).
pub type TimerInfo = <Current as Time>::TimerInfo;
/// Architekturni cast popisu platformy.
pub type PlatformArchInfo = <Current as api::PlatformInfo>::Info;
/// Mechanismus startu dalsich CPU.
pub type CpuStarter = <Current as Smp>::Starter;

pub use api::{CpuLocalData, CpuStartResult, CpuStarter as CpuStarterOps};

/// Citac cyklu je na vsech CPU stejny bez mereni (viz `api::Time`).
pub const CYCLE_COUNTER_SYNCED: bool = <Current as Time>::CYCLE_COUNTER_SYNCED;

/// Self-testy architektury.
pub const TESTS: &[(&str, fn())] = <Current as Diagnostics>::TESTS;

/// Preruseni na aktualnim CPU.
pub mod interrupts {
    use super::Current;
    use super::api::Interrupts;

    #[inline]
    pub fn are_enabled() -> bool {
        <Current as Interrupts>::are_enabled()
    }

    /// Zakaze preruseni a vrati, zda byla predtim povolena.
    #[inline]
    pub fn disable() -> bool {
        <Current as Interrupts>::disable()
    }

    /// Obnovi stav vraceny `disable`.
    #[inline]
    pub fn restore(was_enabled: bool) {
        <Current as Interrupts>::restore(was_enabled)
    }

    #[inline]
    pub fn enable() {
        <Current as Interrupts>::enable()
    }

    /// Atomicky povoli preruseni a uspi CPU do dalsiho preruseni.
    #[inline]
    pub fn enable_and_halt() {
        <Current as Interrupts>::enable_and_halt()
    }
}

/// Kontext vlakna jadra a jeho prepnuti.
pub mod context {
    use super::Current;
    use super::api::Threads;

    pub use super::api::ThreadContext;

    /// Ulozeny kontext vlakna.
    pub type Context = <Current as Threads>::Context;

    /// Ulozi aktualni kontext do `from` a prepne na `to`.
    ///
    /// # Safety
    ///
    /// Viz `api::Threads::switch`.
    #[inline(always)]
    pub unsafe fn switch(from: *mut Context, to: *const Context) {
        // SAFETY: predano volajicim.
        unsafe { <Current as Threads>::switch(from, to) }
    }
}

/// Tabulky stranek.
pub mod paging {
    use super::Current;
    use super::api::Memory;

    pub use super::api::paging::{MapError, MapFlags, PAGE_SIZE, PageSize, PageTableOps};

    /// Tabulky stranek jednoho adresniho prostoru.
    pub type PageTable = <Current as Memory>::PageTable;

    /// Umi CPU 1 GiB stranky?
    #[inline]
    pub fn supports_1g_pages() -> bool {
        <Current as Memory>::supports_1g_pages()
    }
}

/// IOMMU (preklad DMA zarizeni).
pub mod iommu {
    use super::Current;
    use super::api::Iommu;
    use crate::acpi::Platform;
    use crate::error::Error;

    pub use super::api::IommuDomain;

    /// Domena prekladu DMA jednoho zarizeni.
    pub type Domain = <Current as Iommu>::Domain;

    /// Zapne vsechny IOMMU platformy.
    ///
    /// # Safety
    ///
    /// Volat jednou pri startu, pred spustenim ovladacu.
    pub unsafe fn init(platform: &Platform) {
        // SAFETY: predano volajicim.
        unsafe { <Current as Iommu>::init(platform) }
    }

    /// Domena zarizeni (segment, sbernice, zarizeni << 3 | funkce).
    pub fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Domain>, Error> {
        <Current as Iommu>::domain(segment, bus, devfn)
    }
}

/// Zakladni inicializace bootovaciho CPU.
///
/// # Safety
///
/// Volat jednou, co nejdrive po startu, se zakazanymi prerusenimi.
pub unsafe fn init_boot_cpu() {
    // SAFETY: predano volajicim.
    unsafe { <Current as Cpu>::init_boot_cpu() }
}

/// Inicializace casovacu a radice preruseni na bootovacim CPU.
///
/// # Safety
///
/// Volat jednou, po `init_boot_cpu` a inicializaci pameti.
pub unsafe fn init_platform(platform: &Platform) -> TimerInfo {
    // SAFETY: predano volajicim.
    unsafe { <Current as Time>::init_platform(platform) }
}

/// Inicializace dalsiho CPU.
///
/// # Safety
///
/// Volat jednou na kazdem dalsim CPU, hned po startu, se zakazanymi
/// prerusenimi.
pub unsafe fn init_ap_cpu() {
    // SAFETY: predano volajicim.
    unsafe { <Current as Cpu>::init_ap_cpu() }
}

/// Hardwarove ID aktualniho CPU.
#[inline]
pub fn cpu_hw_id() -> u32 {
    <Current as Cpu>::hw_id()
}

/// Typ jadra aktualniho CPU (`abi::sched::core_kind`).
#[inline]
pub fn core_kind() -> u8 {
    <Current as Cpu>::core_kind()
}

/// Nastavi per-CPU ukazatel na prazdna data (prvni vec pri startu CPU).
#[inline]
pub fn clear_cpu_local() {
    <Current as Cpu>::clear_cpu_local()
}

/// Nastavi per-CPU ukazatel jadra.
///
/// # Safety
///
/// Viz `api::Cpu::set_cpu_local`.
#[inline]
pub unsafe fn set_cpu_local(ptr: u64) {
    // SAFETY: predano volajicim.
    unsafe { <Current as Cpu>::set_cpu_local(ptr) }
}

/// Ukazatel nastaveny `set_cpu_local`.
#[inline]
pub fn cpu_local() -> u64 {
    <Current as Cpu>::cpu_local()
}

/// Zakaze preruseni a navzdy zastavi aktualni CPU.
#[inline]
pub fn halt_forever() -> ! {
    <Current as Cpu>::halt_forever()
}

/// Vyvola vyjimku breakpoint (test obsluhy vyjimek).
#[inline]
pub fn breakpoint() {
    <Current as Cpu>::breakpoint()
}

/// Zakladni uroven CPU programu a co z ni chybi (viz `api::Cpu::missing_baseline`).
pub fn missing_baseline(f: &mut dyn FnMut(&'static str)) -> &'static str {
    <Current as Cpu>::missing_baseline(f);
    <Current as Cpu>::BASELINE
}

/// 64 bitu z hardwaroveho generatoru CPU (viz `api::Cpu::hw_random`).
#[inline]
pub fn hw_random() -> Option<u64> {
    <Current as Cpu>::hw_random()
}

/// Prepne na novy zasobnik a zavola `f(arg)`.
///
/// # Safety
///
/// Viz `api::Threads::switch_stack`.
pub unsafe fn switch_stack(top: u64, f: extern "C" fn(u64) -> !, arg: u64) -> ! {
    // SAFETY: predano volajicim.
    unsafe { <Current as Threads>::switch_stack(top, f, arg) }
}

/// Nastavi zasobnik jadra pro vstup z userspace na tomto CPU.
///
/// # Safety
///
/// Viz `api::Threads::set_kernel_stack`.
#[inline]
pub unsafe fn set_kernel_stack(local: &CpuLocal, top: u64) {
    // SAFETY: predano volajicim.
    unsafe { <Current as Threads>::set_kernel_stack(local, top) }
}

/// Viz `api::Threads::switch_thread_pointer`.
///
/// # Safety
///
/// Viz `api::Threads::switch_thread_pointer`.
#[inline]
pub unsafe fn switch_thread_pointer(save: &core::sync::atomic::AtomicU64, next: u64) {
    // SAFETY: predano volajicim.
    unsafe { <Current as Threads>::switch_thread_pointer(save, next) }
}

/// Viz `api::Threads::set_thread_pointer`.
///
/// # Safety
///
/// Viz `api::Threads::set_thread_pointer`.
#[inline]
pub unsafe fn set_thread_pointer(value: u64) {
    // SAFETY: predano volajicim.
    unsafe { <Current as Threads>::set_thread_pointer(value) }
}

/// Kontext uzivatelskeho vlakna (`THREAD_STATE`), viz `api::UserState`.
pub mod ustate {
    use super::Current;
    use super::api::UserState;
    use super::context::Context;

    pub const GENERAL_SIZE: usize = <Current as UserState>::GENERAL_SIZE;
    /// Velikost stavu FPU (s hlavickou), viz `api::UserState::fp_size`.
    pub fn fp_size() -> usize {
        <Current as UserState>::fp_size()
    }

    /// Hlavicka stavu FPU tohoto CPU (`api::UserState::fp_header`).
    pub fn fp_header() -> abi::thread::fp::Header {
        <Current as UserState>::fp_header()
    }

    /// # Safety
    ///
    /// Viz `api::UserState::read_general`.
    pub unsafe fn read_general(stack_top: u64, tp: u64, out: &mut [u8]) {
        // SAFETY: predano volajicim.
        unsafe { <Current as UserState>::read_general(stack_top, tp, out) }
    }

    pub fn check_general(buf: &[u8]) -> bool {
        <Current as UserState>::check_general(buf)
    }

    /// # Safety
    ///
    /// Viz `api::UserState::write_general`.
    pub unsafe fn write_general(stack_top: u64, buf: &[u8]) -> u64 {
        // SAFETY: predano volajicim.
        unsafe { <Current as UserState>::write_general(stack_top, buf) }
    }

    pub fn read_fp(ctx: &Context, out: &mut [u8]) {
        <Current as UserState>::read_fp(ctx, out)
    }

    pub fn check_fp(buf: &[u8]) -> bool {
        <Current as UserState>::check_fp(buf)
    }

    /// # Safety
    ///
    /// Viz `api::UserState::load_fp`.
    pub unsafe fn load_fp(buf: &[u8]) {
        // SAFETY: predano volajicim.
        unsafe { <Current as UserState>::load_fp(buf) }
    }

    /// # Safety
    ///
    /// Viz `api::UserState::save_fp`.
    pub unsafe fn save_fp(out: &mut [u8]) {
        // SAFETY: predano volajicim.
        unsafe { <Current as UserState>::save_fp(out) }
    }

    pub const FRAME_FP: usize = <Current as UserState>::FRAME_FP;

    /// Velikost ramce vyjimky (zavisi na velikosti stavu FPU).
    pub fn frame_size() -> usize {
        abi::exception::frame::size(FRAME_FP, fp_size() - abi::thread::fp::HEADER)
    }

    pub fn general_sp_ret(buf: &[u8]) -> (u64, u64) {
        <Current as UserState>::general_sp_ret(buf)
    }

    pub fn prepare_handler_call(buf: &mut [u8], entry: u64, frame: u64) -> Option<u64> {
        <Current as UserState>::prepare_handler_call(buf, entry, frame)
    }
}

/// Prvni vstup aktualniho vlakna do userspace.
///
/// # Safety
///
/// Viz `api::Threads::enter_user`.
pub unsafe fn enter_user(entry: u64, stack: u64, arg0: u64, arg1: u64) -> ! {
    // SAFETY: predano volajicim.
    unsafe { <Current as Threads>::enter_user(entry, stack, arg0, arg1) }
}

/// Monotonni cas od startu v nanosekundach.
#[inline]
pub fn now_ns() -> u64 {
    <Current as Time>::now_ns()
}

/// Nanosekundy od hodnoty citace cyklu `counter` do startu monotonnich hodin.
pub fn ns_from_counter_to_start(counter: u64) -> i128 {
    <Current as Time>::ns_from_counter_to_start(counter)
}

/// Realny cas z hardwarovych hodin platformy (zaloha za firmware).
pub fn hardware_clock(platform: &Platform) -> Option<abi::time::WallTime> {
    <Current as Time>::hardware_clock(platform)
}

/// Zlomek sekundy (ns), ve kterem zapisovat do hardwarovych hodin.
pub const HARDWARE_CLOCK_PHASE_NS: u32 = <Current as Time>::HARDWARE_CLOCK_PHASE_NS;

/// Zapise cas do hardwarovych hodin platformy; false = nejdou nastavit.
pub fn set_hardware_clock(platform: &Platform, t: abi::time::WallTime) -> bool {
    <Current as Time>::set_hardware_clock(platform, t)
}

/// Hodnota citace cyklu aktualniho CPU.
#[inline]
pub fn cycle_counter() -> u64 {
    <Current as Time>::cycle_counter()
}

/// Posune citac cyklu tohoto CPU; false, pokud to CPU neumi.
pub fn adjust_cycle_counter(delta: i64) -> bool {
    <Current as Time>::adjust_cycle_counter(delta)
}

/// Naplanuje preruseni casovace na tomto CPU za `ns`; `None` ho zastavi.
#[inline]
pub fn set_timer(ns: Option<u64>) {
    <Current as Time>::set_timer(ns)
}

/// Pozada CPU o preplanovani.
pub fn send_reschedule(cpu_hw_id: u32) {
    <Current as Ipi>::send_reschedule(cpu_hw_id)
}

/// Zastavi CPU (pri panice).
pub fn send_halt(cpu_hw_id: u32) {
    <Current as Ipi>::send_halt(cpu_hw_id)
}

/// Pozada CPU o vyprazdneni TLB.
pub fn send_tlb_flush(cpu_hw_id: u32) {
    <Current as Ipi>::send_tlb_flush(cpu_hw_id)
}

/// Prepne adresni prostor, pokud uz neni aktivni.
///
/// # Safety
///
/// Viz `api::Memory::switch_user_space`.
#[inline]
pub unsafe fn switch_user_space(root: Option<u64>) {
    // SAFETY: predano volajicim.
    unsafe { <Current as Memory>::switch_user_space(root) }
}

/// Zapise cache do pameti pred DMA do zarizeni (viz `api::Memory`).
#[inline]
pub fn dma_sync_for_device(phys: u64, len: u64) {
    <Current as Memory>::dma_sync_for_device(phys, len)
}

/// Zahodi cache po DMA ze zarizeni (viz `api::Memory`).
#[inline]
pub fn dma_sync_for_cpu(phys: u64, len: u64) {
    <Current as Memory>::dma_sync_for_cpu(phys, len)
}

/// Zahodi TLB zaznamy userspace na tomto CPU.
#[inline]
pub fn flush_tlb_local() {
    <Current as Memory>::flush_tlb_local()
}

/// Zahodi TLB zaznamy userspace na vsech CPU bez IPI; false = neumi.
#[inline]
pub fn flush_tlb_broadcast() -> bool {
    <Current as Memory>::flush_tlb_broadcast()
}

/// Rozsah cisel preruseni pro zarizeni (vcetne).
pub const DEVICE_IRQ_FIRST: u32 = <Current as DeviceIrq>::DEVICE_IRQ_FIRST;
pub const DEVICE_IRQ_LAST: u32 = <Current as DeviceIrq>::DEVICE_IRQ_LAST;

/// Uvolni preruseni `irq` (objekt zanikl).
pub fn release_irq(irq: u32) {
    <Current as DeviceIrq>::release(irq)
}

/// Zprava MSI (adresa, data) pro preruseni `irq`.
pub fn msi_message(irq: u32, device: Option<(u16, u8, u8)>) -> crate::error::Result<(u64, u32)> {
    <Current as DeviceIrq>::msi_message(irq, device)
}

/// Cteni I/O portu (jen architektury s porty, jinak `NotSupported`).
pub fn port_read(port: u16, width: u8) -> crate::error::Result<u32> {
    <Current as LegacyIo>::port_read(port, width)
}

/// Zapis I/O portu.
pub fn port_write(port: u16, width: u8, value: u32) -> crate::error::Result<()> {
    <Current as LegacyIo>::port_write(port, width, value)
}

/// Nasmeruje pevnou linku preruseni (x86 ISA IRQ, aarch64 SPI); vraci, zda
/// je spoustena urovni.
pub fn line_route(line: u32, irq: u32, edge: bool) -> crate::error::Result<bool> {
    <Current as LegacyIo>::line_route(line, irq, edge)
}

/// Vypne (`off`) nebo restartuje pocitac pres firmware; pri uspechu se
/// nevraci.
pub fn system_power(off: bool) -> crate::error::Result<()> {
    <Current as Power>::system_power(off)
}

/// Zamaskuje / odmaskuje starou linku preruseni.
pub fn line_mask(line: u32, masked: bool) {
    <Current as LegacyIo>::line_mask(line, masked)
}

/// Uvolni starou linku preruseni.
pub fn line_release(line: u32) {
    <Current as LegacyIo>::line_release(line)
}

/// Pripravi start dalsich CPU; `None` = jadro pobezi na jednom CPU.
///
/// # Safety
///
/// Volat jednou, na bootovacim CPU, po inicializaci pameti.
pub unsafe fn prepare_cpu_start(boot_info: &BootInfo) -> Option<CpuStarter> {
    // SAFETY: predano volajicim.
    unsafe { <Current as Smp>::prepare_cpu_start(boot_info) }
}

/// Zpracuje ACPI tabulku, kterou obecna cast nezna.
pub fn acpi_table(signature: &[u8; 4], table: &[u8], platform: &mut Platform) {
    <Current as api::PlatformInfo>::acpi_table(signature, table, platform)
}

/// Kontrola popisu platformy po precteni ACPI.
pub fn acpi_check(platform: &Platform) -> crate::error::Result<()> {
    <Current as api::PlatformInfo>::acpi_check(platform)
}

/// Prvni krok na bootovacim CPU: prevzit od loaderu ladici linku apod.
pub fn init_early(boot_info: &BootInfo) {
    <Current as Diagnostics>::init_early(boot_info)
}

/// Vrati ladici seriovou linku (bezpecne opakovane i v panic handleru).
pub fn debug_serial() -> SerialPort {
    <Current as Diagnostics>::debug_serial()
}

/// Nastavi ladici seriovou linku.
pub fn init_debug_serial(serial: &mut SerialPort) {
    DebugSerial::init(serial)
}

/// Vypis zasobniku od ukazatele ramce `fp`.
pub fn stack_trace(fp: u64, f: impl FnMut(u64)) {
    <Current as Diagnostics>::stack_trace(fp, f)
}

/// Vypis zasobniku od mista volani.
#[inline(always)]
pub fn current_stack_trace(f: impl FnMut(u64)) {
    <Current as Diagnostics>::current_stack_trace(f)
}

/// Ukonci QEMU s vysledkem testu (bez testovaciho zarizeni se vrati).
pub fn qemu_exit(success: bool) {
    <Current as Diagnostics>::qemu_exit(success)
}
