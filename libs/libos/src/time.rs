//! Cas: monotonni (od startu, pro mereni a deadliny) a realny (hodiny na
//! zdi, pro casy souboru a zobrazeni).
//!
//! Jadro drzi realny cas v UTC. Mistni cas je nastaveni systemu: pasmo
//! a pravidla letniho casu jsou v [`zone`] (kompaktni tabulka, bez tzdata),
//! prevod pres [`local`]. Nastavit hodiny smi jen sluzba casu ([`set`]).

use core::time::Duration;

pub use abi::time::{Civil, Reading, SetRequest, WallTime, flags, set_flags, source};
/// Casova pasma: [`zone::Zone`], [`zone::Settings`], prevody UTC <-> mistni.
pub use tz as zone;

use crate::hw::Resource;
use crate::sys;

/// Realny cas (sekundy a nanosekundy od 1970 UTC); `None` = system ho nezna.
/// Neni monotonni: muze skocit pri nastaveni hodin.
pub fn wall() -> Option<WallTime> {
    sys::clock_realtime().ok().map(|r| r.time)
}

/// Realny cas i se zdrojem a priznaky (napr. pasmo bylo jen predpokladane).
pub fn wall_reading() -> crate::Result<Reading> {
    sys::clock_realtime()
}

/// Aktualni mistni cas v pasmu `zone`; `None` = system realny cas nezna.
pub fn local(zone: &zone::Zone) -> Option<zone::Local> {
    wall().map(|t| zone.to_local(t))
}

/// Nastavi realny cas na `t` (UTC). `clock` = Resource s pravem
/// `SET_CLOCK` (bez nej `ACCESS_DENIED`). S `write_hardware` se cas zapise
/// i do hardwarovych hodin - v mistnim case, kdyz to rika nastaveni
/// (`settings.rtc_local`). Vraci true, kdyz se hardwarove hodiny zapsaly.
pub fn set(
    clock: &Resource,
    t: WallTime,
    settings: &zone::Settings,
    write_hardware: bool,
) -> crate::Result<bool> {
    let req = SetRequest { time: t, rtc_offset: settings.rtc_offset(t) };
    let flags = if write_hardware { set_flags::WRITE_HARDWARE } else { 0 };
    clock.set_clock(&req, flags)
}

/// Plynula korekce realneho casu (`clock` = Resource s `SET_CLOCK`):
/// `offset` = posun k dorovnani bez skoku (nahradi zbytek predchoziho),
/// `freq` = trvala oprava frekvence v ppb. `None` = nemenit. Bez zmen jen
/// vrati stav. Meze: `abi::time::adjust`.
pub fn adjust(
    clock: &Resource,
    offset: Option<i64>,
    freq: Option<i64>,
) -> crate::Result<abi::time::AdjustStatus> {
    use abi::time::adjust as a;
    let mut flags = 0;
    if offset.is_some() {
        flags |= a::OFFSET;
    }
    if freq.is_some() {
        flags |= a::FREQUENCY;
    }
    sys::clock_adjust(
        clock.handle().raw(),
        offset.unwrap_or(0),
        freq.unwrap_or(0),
        flags,
    )
}

/// Oprava pri startu, kdyz je RTC v mistnim case (dual-boot s Windows):
/// jadro precteny cas bralo jako UTC, takze se od nej odecte posun pasma.
/// Hardwarove hodiny se nemeni (zustavaji v mistnim case). Vraci opraveny
/// cas, nebo `None`, kdyz oprava nebyla potreba (viz
/// [`zone::rtc_local_correction`]).
pub fn apply_rtc_local(clock: &Resource, settings: &zone::Settings) -> crate::Result<Option<WallTime>> {
    let reading = sys::clock_realtime()?;
    let read_at = now();
    let Some(fixed) = zone::rtc_local_correction(settings, &reading) else {
        return Ok(None);
    };
    // Mezi ctenim a nastavenim ubehl cas: pricist ho (z monotonnich hodin).
    let fixed = fixed.add_ns(now().saturating_sub(read_at) as i128);
    set(clock, fixed, settings, false)?;
    Ok(Some(fixed))
}

/// Nanosekundy od startu systemu (monotonicky).
pub fn now() -> u64 {
    sys::clock_monotonic()
}

/// Deadline (ns od startu) za `d` od ted.
pub fn deadline_after(d: Duration) -> u64 {
    now().saturating_add(d.as_nanos() as u64)
}

/// Uspi vlakno alespon na `d`.
pub fn sleep(d: Duration) {
    let _ = sys::object_wait_many(&mut [], deadline_after(d));
}
