//! acpid: sluzba ACPI (ACPICA v userspace).
//!
//! Interpretuje AML firmwaru (DSDT, SSDT) pres ACPICA a nabizi initu
//! napajeni: vypnuti (S5), restart, udalosti tlacitek, teploty termalnich
//! zon, stav baterii a co firmware umi pro rizeni frekvence CPU. Protokol:
//! `libs/powerproto` po bootstrap kanalu.
//!
//! Start (handly `kind::USER` od initu, v tomto poradi):
//! 1. Resource s rozsahy pro ACPI (MANAGE): pamet mimo RAM a pamet firmwaru
//!    ACPI, I/O porty (x86), pevne linky preruseni,
//! 2. Resource jen s pravem `POWER` (vypnuti/restart pres firmware), nebo
//!    chybi,
//! 3. objekt pameti s udaji o ACPI od jadra (`abi::acpi::Info`).
//!
//! Proc v userspace: AML je kod firmwaru, ktery bezi s plnym pristupem
//! k hardwaru, co si sam nadeklaruje. Chyba v nem (nebo v interpretu)
//! v jadre = pad systemu; tady jen pad sluzby. Proti RAM jadra a procesu
//! chrani jadro (fyzicke mapovani jen registru a pameti ACPI). Viz
//! `docs/cs/acpi.md`, kapitola "Bezpecnost".
//!
//! Konec bootstrap kanalu u initu = konec sluzby.

#![no_std]
#![no_main]

extern crate alloc;

mod events;
mod osl;
mod power;
mod sensors;

use alloc::vec::Vec;

use acpica::ffi;
use libos::hw::Resource;
use libos::{Signals, Startup, Vmo, println};
use powerproto::{Reply, Request};

libos::entry!(main);

fn log_line(b: &[u8]) {
    let s = core::str::from_utf8(b).unwrap_or("?");
    println!("acpid: {}", s.trim_end());
}

fn check(what: &str, st: ffi::Status) -> Result<(), ffi::Status> {
    if st == ffi::AE_OK {
        Ok(())
    } else {
        println!("acpid: {what} selhalo: {}", acpica::status_name(st));
        Err(st)
    }
}

/// Inicializace ACPICA v poradi podle referencni prirucky (a Linuxu).
fn start_acpica() -> Result<(), ffi::Status> {
    // SAFETY: volani ACPICA z jednoho vlakna pri startu; vrstva OS je pripravena.
    unsafe {
        check("AcpiInitializeSubsystem", ffi::AcpiInitializeSubsystem())?;
        check("AcpiInitializeTables", ffi::AcpiInitializeTables(core::ptr::null_mut(), 16, 0))?;
    }
    power::read_mcfg();
    // SAFETY: dtto.
    unsafe {
        check("AcpiLoadTables", ffi::AcpiLoadTables())?;
        check("AcpiEnableSubsystem", ffi::AcpiEnableSubsystem(ffi::ACPI_FULL_INITIALIZATION))?;
        check("AcpiInitializeObjects", ffi::AcpiInitializeObjects(ffi::ACPI_FULL_INITIALIZATION))?;
        // GPE s metodami _Lxx/_Exx (napr. hotplug) zapnout az po _INI.
        let _ = ffi::AcpiUpdateAllGpes();
    }
    Ok(())
}

fn main(mut startup: Startup) -> i64 {
    cstd::set_output(log_line);
    cstd::init_stack_guard(libos::random::u64().0);
    if startup.handles.len() < 3 {
        println!("acpid: chybi handly od initu (Resource, Resource POWER, udaje ACPI)");
        return 1;
    }
    let info_vmo = Vmo::from_handle(startup.handles.remove(2));
    let power_res = Resource::from_handle(startup.handles.remove(1));
    let resource = Resource::from_handle(startup.handles.remove(0));
    let mut raw = alloc::vec![0u8; 56 + 16 * abi::acpi::MAX_RANGES];
    let info = match info_vmo.read(0, &mut raw).ok().and_then(|_| abi::acpi::Info::decode(&raw)) {
        Some(i) => i,
        None => {
            println!("acpid: neplatne udaje ACPI od jadra");
            return 1;
        }
    };
    drop(info_vmo);
    osl::init(osl::Env { resource, info });
    if let Err(e) = osl::sched::start_workers() {
        println!("acpid: vlakna pro odlozenou praci nelze spustit: {e}");
        return 1;
    }
    if start_acpica().is_err() {
        // Bez ACPI odpovida na pozadavky chybou (init muze vypnout jinak).
        println!("acpid: ACPI nefunguje, pozadavky budou odmitnuty");
        return serve(&startup, None, &power_res);
    }
    let platform = power::platform();
    events::install(&platform);
    let st = power::status(&platform);
    println!(
        "acpid: ACPI pripraveno (FADT rev {}, OEM {}, {}{}{}zony {}, baterie {}, CPU {})",
        st.fadt_revision,
        st.oem,
        if platform.hw_reduced != 0 { "HW-reduced, " } else { "" },
        if st.features & powerproto::feature::SHUTDOWN != 0 { "S5, " } else { "" },
        if st.features & powerproto::feature::POWER_BUTTON != 0 { "tlacitko, " } else { "" },
        st.thermal_zones,
        st.batteries,
        st.cpus
    );
    serve(&startup, Some(&platform), &power_res)
}

/// Smycka sluzby: pozadavky od initu a udalosti tlacitek.
fn serve(startup: &Startup, platform: Option<&ffi::Platform>, power_res: &Resource) -> i64 {
    let chan = &startup.bootstrap;
    let wake = events::wake_event();
    loop {
        let mut items: Vec<(&libos::Handle, Signals)> = alloc::vec![(chan.handle(), Signals::READABLE | Signals::PEER_CLOSED)];
        if let Some(w) = wake {
            items.push((w.handle(), Signals::SIGNALED));
        }
        let observed = match libos::wait_many(&items, None) {
            Ok((_, o)) => o,
            Err(e) => {
                println!("acpid: cekani selhalo: {e}");
                return 1;
            }
        };
        if observed.get(1).is_some_and(|s| s.intersects(Signals::SIGNALED)) {
            for e in events::take() {
                println!("acpid: udalost {e:?}, hlasim initu");
                let _ = chan.write(&Reply::Event(e).encode(), Vec::new());
            }
        }
        let sig = observed.first().copied().unwrap_or(Signals::NONE);
        if sig.intersects(Signals::READABLE) {
            while let Ok(m) = chan.try_read() {
                let reply = match (Request::decode(&m.data), platform) {
                    (Err(e), _) => Reply::Failed(e),
                    (Ok(_), None) => Reply::Failed(libos::Error::NotSupported),
                    (Ok(r), Some(p)) => handle(r, p, power_res),
                };
                let _ = chan.write(&reply.encode(), Vec::new());
            }
        } else if sig.intersects(Signals::PEER_CLOSED) {
            println!("acpid: init zavrel kanal, koncim");
            return 0;
        }
    }
}

fn handle(r: Request, p: &ffi::Platform, power_res: &Resource) -> Reply {
    match r {
        Request::Status => Reply::Status(power::status(p)),
        Request::Shutdown => Reply::Failed(power::shutdown(p, power_res)),
        Request::Reboot => Reply::Failed(power::reboot(p, power_res)),
        Request::Thermal => Reply::Thermal(sensors::thermal()),
        Request::Battery => Reply::Battery(sensors::batteries()),
        Request::CpuPerf => Reply::CpuPerf(sensors::cpu_perf()),
        // Balicek pro vyvojare resi init se sluzbou logu, ne acpid.
        Request::Report | Request::LogLevel { .. } | Request::Snapshot | Request::OpenInstaller | Request::Appearance { .. }
        | Request::Permissions { .. } => {
            Reply::Failed(libos::Error::NotSupported)
        }
    }
}
