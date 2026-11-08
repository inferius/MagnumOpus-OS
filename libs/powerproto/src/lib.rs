//! Protokol sluzby napajeni `acpid` (verze 1).
//!
//! Kanal init <-> acpid je bootstrap kanal acpid. Init posila [`Request`],
//! acpid odpovida [`Reply`] (v poradi pozadavku) a kdykoli sam posle
//! [`Reply::Event`] (tlacitko napajeni, ...). Shell o vypnuti zada init
//! (stejne [`Request`] po vlastnim kanalu), ne primo acpid: init nejdriv
//! cisto ukonci sluzby (zapis FS na disk), az pak preda pozadavek acpid.
//!
//! Zprava = kod operace `u32` + pole (`libs/wire`, little-endian). Neplatna
//! zprava = `INVALID_ARGS`, nikdy panika.
//!
//! Jednotky: teplota v desetinach kelvinu (jako ACPI `_TMP`; 2982 = 25,0 C),
//! kapacita a vykon baterie v jednotkach z `_BIF`/`_BIX` (mWh a mW, nebo
//! mAh a mA - [`Battery::unit_mw`]).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

#[cfg(test)]
mod tests;

use alloc::string::String;
use alloc::vec::Vec;

pub use abi::{Error, Result};
use wire::{Reader, Writer};

pub const VERSION: u32 = 1;

/// Nejvic termalnich zon, baterii a procesoru v jedne odpovedi.
pub const MAX_ITEMS: usize = 64;

/// Pozadavek na sluzbu napajeni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Co platforma umi ([`Reply::Status`]).
    Status,
    /// Vypnout (S5). Uspech = pocitac se vypne, odpoved neprijde; chyba =
    /// [`Reply::Failed`].
    Shutdown,
    /// Restartovat (registr reset z FADT, jinak firmware).
    Reboot,
    /// Teploty termalnich zon (`_TZ`).
    Thermal,
    /// Stav baterii (`PNP0C0A`).
    Battery,
    /// Rizeni frekvence CPU: co firmware nabizi (`_PSS`, `_CPC`). Zatim jen
    /// zjisteni, rizeni neni.
    CpuPerf,
}

/// Udalost hlasena sluzbou bez pozadavku.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Stisk tlacitka napajeni (pevna udalost nebo zarizeni `PNP0C0C`).
    PowerButton,
    /// Stisk tlacitka spanku (`PNP0C0E`).
    SleepButton,
}

/// Co platforma podporuje (bity [`Status::features`]).
pub mod feature {
    /// Vypnuti S5 (x86: registr spanku z FADT; aarch64: PSCI).
    pub const SHUTDOWN: u32 = 1 << 0;
    /// Restart.
    pub const REBOOT: u32 = 1 << 1;
    /// Tlacitko napajeni hlasi udalost.
    pub const POWER_BUTTON: u32 = 1 << 2;
    /// Platforma bez pevneho hardwaru ACPI (HW-reduced, typicky ARM).
    pub const HW_REDUCED: u32 = 1 << 3;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub features: u32,
    /// Revize FADT a OEM ID (pro log a diagnostiku).
    pub fadt_revision: u8,
    pub oem: String,
    pub thermal_zones: u32,
    pub batteries: u32,
    pub cpus: u32,
}

/// Jedna termalni zona.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThermalZone {
    /// Cesta v jmennem prostoru ACPI (napr. `\_TZ.TZ00`).
    pub name: String,
    /// Aktualni teplota (`_TMP`), desetiny K; `None` = nejde precist.
    pub temp_dk: Option<u32>,
    /// Kriticka teplota (`_CRT`), pri ktere se ma pocitac vypnout.
    pub critical_dk: Option<u32>,
    /// Pasivni chlazeni (`_PSV`, snizeni frekvence).
    pub passive_dk: Option<u32>,
}

/// Stav baterie (bity [`Battery::state`], jako `_BST`).
pub mod battery_state {
    pub const DISCHARGING: u32 = 1 << 0;
    pub const CHARGING: u32 = 1 << 1;
    pub const CRITICAL: u32 = 1 << 2;
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Battery {
    pub name: String,
    pub present: bool,
    /// Jednotky mWh/mW (`true`) nebo mAh/mA (`false`).
    pub unit_mw: bool,
    pub state: u32,
    /// Aktualni vykon/proud (`_BST`), `None` = nezname.
    pub rate: Option<u32>,
    pub remaining: Option<u32>,
    /// Plna kapacita a navrhova kapacita (`_BIF`/`_BIX`).
    pub full: Option<u32>,
    pub design: Option<u32>,
}

impl Battery {
    /// Nabiti v procentech (zbyva / plna kapacita).
    pub fn percent(&self) -> Option<u32> {
        let (r, f) = (self.remaining?, self.full?);
        (f > 0).then(|| ((r as u64 * 100) / f as u64).min(100) as u32)
    }
}

/// Moznosti rizeni vykonu jednoho CPU.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CpuPerf {
    pub name: String,
    /// Pocet stavu vykonu `_PSS` (0 = zadne).
    pub pss_states: u32,
    /// Nejvyssi frekvence z `_PSS` (MHz), 0 = neznama.
    pub max_mhz: u32,
    /// Firmware nabizi CPPC (`_CPC`).
    pub cppc: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Ok,
    Failed(Error),
    Status(Status),
    Thermal(Vec<ThermalZone>),
    Battery(Vec<Battery>),
    CpuPerf(Vec<CpuPerf>),
    Event(Event),
}

mod op {
    pub const STATUS: u32 = 1;
    pub const SHUTDOWN: u32 = 2;
    pub const REBOOT: u32 = 3;
    pub const THERMAL: u32 = 4;
    pub const BATTERY: u32 = 5;
    pub const CPU_PERF: u32 = 6;

    pub const R_OK: u32 = 100;
    pub const R_FAILED: u32 = 101;
    pub const R_STATUS: u32 = 102;
    pub const R_THERMAL: u32 = 103;
    pub const R_BATTERY: u32 = 104;
    pub const R_CPU_PERF: u32 = 105;
    pub const R_EVENT: u32 = 106;
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        Writer::op(match self {
            Request::Status => op::STATUS,
            Request::Shutdown => op::SHUTDOWN,
            Request::Reboot => op::REBOOT,
            Request::Thermal => op::THERMAL,
            Request::Battery => op::BATTERY,
            Request::CpuPerf => op::CPU_PERF,
        })
        .finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let req = match r.u32()? {
            op::STATUS => Request::Status,
            op::SHUTDOWN => Request::Shutdown,
            op::REBOOT => Request::Reboot,
            op::THERMAL => Request::Thermal,
            op::BATTERY => Request::Battery,
            op::CPU_PERF => Request::CpuPerf,
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(req)
    }
}

/// `Option<u32>` jako priznak + hodnota.
fn opt(w: &mut Writer, v: Option<u32>) {
    w.u8(v.is_some() as u8).u32(v.unwrap_or(0));
}

fn read_opt(r: &mut Reader) -> Result<Option<u32>> {
    let some = r.u8()?;
    let v = r.u32()?;
    match some {
        0 => Ok(None),
        1 => Ok(Some(v)),
        _ => Err(Error::InvalidArgs),
    }
}

fn read_count(r: &mut Reader) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > MAX_ITEMS {
        return Err(Error::InvalidArgs);
    }
    Ok(n)
}

impl Reply {
    pub fn encode(&self) -> Vec<u8> {
        let mut w;
        match self {
            Reply::Ok => w = Writer::op(op::R_OK),
            Reply::Failed(e) => {
                w = Writer::op(op::R_FAILED);
                w.i32(e.code() as i32);
            }
            Reply::Status(s) => {
                w = Writer::op(op::R_STATUS);
                w.u32(s.features).u8(s.fadt_revision).str(&s.oem).u32(s.thermal_zones).u32(s.batteries).u32(s.cpus);
            }
            Reply::Thermal(zones) => {
                w = Writer::op(op::R_THERMAL);
                w.u32(zones.len().min(MAX_ITEMS) as u32);
                for z in zones.iter().take(MAX_ITEMS) {
                    w.str(&z.name);
                    opt(&mut w, z.temp_dk);
                    opt(&mut w, z.critical_dk);
                    opt(&mut w, z.passive_dk);
                }
            }
            Reply::Battery(bats) => {
                w = Writer::op(op::R_BATTERY);
                w.u32(bats.len().min(MAX_ITEMS) as u32);
                for b in bats.iter().take(MAX_ITEMS) {
                    w.str(&b.name).u8(b.present as u8).u8(b.unit_mw as u8).u32(b.state);
                    opt(&mut w, b.rate);
                    opt(&mut w, b.remaining);
                    opt(&mut w, b.full);
                    opt(&mut w, b.design);
                }
            }
            Reply::CpuPerf(cpus) => {
                w = Writer::op(op::R_CPU_PERF);
                w.u32(cpus.len().min(MAX_ITEMS) as u32);
                for c in cpus.iter().take(MAX_ITEMS) {
                    w.str(&c.name).u32(c.pss_states).u32(c.max_mhz).u8(c.cppc as u8);
                }
            }
            Reply::Event(e) => {
                w = Writer::op(op::R_EVENT);
                w.u32(match e {
                    Event::PowerButton => 1,
                    Event::SleepButton => 2,
                });
            }
        }
        w.finish()
    }

    pub fn decode(b: &[u8]) -> Result<Self> {
        let mut r = Reader::new(b);
        let bool8 = |r: &mut Reader| -> Result<bool> {
            match r.u8()? {
                0 => Ok(false),
                1 => Ok(true),
                _ => Err(Error::InvalidArgs),
            }
        };
        let reply = match r.u32()? {
            op::R_OK => Reply::Ok,
            op::R_FAILED => Reply::Failed(Error::from_code(r.i32()? as i64).ok_or(Error::InvalidArgs)?),
            op::R_STATUS => Reply::Status(Status {
                features: r.u32()?,
                fadt_revision: r.u8()?,
                oem: r.str()?,
                thermal_zones: r.u32()?,
                batteries: r.u32()?,
                cpus: r.u32()?,
            }),
            op::R_THERMAL => {
                let n = read_count(&mut r)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(ThermalZone {
                        name: r.str()?,
                        temp_dk: read_opt(&mut r)?,
                        critical_dk: read_opt(&mut r)?,
                        passive_dk: read_opt(&mut r)?,
                    });
                }
                Reply::Thermal(v)
            }
            op::R_BATTERY => {
                let n = read_count(&mut r)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(Battery {
                        name: r.str()?,
                        present: bool8(&mut r)?,
                        unit_mw: bool8(&mut r)?,
                        state: r.u32()?,
                        rate: read_opt(&mut r)?,
                        remaining: read_opt(&mut r)?,
                        full: read_opt(&mut r)?,
                        design: read_opt(&mut r)?,
                    });
                }
                Reply::Battery(v)
            }
            op::R_CPU_PERF => {
                let n = read_count(&mut r)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(CpuPerf { name: r.str()?, pss_states: r.u32()?, max_mhz: r.u32()?, cppc: bool8(&mut r)? });
                }
                Reply::CpuPerf(v)
            }
            op::R_EVENT => Reply::Event(match r.u32()? {
                1 => Event::PowerButton,
                2 => Event::SleepButton,
                _ => return Err(Error::InvalidArgs),
            }),
            _ => return Err(Error::InvalidArgs),
        };
        if !r.is_empty() {
            return Err(Error::InvalidArgs);
        }
        Ok(reply)
    }
}

/// Teplota v desetinach K jako text ve stupnich Celsia ("25.0 C").
pub fn celsius_text(dk: u32) -> String {
    let c10 = dk as i64 - 2732;
    let sign = if c10 < 0 { "-" } else { "" };
    let a = c10.unsigned_abs();
    alloc::format!("{sign}{}.{} C", a / 10, a % 10)
}
