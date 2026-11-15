//! Implementace smlouvy architektury (`arch::api`) pro x86_64.
//!
//! Prehled (podrobnosti v jednotlivych modulech):
//! - CPU: GDT/TSS (`gdt`), IDT a obsluha vyjimek (`idt`), per-CPU data pres
//!   GS bazi, FPU/SSE (`context`), syscall/sysret MSR (`user`),
//! - preruseni a casovace: lokalni APIC/x2APIC (`apic`), HPET jako
//!   reference pro kalibraci (`hpet`), TSC jako hodiny (`time`),
//! - pamet: 4urovnove strankovani (`paging`), IOMMU VT-d a AMD-Vi (`iommu`),
//! - start dalsich CPU: INIT-SIPI s trampolinou pod 1 MiB (`smp`),
//! - platforma: ACPI MADT, HPET, DMAR, IVRS (`acpi`),
//! - ladeni: COM1 (`serial`), isa-debug-exit.

pub mod acpi;
mod apic;
mod ioapic;
mod context;
mod cpu;
mod gdt;
mod hpet;
mod idt;
mod interrupts;
mod iommu;
mod paging;
mod rtc;
mod serial;
mod smp;
mod tests;
mod time;
mod user;
mod ustate;

use core::arch::asm;
use core::fmt;

use boot_info::BootInfo;

use crate::acpi::Platform;
use crate::arch::api;
use crate::error::Error;

use serial::SerialPort;

/// Typ, na kterem je smlouva pro x86_64 implementovana.
pub struct X86_64;

impl api::Architecture for X86_64 {}

/// Port prvni seriove linky (COM1). Na UEFI PC je to stale standardni adresa
/// a QEMU ji vystavuje na `-serial`.
const COM1: u16 = 0x3F8;

const MSR_GS_BASE: u32 = 0xC000_0101;

/// Prazdna per-CPU data: gs:[0] = 0, tedy "CPU jeste neni zaregistrovane".
static EMPTY_CPU_LOCAL: u64 = 0;

impl api::Cpu for X86_64 {
    const NAME: &'static str = "x86_64";
    const HW_ID_NAME: &'static str = "APIC ID";
    const PC_NAME: &'static str = "rip";

    type CpuLocal = user::CpuLocal;
    const CPU_LOCAL_OFFSET: usize = user::CPU_LOCAL_OFFSET;

    /// GDT, TSS, IDT, funkce pro strankovani, FPU, syscall. Starsi radic
    /// preruseni 8259 se uplne zamaskuje, preruseni chodi pres APIC.
    unsafe fn init_boot_cpu() {
        // SAFETY: predano volajicim (jednou, BSP, bez preruseni).
        unsafe {
            gdt::init_bsp();
            // Nacteni selektoru GS v gdt vynuluje jeho bazi: znovu nastavit
            // prazdna per-CPU data, jinak by je cetlo logovani z adresy 0.
            Self::clear_cpu_local();
            idt::init();
            idt::load();
            paging::init_cpu();
            context::init_cpu(true);
            log_fpu();
            user::init_cpu();
            // 8259 PIC: zamaskovat vsechny linky na master i slave.
            cpu::outb(0x21, 0xFF);
            cpu::outb(0xA1, 0xFF);
        }
    }

    unsafe fn init_ap_cpu() {
        // SAFETY: predano volajicim.
        unsafe {
            gdt::init_ap();
            Self::clear_cpu_local();
            idt::load();
            paging::init_cpu();
            context::init_cpu(false);
            user::init_cpu();
            apic::init_cpu(false, 0);
        }
    }

    /// ID lokalniho APIC.
    fn hw_id() -> u32 {
        apic::id()
    }

    fn core_kind() -> u8 {
        let max = cpu::cpuid(0, 0).eax;
        let leaf7 = if max >= 7 { cpu::cpuid(7, 0).edx } else { 0 };
        let leaf1a = if max >= 0x1A { cpu::cpuid(0x1A, 0).eax } else { 0 };
        schedcore::topo::x86_core_kind(max, leaf7, leaf1a)
    }

    fn hw_random() -> Option<u64> {
        cpu::hw_random()
    }

    const BASELINE: &'static str = "x86-64-v2";

    fn missing_baseline(f: &mut dyn FnMut(&'static str)) {
        cpu::missing_v2(f)
    }

    fn clear_cpu_local() {
        // SAFETY: staticka promenna zije navzdy a jeji prvni slovo je 0.
        unsafe { cpu::wrmsr(MSR_GS_BASE, &raw const EMPTY_CPU_LOCAL as u64) };
    }

    unsafe fn set_cpu_local(ptr: u64) {
        // SAFETY: predano volajicim.
        unsafe { cpu::wrmsr(MSR_GS_BASE, ptr) };
    }

    /// Cte se pres gs:[0], bez MSR.
    #[inline]
    fn cpu_local() -> u64 {
        let v;
        // SAFETY: GS baze ukazuje na per-CPU data, jejichz prvni slovo je
        // ukazatel na ne same (kontrakt `set_cpu_local`).
        unsafe { asm!("mov {}, gs:[0]", out(reg) v, options(readonly, nostack, preserves_flags)) };
        v
    }

    fn halt_forever() -> ! {
        halt_forever()
    }

    fn breakpoint() {
        // SAFETY: int3 jen vyvola #BP, ktery obsluha vrati zpet sem.
        unsafe { asm!("int3", options(nomem, nostack)) };
    }
}

/// Zakaze preruseni a navzdy zastavi aktualni CPU.
fn halt_forever() -> ! {
    loop {
        // SAFETY: cli + hlt jen zastavi toto CPU, nemeni zadny stav pameti.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
    }
}

impl api::Interrupts for X86_64 {
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
        interrupts::restore(was_enabled)
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

impl api::Threads for X86_64 {
    type Context = context::Context;

    #[inline(always)]
    unsafe fn switch(from: *mut Self::Context, to: *const Self::Context) {
        // SAFETY: predano volajicim.
        unsafe { context::switch(from, to) }
    }

    unsafe fn switch_stack(top: u64, f: extern "C" fn(u64) -> !, arg: u64) -> ! {
        // SAFETY: predano volajicim. `push 0` = falesna navratova adresa, takze
        // plati zarovnani podle ABI a vypis zasobniku skonci na nule.
        unsafe {
            asm!(
                "mov rsp, {top}",
                "xor ebp, ebp",
                "push 0",
                "jmp {f}",
                top = in(reg) top,
                f = in(reg) f,
                in("rdi") arg,
                options(noreturn)
            )
        }
    }

    #[inline]
    unsafe fn set_kernel_stack(local: &Self::CpuLocal, top: u64) {
        // SAFETY: predano volajicim.
        unsafe { user::set_kernel_stack(local, top) }
    }

    /// Program `FS_BASE` sam zmenit nemuze (CR4.FSGSBASE je vypnute;
    /// nacteni selektoru FS bazi jen vynuluje), proto se neuklada - plati
    /// hodnota z `THREAD_SET_POINTER` ulozena ve vlaknu.
    #[inline]
    unsafe fn switch_thread_pointer(_save: &core::sync::atomic::AtomicU64, next: u64) {
        // SAFETY: predano volajicim (kanonicka adresa userspace).
        unsafe { cpu::wrmsr(cpu::MSR_FS_BASE, next) }
    }

    #[inline]
    unsafe fn set_thread_pointer(value: u64) {
        // SAFETY: predano volajicim.
        unsafe { cpu::wrmsr(cpu::MSR_FS_BASE, value) }
    }

    const HAS_SECOND_POINTER: bool = true;

    /// GS programu: v jadre (po `swapgs`) lezi v `KERNEL_GS_BASE`, navrat
    /// do userspace ho `swapgs` prohodi zpet.
    #[inline]
    unsafe fn set_second_pointer(value: u64) {
        // SAFETY: predano volajicim (kanonicka adresa userspace, jadro).
        unsafe { user::set_user_gs(value) }
    }

    unsafe fn enter_user(entry: u64, stack: u64, arg0: u64, arg1: u64) -> ! {
        // SAFETY: predano volajicim.
        unsafe { user::enter_user(entry, stack, arg0, arg1) }
    }
}

/// Informace o casovacich zjistene pri inicializaci platformy.
pub struct TimerInfo {
    pub tsc_hz: u64,
    pub tsc_invariant: bool,
    pub apic_timer_hz: u64,
    pub x2apic: bool,
}

impl fmt::Display for TimerInfo {
    /// Do logu: "TSC 2400 MHz (invariantni), APIC timer 1000000 kHz, x2APIC".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "TSC {} MHz{}, APIC timer {} kHz, {}",
            self.tsc_hz / 1_000_000,
            if self.tsc_invariant {
                " (invariantni)"
            } else {
                " (CPU nehlasi invariantni TSC)"
            },
            self.apic_timer_hz / 1000,
            if self.x2apic { "x2APIC" } else { "xAPIC" },
        )
    }
}

impl api::Time for X86_64 {
    type TimerInfo = TimerInfo;

    /// TSC muze mit kazde CPU posunuty, jadro ho po startu CPU srovna.
    const CYCLE_COUNTER_SYNCED: bool = false;

    /// HPET jako reference, kalibrace TSC (hodiny) a APIC timeru, zapnuti
    /// lokalniho APIC.
    unsafe fn init_platform(platform: &Platform) -> TimerInfo {
        let hpet_phys = platform.arch.hpet_phys.expect("platforma nema HPET");
        // SAFETY: adresa HPET z ACPI.
        let hpet = unsafe { hpet::Hpet::new(hpet_phys) };
        const WINDOW_NS: u64 = 20_000_000;
        let tsc_hz = hpet.measure_hz(WINDOW_NS, time::rdtsc);
        time::init(tsc_hz);

        // SAFETY: jednou na BSP, IDT je nahrana, preruseni zakazana.
        unsafe { apic::init_cpu(true, platform.arch.lapic_phys) };
        ioapic::init(&platform.arch.ioapics, &platform.arch.overrides);
        apic::calibrate_timer(|read| hpet.measure_hz(WINDOW_NS, read));

        TimerInfo {
            tsc_hz,
            tsc_invariant: time::is_invariant(),
            apic_timer_hz: apic::timer_hz(),
            x2apic: apic::is_x2apic(),
        }
    }

    #[inline]
    fn now_ns() -> u64 {
        time::now_ns()
    }

    #[inline]
    fn cycle_counter() -> u64 {
        time::rdtsc()
    }

    fn adjust_cycle_counter(delta: i64) -> bool {
        time::adjust(delta)
    }

    fn set_timer(ns: Option<u64>) {
        apic::set_oneshot(ns);
    }

    fn ns_from_counter_to_start(counter: u64) -> i128 {
        time::ns_from_tsc_to_start(counter)
    }

    fn hardware_clock(platform: &Platform) -> Option<abi::time::WallTime> {
        if platform.arch.rtc_absent {
            return None;
        }
        rtc::read_time(platform.arch.rtc_century)
    }

    const HARDWARE_CLOCK_PHASE_NS: u32 = rtc::WRITE_PHASE_NS;

    fn set_hardware_clock(platform: &Platform, t: abi::time::WallTime) -> bool {
        if platform.arch.rtc_absent {
            return false;
        }
        rtc::write_time(platform.arch.rtc_century, t)
    }
}

impl api::Ipi for X86_64 {
    fn send_reschedule(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_RESCHEDULE);
    }

    fn send_halt(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_HALT);
    }

    fn send_tlb_flush(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_TLB_FLUSH);
    }
}

impl api::Memory for X86_64 {
    type PageTable = paging::PageTable;

    fn supports_1g_pages() -> bool {
        paging::supports_1g_pages()
    }

    /// Vlakno jadra (`None`) bezi v tabulkach jadra: zadne CPU pak nedrzi
    /// aktivni tabulky procesu, ktery uz mozna zanikl.
    #[inline]
    unsafe fn switch_user_space(root: Option<u64>) {
        let root = root.unwrap_or_else(|| crate::mm::kernel_root().as_u64());
        if cpu::read_cr3() & 0x000F_FFFF_FFFF_F000 != root {
            // SAFETY: predano volajicim; jadro je ve vsech adresnich
            // prostorech namapovane stejne.
            unsafe { cpu::write_cr3(root) };
        }
    }

    fn flush_tlb_local() {
        // SAFETY: zapis stejne hodnoty do CR3 jen vyprazdni TLB.
        unsafe { cpu::write_cr3(cpu::read_cr3()) };
    }

    /// x86 nema broadcast TLB invalidaci (INVLPGB jen na novych AMD),
    /// jadro posle IPI.
    fn flush_tlb_broadcast() -> bool {
        false
    }

    // DMA na x86 je koherentni s cache CPU: neni co delat.
    fn dma_sync_for_device(_phys: u64, _len: u64) {}

    fn dma_sync_for_cpu(_phys: u64, _len: u64) {}
}

impl api::DeviceIrq for X86_64 {
    /// Nize jsou vyjimky a vektory jadra (`apic::VECTOR_*`), nahore spurious.
    const DEVICE_IRQ_FIRST: u32 = idt::DEVICE_VECTOR_FIRST as u32;
    const DEVICE_IRQ_LAST: u32 = idt::DEVICE_VECTOR_LAST as u32;

    /// Pevny rezim, hrana, cil = bootovaci CPU.
    /// TODO: rozkladani preruseni mezi CPU.
    fn msi_message(irq: u32, _device: Option<(u16, u8, u8)>) -> Result<(u64, u32), Error> {
        Ok(msi_message(irq as u8))
    }

    /// Vektor nema v radici zadny stav: nic.
    fn release(_irq: u32) {}
}

impl api::LegacyIo for X86_64 {
    fn port_read(port: u16, width: u8) -> Result<u32, Error> {
        // SAFETY: opravneni k portu overil volajici (Resource); cteni
        // portu nesaha do pameti jadra.
        unsafe {
            Ok(match width {
                1 => cpu::inb(port) as u32,
                2 => cpu::inw(port) as u32,
                4 => cpu::inl(port),
                _ => return Err(Error::InvalidArgs),
            })
        }
    }

    fn port_write(port: u16, width: u8, value: u32) -> Result<(), Error> {
        // SAFETY: viz `port_read`.
        unsafe {
            match width {
                1 => cpu::outb(port, value as u8),
                2 => cpu::outw(port, value as u16),
                4 => cpu::outl(port, value),
                _ => return Err(Error::InvalidArgs),
            }
        }
        Ok(())
    }

    /// Cil = stejne CPU jako u MSI (bootovaci).
    fn line_route(line: u32, irq: u32, _edge: bool) -> Result<bool, Error> {
        let vector = u8::try_from(irq).map_err(|_| Error::InvalidArgs)?;
        ioapic::route(line, vector, crate::trap::irq_target_hw_id())
    }

    fn line_mask(line: u32, masked: bool) {
        ioapic::set_masked(line, masked)
    }

    fn line_release(line: u32) {
        ioapic::release(line)
    }
}

impl api::Power for X86_64 {
    /// Vypnuti potrebuje AML (S5 pres sluzbu ACPI). Restart: registr
    /// Reset Control (0xCF9, cipove sady Intel a QEMU q35), pak radic
    /// klavesnice (0x64 <- 0xFE). Kdyz ani jedno nezabere, `NotSupported`
    /// (sluzba ACPI pouzije registr reset z FADT, ktery zkousi napred).
    fn system_power(off: bool) -> Result<(), Error> {
        if off {
            return Err(Error::NotSupported);
        }
        // SAFETY: zapis do resetovacich registru desky; proces s pravem
        // POWER pocitac restartovat smi.
        unsafe {
            cpu::outb(0xCF9, 0x02);
            cpu::outb(0xCF9, 0x06);
        }
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }
        // SAFETY: dtto, pulz reset pres radic 8042.
        unsafe { cpu::outb(0x64, 0xFE) };
        for _ in 0..1_000_000 {
            core::hint::spin_loop();
        }
        Err(Error::NotSupported)
    }
}

/// Zprava MSI pro vektor `vector` (i pro preruseni chyb IOMMU).
fn msi_message(vector: u8) -> (u64, u32) {
    let dest = crate::trap::irq_target_hw_id();
    (0xFEE0_0000 | ((dest as u64 & 0xFF) << 12), vector as u32)
}

impl api::Iommu for X86_64 {
    type Domain = iommu::Domain;

    unsafe fn init(platform: &Platform) {
        // SAFETY: predano volajicim.
        unsafe {
            iommu::init(
                &platform.arch.iommus,
                &platform.arch.amd_iommus,
                &platform.pci_ecam,
            )
        }
    }

    fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<iommu::Domain>, Error> {
        iommu::domain(segment, bus, devfn)
    }

    fn poll_faults() {
        iommu::on_fault_interrupt();
    }
}

impl api::Smp for X86_64 {
    type Starter = smp::Trampoline;

    unsafe fn prepare_cpu_start(boot_info: &BootInfo) -> Option<smp::Trampoline> {
        // SAFETY: predano volajicim.
        unsafe { smp::prepare(boot_info) }
    }
}

impl api::PlatformInfo for X86_64 {
    type Info = acpi::X86Platform;

    fn acpi_table(signature: &[u8; 4], table: &[u8], platform: &mut Platform) {
        acpi::table(signature, table, platform);
    }

    fn acpi_check(platform: &Platform) -> Result<(), Error> {
        acpi::check(platform)
    }
}

impl api::Diagnostics for X86_64 {
    type Serial = SerialPort;

    /// x86 pouziva COM1 na pevnem portu, od loaderu nic nepotrebuje.
    fn init_early(_boot_info: &BootInfo) {}

    /// Port I/O na COM1 je neskodny i kdyz tam zadna linka neni (zapisy se
    /// zahodi), proto je bezpecne ho vytvaret opakovane, napr. v panic
    /// handleru.
    fn debug_serial() -> SerialPort {
        // SAFETY: COM1 v kernelu nepouziva nic jineho nez ladici vystup.
        unsafe { SerialPort::new(COM1) }
    }

    fn stack_trace(fp: u64, f: impl FnMut(u64)) {
        stack_trace(fp, f);
    }

    #[inline(always)]
    fn current_stack_trace(f: impl FnMut(u64)) {
        stack_trace(cpu::read_rbp(), f);
    }

    /// Ukonci QEMU pres zarizeni `isa-debug-exit`. Bez nej zapis nic
    /// neudela a funkce se vrati.
    fn qemu_exit(success: bool) {
        // Hodnota v se v QEMU prevede na navratovy kod (v << 1) | 1,
        // tj. 0x10 -> 33 (uspech), 0x11 -> 35 (selhani). Musi odpovidat xtask.
        let value = if success { 0x10 } else { 0x11 };
        // SAFETY: port 0xF4 pouzivame vyhradne pro isa-debug-exit.
        unsafe { cpu::outb(0xF4, value) };
    }

    const TESTS: &'static [(&'static str, fn())] = tests::TESTS;
}

/// Projde retez ramcu (kernel se preklada s frame pointery) od `rbp`
/// a pro kazdou navratovou adresu zavola `f`. Cte jen namapovanou pamet
/// jadra, aby pri poskozenem zasobniku nevznikla dalsi vyjimka.
fn stack_trace(mut rbp: u64, mut f: impl FnMut(u64)) {
    const MAX_DEPTH: usize = 32;
    for _ in 0..MAX_DEPTH {
        if rbp == 0
            || rbp % 8 != 0
            || rbp < 0xFFFF_8000_0000_0000
            || !paging::is_mapped(rbp)
            || !paging::is_mapped(rbp + 8)
        {
            return;
        }
        // SAFETY: obe slova ramce jsou zarovnana a namapovana (overeno vyse).
        let (next, ret) = unsafe { (*(rbp as *const u64), *((rbp + 8) as *const u64)) };
        if ret == 0 {
            return;
        }
        f(ret);
        rbp = next;
    }
}

/// Zaloguje, jak se uklada stav FPU vlaken (format, XCR0, velikost).
fn log_fpu() {
    let x = context::xcr0();
    if x == 0 {
        crate::log::info!("FPU: FXSAVE (CPU bez XSAVE), {} B na vlakno", context::fp_len());
    } else {
        crate::log::info!("FPU: XSAVE, XCR0 {x:#x}, {} B na vlakno", context::fp_len());
    }
}
