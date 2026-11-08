//! Cidla a moznosti z jmenneho prostoru ACPI: termalni zony (`_TZ`),
//! baterie (`PNP0C0A`), stavy vykonu procesoru (`_PSS`, `_CPC`).
//!
//! Rizeni frekvence CPU zatim neni: [`cpu_perf`] jen zjisti, co firmware
//! nabizi (podklad pro budouci ovladac P-stavu / CPPC).

use alloc::vec::Vec;

use acpica::ffi::{self, Handle};
use acpica::ns::{self, Value};
use powerproto::{Battery, CpuPerf, ThermalZone, battery_state};

/// Hodnota "nezname" v balicich `_BST`/`_BIF`.
const UNKNOWN: u64 = 0xFFFF_FFFF;

fn known(v: Option<&Value>) -> Option<u32> {
    let v = v?.as_integer()?;
    (v != UNKNOWN && v <= u32::MAX as u64).then_some(v as u32)
}

pub fn thermal_count() -> u32 {
    ns::objects(ffi::ACPI_TYPE_THERMAL).len() as u32
}

pub fn thermal() -> Vec<ThermalZone> {
    ns::objects(ffi::ACPI_TYPE_THERMAL)
        .into_iter()
        .map(|z| {
            let int = |n: &core::ffi::CStr| ns::eval_integer(z, n).filter(|&v| v <= u32::MAX as u64).map(|v| v as u32);
            ThermalZone { name: ns::path(z), temp_dk: int(c"_TMP"), critical_dk: int(c"_CRT"), passive_dk: int(c"_PSV") }
        })
        .collect()
}

pub fn battery_count() -> u32 {
    ns::devices(c"PNP0C0A").len() as u32
}

fn battery(dev: Handle) -> Battery {
    let mut b = Battery { name: ns::path(dev), ..Default::default() };
    // _STA bit 4: baterie je vlozena.
    b.present = ns::eval_integer(dev, c"_STA").is_none_or(|s| s & 0x10 != 0);
    if !b.present {
        return b;
    }
    // _BIX (ACPI 4.0+): [1] jednotka, [2] navrhova, [3] plna kapacita;
    // _BIF: [0] jednotka, [1] navrhova, [2] plna.
    if let Ok(Value::Package(p)) = ns::eval(dev, c"_BIX", &[]) {
        b.unit_mw = p.get(1).and_then(Value::as_integer) == Some(0);
        b.design = known(p.get(2));
        b.full = known(p.get(3));
    } else if let Ok(Value::Package(p)) = ns::eval(dev, c"_BIF", &[]) {
        b.unit_mw = p.first().and_then(Value::as_integer) == Some(0);
        b.design = known(p.get(1));
        b.full = known(p.get(2));
    }
    // _BST: [0] stav, [1] vykon/proud, [2] zbyva, [3] napeti.
    if let Ok(Value::Package(p)) = ns::eval(dev, c"_BST", &[]) {
        let s = p.first().and_then(Value::as_integer).unwrap_or(0) as u32;
        b.state = s & (battery_state::DISCHARGING | battery_state::CHARGING | battery_state::CRITICAL);
        b.rate = known(p.get(1));
        b.remaining = known(p.get(2));
    }
    b
}

pub fn batteries() -> Vec<Battery> {
    ns::devices(c"PNP0C0A").into_iter().map(battery).collect()
}

/// Procesory: stare objekty `Processor` i zarizeni `ACPI0007`.
fn cpus() -> Vec<Handle> {
    let mut v = ns::objects(ffi::ACPI_TYPE_PROCESSOR);
    for d in ns::devices(c"ACPI0007") {
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

pub fn cpu_count() -> u32 {
    cpus().len() as u32
}

pub fn cpu_perf() -> Vec<CpuPerf> {
    cpus()
        .into_iter()
        .map(|c| {
            let (states, max) = match ns::eval(c, c"_PSS", &[]) {
                // _PSS: balicek stavu, kazdy [0] = frekvence v MHz.
                Ok(Value::Package(p)) => {
                    let max = p.iter().filter_map(|s| s.as_package()?.first()?.as_integer()).max().unwrap_or(0);
                    (p.len() as u32, max.min(u32::MAX as u64) as u32)
                }
                _ => (0, 0),
            };
            CpuPerf { name: ns::path(c), pss_states: states, max_mhz: max, cppc: ns::exists(c, c"_CPC") }
        })
        .collect()
}
