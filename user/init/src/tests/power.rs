//! Testy sluzby ACPI (`acpid`) a napajeni.
//!
//! Sada "acpi" bezi v beznem testovacim behu: acpid odpovida, hlasi, co
//! platforma umi, cte cidla, a jadro hlida prava (pamet ACPI ano, RAM ne;
//! `SYSTEM_POWER` jen s pravem `POWER`).
//!
//! Vypnuti samo testuje zvlastni beh QEMU (`cargo xtask test` ho spusti po
//! hlavnim behu): v initfs je `power-test` s rezimem a init misto testu
//! udela [`scenario`]:
//! - `button`: ceka na tlacitko napajeni (xtask posle QMP
//!   `system_powerdown`), pak cisto vypne system pres S5,
//! - `reboot`: rovnou restartuje (QEMU s `-no-reboot` skonci).
//!
//! xtask pozna uspech podle konce QEMU s kodem 0 a znacek v logu.

use core::time::Duration;

use abi::syscall::power_op;
use libos::{Error, Rights, Startup, println};
use powerproto::{Reply, Request, feature};

use super::{TestResult, fail};
use crate::{check, check_eq};

pub const TESTS: &[(&str, fn(&Startup) -> TestResult)] = &[
    ("acpid bezi a hlasi moznosti platformy", status),
    ("teploty, baterie a vykon CPU", sensors),
    ("pamet ACPI jde mapovat, RAM ne", acpi_memory),
    ("SYSTEM_POWER jen s pravem POWER", power_right),
    ("linky preruseni v prostredku", lines),
];

/// Jak dlouho smi trvat inicializace ACPICA (TCG na aarch64 je pomale).
const READY: Duration = Duration::from_secs(60);

fn status(_s: &Startup) -> TestResult {
    let st = crate::power::wait_ready(READY).map_err(|e| fail("Status", e))?;
    println!("init: acpid: {st:?}");
    check!(st.features & feature::SHUTDOWN != 0, "platforma nehlasi vypnuti");
    check!(st.features & feature::REBOOT != 0, "platforma nehlasi restart");
    check!(st.features & feature::POWER_BUTTON != 0, "platforma nehlasi tlacitko napajeni");
    let hw_reduced = st.features & feature::HW_REDUCED != 0;
    check_eq!(hw_reduced, cfg!(target_arch = "aarch64"), "HW-reduced (ARM ano, x86 ne)");
    check!(st.cpus >= 1, "zadny procesor v jmennem prostoru ACPI");
    Ok(())
}

fn sensors(_s: &Startup) -> TestResult {
    let t = Duration::from_secs(10);
    match crate::power::request(&Request::Thermal, t).map_err(|e| fail("Thermal", e))? {
        // QEMU termalni zony nema; staci, ze dotaz projde.
        Reply::Thermal(z) => println!("init: termalni zony: {}", z.len()),
        r => return Err(alloc::format!("Thermal: necekana odpoved {r:?}")),
    }
    match crate::power::request(&Request::Battery, t).map_err(|e| fail("Battery", e))? {
        Reply::Battery(b) => println!("init: baterie: {}", b.len()),
        r => return Err(alloc::format!("Battery: necekana odpoved {r:?}")),
    }
    let st = crate::power::wait_ready(t).map_err(|e| fail("Status", e))?;
    match crate::power::request(&Request::CpuPerf, t).map_err(|e| fail("CpuPerf", e))? {
        Reply::CpuPerf(c) => check_eq!(c.len() as u32, st.cpus, "pocet CPU v CpuPerf"),
        r => return Err(alloc::format!("CpuPerf: necekana odpoved {r:?}")),
    }
    Ok(())
}

fn acpi_memory(s: &Startup) -> TestResult {
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    let vmo = s.acpi.as_ref().ok_or("chybi udaje ACPI od jadra")?;
    let mut raw = alloc::vec![0u8; 56 + 16 * abi::acpi::MAX_RANGES];
    vmo.read(0, &mut raw).map_err(|e| fail("cteni udaju ACPI", e))?;
    let info = abi::acpi::Info::decode(&raw).ok_or("neplatne udaje ACPI")?;
    check!(&info.rsdp[..8] == b"RSD PTR ", "kopie RSDP nema podpis");
    let &(start, len) = info.ranges.as_slice().first().ok_or("zadna oblast pameti ACPI")?;
    check!(len >= 4096, "oblast ACPI mensi nez stranka");
    root.device_memory(start, 4096).map_err(|e| fail("mapovani pameti ACPI", e))?;
    // Bezna RAM (pamet pro DMA s fyzickou adresou) mapovat nejde.
    // Pamet pro DMA jen s identitou zarizeni (00:00.0).
    let dev = root.restrict_device(&[], 0, 4096, (0, 0, 0, 0)).map_err(|e| fail("prostredek zarizeni", e))?;
    let dma = dev.dma_memory(4096).map_err(|e| fail("DMA pamet", e))?;
    let phys = dev.phys_addr(&dma, 0).map_err(|e| fail("fyzicka adresa", e))?;
    check_eq!(root.device_memory(phys, 4096).err(), Some(Error::AccessDenied), "RAM pres VMO_CREATE_PHYSICAL");
    // Rozsah pres hranici oblasti ACPI (cast RAM) take ne.
    check_eq!(
        root.device_memory(start, len + (1 << 30)).err(),
        Some(Error::AccessDenied),
        "rozsah presahujici pamet ACPI"
    );
    Ok(())
}

fn power_right(s: &Startup) -> TestResult {
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    let without = root
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::MANAGE))
        .map_err(|e| fail("duplikat bez POWER", e))?;
    check_eq!(
        libos::sys::system_power(without.raw(), power_op::OFF).err(),
        Some(Error::AccessDenied),
        "vypnuti bez prava POWER"
    );
    // Neplatna operace se odmitne drive, nez by se cokoli vypnulo.
    check_eq!(
        libos::sys::system_power(root.handle().raw(), 7).err(),
        Some(Error::InvalidArgs),
        "neplatna operace"
    );
    // Omezeny prostredek pravo POWER nikdy nema.
    let child = root.restrict(&[(0, 1 << 30)], 0, 0).map_err(|e| fail("omezeny prostredek", e))?;
    check_eq!(
        libos::sys::system_power(child.handle().raw(), power_op::OFF).err(),
        Some(Error::AccessDenied),
        "vypnuti s omezenym prostredkem"
    );
    Ok(())
}

fn lines(s: &Startup) -> TestResult {
    use libos::hw::irq_lines;
    let root = s.root_resource.as_ref().ok_or("chybi korenovy Resource")?;
    root.restrict(&[irq_lines(32, 988)], 1, 0).map_err(|e| fail("linky SPI 32-1019", e))?;
    check_eq!(
        root.restrict(&[irq_lines(1000, 25)], 1, 0).err(),
        Some(Error::InvalidArgs),
        "linky za IRQ_LINES_MAX"
    );
    let child = root.restrict(&[irq_lines(40, 2)], 1, 0).map_err(|e| fail("dve linky", e))?;
    check_eq!(child.restrict(&[irq_lines(39, 2)], 1, 0).err(), Some(Error::AccessDenied), "potomek s linkou navic");
    check_eq!(child.interrupt_line(44).err(), Some(Error::AccessDenied), "linka mimo prostredek");
    check_eq!(
        libos::sys::interrupt_create_line_flags(root.handle().raw(), 9, 1 << 7).err(),
        Some(Error::InvalidArgs),
        "neznamy priznak linky"
    );
    Ok(())
}

/// Rezim zvlastniho behu napajeni z initfs (`power-test`), nebo `None`.
pub fn mode(startup: &Startup) -> Option<alloc::string::String> {
    let fs = libos::initfs::Initfs::open(startup).ok()?;
    let d = fs.find("power-test")?;
    Some(alloc::string::String::from_utf8_lossy(d).trim().into())
}

/// Scenar zvlastniho behu (viz modul). Vraci jen pri chybe (kod initu).
pub fn scenario(startup: &Startup, mode: &str) -> i64 {
    let st = match crate::power::wait_ready(READY) {
        Ok(s) => s,
        Err(e) => {
            println!("init: test napajeni: acpid neodpovida: {e}");
            return 1;
        }
    };
    println!("init: test napajeni ({mode}): acpid pripraveno, funkce {:#x}", st.features);
    let mut sup = crate::supervisor::SUPERVISOR.lock();
    match mode {
        "reboot" => {
            crate::power::shutdown(&mut sup, true);
        }
        _ => {
            // Znacka pro xtask: ted poslat QMP system_powerdown.
            println!("init: test napajeni: cekam na tlacitko napajeni");
            let deadline = libos::time::deadline_after(Duration::from_secs(300));
            while libos::time::now() < deadline && !crate::power::ATTEMPTED.load(core::sync::atomic::Ordering::Acquire) {
                // Tlacitko zpracuje supervisor (`power::on_events` -> vypnuti).
                sup.step(startup, Some(libos::time::deadline_after(Duration::from_secs(1))));
            }
            if !crate::power::ATTEMPTED.load(core::sync::atomic::Ordering::Acquire) {
                println!("init: test napajeni: tlacitko neprislo do 300 s");
            }
        }
    }
    1
}
