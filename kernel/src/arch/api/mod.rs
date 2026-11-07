//! Smlouva mezi jadrem a architekturou.
//!
//! Kazda architektura (`arch/x86_64`, `arch/aarch64`, ...) implementuje
//! vsechny traity z tohoto modulu na jednom prazdnem typu (`X86_64`,
//! `Aarch64`) a souhrnny trait [`Architecture`]. Prekladac tak sam hlida,
//! ze nova architektura ma vsechno a se stejnymi signaturami. Zbytek jadra
//! traity primo nevola: pouziva volne funkce a typove aliasy z `crate::arch`
//! (fasada), ktere volani preposilaji na aktualni architekturu. Vsechno je
//! staticky dispatch bez rezie.
//!
//! Smer zpet (architektura -> jadro: preruseni, syscall, vyjimka
//! v userspace) jde vyhradne pres `crate::trap`. Mimo nej smi kod
//! architektury pouzivat jen obecne sluzby jadra: `log`, `sync`, `error`,
//! `fatal`, `mm` (alokator fyzicke pameti, prima mapa, mapovani MMIO)
//! a cteni ACPI (`crate::acpi::read`).
//!
//! Co architektura dodava (skupiny = traity):
//!
//! | trait             | obsah                                                   |
//! |-------------------|---------------------------------------------------------|
//! | [`Cpu`]           | inicializace CPU, HW ID, per-CPU ukazatel, zastaveni    |
//! | [`Interrupts`]    | povoleni/zakazani preruseni na aktualnim CPU            |
//! | [`Threads`]       | kontext vlakna, prepnuti, zasobnik jadra, vstup do userspace |
//! | [`Time`]          | hodiny, citac cyklu, jednorazovy casovac                |
//! | [`Ipi`]           | preruseni mezi procesory                                |
//! | [`Memory`]        | tabulky stranek, TLB, cache pro DMA                     |
//! | [`DeviceIrq`]     | cisla preruseni zarizeni a zprava MSI                   |
//! | [`Iommu`]         | domeny prekladu DMA                                     |
//! | [`Smp`]           | start dalsich CPU                                       |
//! | [`PlatformInfo`]  | architekturni cast popisu platformy (ACPI tabulky)      |
//! | [`Diagnostics`]   | ladici seriova linka, vypis zasobniku, ukonceni QEMU    |

pub mod paging;

use core::fmt;

use boot_info::BootInfo;

use crate::acpi::Platform;
use crate::error::Error;

pub use paging::PageTableOps;

/// Vsechny casti smlouvy dohromady. Implementace je prazdna; jen vynuti,
/// ze typ architektury implementuje vsechny skupiny.
pub trait Architecture:
    Cpu
    + Interrupts
    + Threads
    + Time
    + Ipi
    + Memory
    + DeviceIrq
    + Iommu
    + Smp
    + PlatformInfo
    + Diagnostics
{
}

/// Procesor: start, identita, per-CPU data, zastaveni.
pub trait Cpu {
    /// Nazev architektury pro log ("x86_64", "aarch64").
    const NAME: &'static str;
    /// Jak se jmenuje hardwarove ID CPU (pro log: "APIC ID", "MPIDR").
    const HW_ID_NAME: &'static str;
    /// Jak se jmenuje registr s adresou instrukce (pro log: "rip", "pc").
    /// Vypis fatalni vyjimky ho pise na zacatek radku ("rip 0x..."), podle
    /// toho `cargo xtask` adresu najde a prelozi na jmeno funkce.
    const PC_NAME: &'static str;

    /// Data pro prechody do a z userspace; jadro je drzi v `PerCpu` na
    /// offsetu [`Cpu::CPU_LOCAL_OFFSET`] (asembler k nim pristupuje primo).
    type CpuLocal: CpuLocalData;
    /// Offset [`Cpu::CpuLocal`] v `PerCpu` (hlida staticka kontrola).
    const CPU_LOCAL_OFFSET: usize;

    /// Zakladni inicializace bootovaciho CPU (tabulky preruseni, funkce
    /// strankovani, FPU, vstup syscallu).
    ///
    /// # Safety
    ///
    /// Volat jednou, co nejdrive po startu, se zakazanymi prerusenimi.
    unsafe fn init_boot_cpu();

    /// Inicializace dalsiho CPU (vola se z jeho vstupniho bodu).
    ///
    /// # Safety
    ///
    /// Volat jednou na kazdem dalsim CPU, hned po startu, se zakazanymi
    /// prerusenimi.
    unsafe fn init_ap_cpu();

    /// Hardwarove ID aktualniho CPU (x86: ID lokalniho APIC, aarch64:
    /// afinita z MPIDR_EL1). Stejne cislo pouziva [`Ipi`] a [`Smp`].
    fn hw_id() -> u32;

    /// Nastavi per-CPU ukazatel na prazdna data (prvni slovo = 0, tj.
    /// "CPU jeste neni zaregistrovane"). Volat jako uplne prvni vec pri
    /// startu kazdeho CPU.
    fn clear_cpu_local();

    /// Nastavi per-CPU ukazatel jadra (x86: GS baze, aarch64: TPIDR_EL1).
    ///
    /// # Safety
    ///
    /// `ptr` musi ukazovat na strukturu, jejiz prvni slovo obsahuje `ptr`
    /// samotny, a musi zustat platna navzdy.
    unsafe fn set_cpu_local(ptr: u64);

    /// Ukazatel nastaveny `set_cpu_local` (rychle, bez zamku).
    fn cpu_local() -> u64;

    /// Zakaze preruseni a navzdy zastavi aktualni CPU.
    fn halt_forever() -> !;

    /// Vyvola ladici vyjimku (breakpoint), ktera se vrati zpet (test).
    fn breakpoint();
}

/// Per-CPU data architektury.
pub trait CpuLocalData: fmt::Debug + Send + Sync {
    /// Data pro aktualni CPU (volat po `init_boot_cpu`/`init_ap_cpu`).
    fn for_this_cpu() -> Self;
}

/// Preruseni na aktualnim CPU.
pub trait Interrupts {
    fn are_enabled() -> bool;
    /// Zakaze preruseni a vrati, zda byla predtim povolena.
    fn disable() -> bool;
    /// Obnovi stav vraceny `disable`.
    fn restore(was_enabled: bool);
    fn enable();
    /// Atomicky povoli preruseni a uspi CPU do dalsiho preruseni
    /// (preruseni mezi povolenim a uspanim nesmi utect).
    fn enable_and_halt();
}

/// Vlakna: kontext, prepinani, prechod do userspace.
pub trait Threads: Cpu {
    /// Ulozeny stav vlakna jadra (registry zachovavane volanym, FPU/SIMD).
    ///
    /// Stav FPU/SIMD programu se uklada jen v `switch`, ne pri kazdem vstupu
    /// do jadra. Jadro proto nesmi registry FPU/SIMD samo pouzivat: preklada
    /// se bez nich (x86_64: soft-float bez SSE, aarch64: cil `-softfloat`).
    type Context: ThreadContext;

    /// Ulozi aktualni kontext do `from` a prepne na `to`.
    ///
    /// # Safety
    ///
    /// `to` musi byt kontext vytvoreny `ThreadContext::new` nebo ulozeny
    /// touto funkci, jehoz vlakno prave nebezi na zadnem CPU. Volat se
    /// zakazanymi prerusenimi.
    unsafe fn switch(from: *mut Self::Context, to: *const Self::Context);

    /// Prepne na novy zasobnik a zavola `f(arg)`. Puvodni zasobnik se opusti.
    ///
    /// # Safety
    ///
    /// `top` musi byt vrchol namapovaneho, zapisovatelneho zasobniku
    /// zarovnany na 16 bajtu, ktery nic jineho nepouziva.
    unsafe fn switch_stack(top: u64, f: extern "C" fn(u64) -> !, arg: u64) -> !;

    /// Nastavi zasobnik jadra pro vstup z userspace na tomto CPU.
    ///
    /// # Safety
    ///
    /// Volat na vlastnim CPU se zakazanymi prerusenimi, `local` jsou data
    /// tohoto CPU.
    unsafe fn set_kernel_stack(local: &Self::CpuLocal, top: u64);

    /// Prvni vstup aktualniho vlakna do userspace: `entry(arg0, arg1)` se
    /// zasobnikem `stack`, preruseni povolena.
    ///
    /// # Safety
    ///
    /// Aktivni musi byt adresni prostor procesu a nastaveny zasobnik jadra
    /// (`set_kernel_stack`); `entry` a `stack` musi lezet v userspace.
    unsafe fn enter_user(entry: u64, stack: u64, arg0: u64, arg1: u64) -> !;
}

/// Kontext vlakna jadra.
pub trait ThreadContext: Default + fmt::Debug + Send {
    /// Kontext noveho vlakna: prvni `switch` na nej skonci skokem do `start`
    /// se zarovnanym zasobnikem a vychozim stavem FPU.
    ///
    /// # Safety
    ///
    /// `stack_top` musi byt vrchol namapovaneho zasobniku zarovnany na 16 B,
    /// ktery nic jineho nepouziva.
    unsafe fn new(stack_top: u64, start: extern "C" fn() -> !) -> Self;
}

/// Cas a casovac.
pub trait Time {
    /// Zjistene vlastnosti casovacu (jen pro log).
    type TimerInfo: fmt::Display;

    /// Citac cyklu je architekturou zaruceny stejny na vsech CPU (aarch64:
    /// CNTVCT_EL0). `false` = jadro ho po startu CPU zmeri a srovna (TSC).
    const CYCLE_COUNTER_SYNCED: bool;

    /// Inicializace casovacu na bootovacim CPU (kalibrace, zapnuti radice
    /// preruseni aktualniho CPU).
    ///
    /// # Safety
    ///
    /// Volat jednou, po `init_boot_cpu` a inicializaci pameti.
    unsafe fn init_platform(platform: &Platform) -> Self::TimerInfo;

    /// Monotonni cas od startu v nanosekundach (stejny na vsech CPU).
    fn now_ns() -> u64;
    /// Hodnota citace cyklu aktualniho CPU.
    fn cycle_counter() -> u64;
    /// Posune citac cyklu tohoto CPU; false, pokud to CPU neumi.
    fn adjust_cycle_counter(delta: i64) -> bool;
    /// Naplanuje preruseni casovace na tomto CPU za `ns`; `None` ho zastavi.
    /// Kdyz nastane, zavola se `trap::timer_interrupt`.
    fn set_timer(ns: Option<u64>);
    /// Nanosekundy od hodnoty citace cyklu `counter` (zmerene loaderem pri
    /// cteni casu firmwaru) do startu monotonnich hodin; zaporne = az po
    /// startu. Volat po `init_platform`.
    fn ns_from_counter_to_start(counter: u64) -> i128;
    /// Realny cas z hardwarovych hodin platformy (x86: CMOS RTC), kdyz cas
    /// nedal firmware. `None` = hodiny nejsou nebo vraci nesmysl. Volat po
    /// `init_platform`, jen pri startu.
    fn hardware_clock(platform: &Platform) -> Option<abi::time::WallTime>;
    /// Zapise cas do hardwarovych hodin platformy (`CLOCK_SET` s
    /// `WRITE_HARDWARE`). `t` uz je v case, ktery maji hodiny drzet (UTC,
    /// nebo mistni cas, kdyz ho tak chce sluzba casu). Volat za behu z
    /// libovolneho CPU; implementace si pristup k hodinam serializuje sama.
    /// `false` = hodiny nejsou, nebo se cas nevejde do jejich rozsahu.
    fn set_hardware_clock(platform: &Platform, t: abi::time::WallTime) -> bool;
}

/// Preruseni mezi procesory. Cil je hardwarove ID ([`Cpu::hw_id`]).
/// Na cilovem CPU se zavola odpovidajici funkce z `crate::trap`.
pub trait Ipi {
    /// Pozadavek na preplanovani (`trap::reschedule_ipi`).
    fn send_reschedule(cpu_hw_id: u32);
    /// Zastaveni CPU pri panice (cil zavola `Cpu::halt_forever`).
    fn send_halt(cpu_hw_id: u32);
    /// Pozadavek na vyprazdneni TLB (`trap::tlb_flush_ipi`).
    fn send_tlb_flush(cpu_hw_id: u32);
}

/// Pamet: tabulky stranek, TLB, cache.
pub trait Memory {
    type PageTable: PageTableOps;

    /// Umi CPU 1 GiB stranky?
    fn supports_1g_pages() -> bool;
    /// Prepne spodni polovinu adresniho prostoru (userspace), pokud uz neni
    /// aktivni: `Some(root)` = tabulky procesu (`PageTableOps::root`),
    /// `None` = zadny proces (vlakno jadra; spodni polovina nic nemapuje).
    /// x86_64 pro `None` nahraje tabulky jadra (CR3), aarch64 vypne
    /// prekladani pres TTBR0 (TCR_EL1.EPD0) - jadro samo v TTBR1 se nemeni.
    ///
    /// # Safety
    ///
    /// `root` musi byt platna korenova tabulka procesu se sdilenou polovinou
    /// jadra, ktera zustane platna, dokud je aktivni.
    unsafe fn switch_user_space(root: Option<u64>);
    /// Zahodi TLB zaznamy userspace na tomto CPU (globalni stranky jadra
    /// zustanou).
    fn flush_tlb_local();
    /// Zahodi TLB zaznamy userspace na vsech CPU jedinou instrukci, kterou
    /// vidi vsechna CPU (aarch64: `TLBI ... IS`), a pocka na dokonceni.
    /// `false` = architektura to neumi, jadro posle IPI ([`Ipi`]).
    fn flush_tlb_broadcast() -> bool;

    // Cache a DMA. Na koherentni platforme (x86_64, QEMU virt) DMA vidi
    // obsah cache CPU a obe funkce nic nedelaji. Na nekoherentni (napr.
    // Raspberry Pi 4, zjisti se pri startu z ACPI _CCA / DT `dma-coherent`)
    // se pamet pro DMA mapuje do procesu bez cache (`MapFlags::DMA`)
    // a jadro udrzuje cache tam, kde k ni samo pristupuje pres primou mapu
    // (vynulovani pri vytvoreni, VMO_READ/VMO_WRITE) - viz `object::vmo`.

    /// Zapise obsah cache do pameti a zneplatni ho, aby zarizeni (a mapovani
    /// bez cache) videlo, co CPU zapsalo pres primou mapu.
    fn dma_sync_for_device(phys: u64, len: u64);
    /// Zahodi obsah cache, aby CPU pres primou mapu videlo data zapsana
    /// zarizenim (nebo pres mapovani bez cache).
    fn dma_sync_for_cpu(phys: u64, len: u64);
}

/// Preruseni zarizeni (MSI/MSI-X). Cisla preruseni jsou cisla
/// architektury (x86: vektor IDT, aarch64: INTID LPI v GICv3).
pub trait DeviceIrq {
    /// Prvni cislo preruseni pro zarizeni.
    const DEVICE_IRQ_FIRST: u32;
    /// Posledni cislo preruseni pro zarizeni (vcetne).
    const DEVICE_IRQ_LAST: u32;

    /// Zprava MSI (adresa, data), kterou zarizeni zapise pro preruseni
    /// `irq`. `device` = adresa zarizeni na PCI (segment, sbernice,
    /// zarizeni << 3 | funkce), pokud je znama: GICv3 ITS ji potrebuje
    /// k prirazeni DeviceID, x86 ji ignoruje. Chyba (napr. `InvalidArgs`
    /// bez zarizeni na GICv3, `NoResources` pri plne tabulce ITS) se vrati
    /// programu ze syscallu `INTERRUPT_CREATE`.
    fn msi_message(irq: u32, device: Option<(u16, u8, u8)>) -> Result<(u64, u32), Error>;
}

/// IOMMU: preklad DMA zarizeni (x86: VT-d, AMD-Vi; aarch64: SMMUv3).
pub trait Iommu {
    type Domain: IommuDomain;

    /// Zapne vsechny IOMMU, ktere platforma ma. Od ted je DMA zarizeni bez
    /// domeny zakazane.
    ///
    /// # Safety
    ///
    /// Volat jednou pri startu, pred spustenim ovladacu.
    unsafe fn init(platform: &Platform);

    /// Domena zarizeni (segment, sbernice, zarizeni << 3 | funkce):
    /// existujici, nebo nova. `Ok(None)` = zadna IOMMU zarizeni nepokryva
    /// (DMA neomezene).
    fn domain(segment: u16, bus: u8, devfn: u8) -> Result<Option<Self::Domain>, Error>;
}

/// Domena prekladu DMA jednoho zarizeni (levne klonovatelny odkaz).
pub trait IommuDomain: Clone + Send + Sync {
    /// Povoli zarizeni cteni i zapis `pages` stranek od `phys` (identicky:
    /// adresa pro zarizeni = fyzicka adresa).
    fn map(&self, phys: u64, pages: u64) -> Result<(), Error>;
    /// Odebere zarizeni pristup ke strankam (pred vracenim pameti).
    fn unmap(&self, phys: u64, pages: u64);
}

/// Start dalsich CPU.
pub trait Smp {
    type Starter: CpuStarter;

    /// Pripravi start dalsich CPU (x86: trampolina pod 1 MiB, aarch64:
    /// PSCI). `None` = nejde, jadro pobezi na jednom CPU (duvod zaloguje
    /// architektura).
    ///
    /// # Safety
    ///
    /// Volat jednou, na bootovacim CPU, po inicializaci pameti.
    unsafe fn prepare_cpu_start(boot_info: &BootInfo) -> Option<Self::Starter>;
}

/// Pripraveny mechanismus startu CPU.
pub trait CpuStarter {
    /// Nastartuje CPU `hw_id`: skoci do `entry(arg)` na zasobniku
    /// `stack_top`. `started` vraci true, jakmile CPU ohlasi start. Vraci,
    /// zda CPU vcas nabehlo.
    fn start(
        &self,
        hw_id: u32,
        stack_top: u64,
        entry: extern "C" fn(u64) -> !,
        arg: u64,
        started: impl Fn() -> bool,
    ) -> bool;

    /// Uklid po startu vsech CPU (nevola se, pokud nektere neodpovedelo -
    /// to muze nabehnout pozdeji a pomocna data jeste potrebovat).
    fn finish(self);
}

/// Architekturni cast popisu platformy. Obecne `crate::acpi` cte RSDP,
/// XSDT a MCFG; ostatni tabulky (MADT, HPET, DMAR, IVRS, GTDT, IORT, ...)
/// preda architekture.
pub trait PlatformInfo {
    /// Architekturni udaje (x86: LAPIC, IOAPIC, HPET, IOMMU).
    /// `Display` = souhrn do logu.
    type Info: Default + fmt::Debug + fmt::Display;

    /// Zpracuje jednu ACPI tabulku (overenou: delka, checksum). Seznam CPU
    /// plni do `platform.cpus`.
    fn acpi_table(signature: &[u8; 4], table: &[u8], platform: &mut Platform);

    /// Kontrola po precteni vsech tabulek (napr. chybi seznam CPU).
    fn acpi_check(platform: &Platform) -> Result<(), Error>;
}

/// Ladeni a testy.
pub trait Diagnostics {
    /// Ladici seriova linka (x86: COM1, aarch64: PL011).
    type Serial: DebugSerial;

    /// Vrati ladici seriovou linku. Musi byt bezpecne volat opakovane,
    /// i z panic handleru.
    fn debug_serial() -> Self::Serial;

    /// Projde retez ramcu od ukazatele ramce `fp` (kernel se preklada
    /// s frame pointery) a pro kazdou navratovou adresu zavola `f`. Cte jen
    /// namapovanou pamet jadra.
    fn stack_trace(fp: u64, f: impl FnMut(u64));

    /// Vypis zasobniku od mista volani.
    fn current_stack_trace(f: impl FnMut(u64));

    /// Ukonci QEMU s vysledkem testu; bez testovaciho zarizeni se vrati.
    fn qemu_exit(success: bool);

    /// Self-testy specificke pro architekturu (spousti je `crate::tests`).
    const TESTS: &'static [(&'static str, fn())];
}

/// Ladici seriova linka.
pub trait DebugSerial: fmt::Write + Send {
    /// Nastavi linku (rychlost, format). Volat jednou pri startu.
    fn init(&mut self);
}
