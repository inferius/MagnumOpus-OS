//! Realny cas (hodiny na zdi).
//!
//! Jadro si pamatuje realny cas v okamziku sveho startu (cas 0 monotonnich
//! hodin); realny cas = tento pocatek + monotonni cas. Zdroj pri startu:
//!
//! 1. UEFI `GetTime` (precetl loader tesne pred ukoncenim boot services,
//!    i s hodnotou TSC - posun do startu jadra se dopocita),
//! 2. jinak hardwarove hodiny platformy (`arch::hardware_clock`, x86: CMOS
//!    RTC, kdyz ho ACPI nehlasi jako chybejici),
//! 3. jinak cas neni znamy (`CLOCK_REALTIME` vrati `NOT_SUPPORTED`).
//!
//! Presnost: firmware i RTC maji rozliseni 1 s, takze pocatek je presny jen
//! na sekundu. Pozdeji ho upresni sitova synchronizace (`source::SET`).

use abi::time::{Reading, WallTime, flags, source};
use boot_info::{BootInfo, FirmwareTime};

use crate::acpi::Platform;
use crate::arch;
use crate::log;
use crate::sync::SpinLock;

struct Origin {
    /// Realny cas v okamziku 0 monotonnich hodin.
    at_zero: WallTime,
    source: u16,
    flags: u16,
}

static ORIGIN: SpinLock<Origin> = SpinLock::new(Origin {
    at_zero: WallTime::UNIX_EPOCH,
    source: source::NONE,
    flags: 0,
});

/// Nastavi pocatek realneho casu ze zdroju pri startu. Volat po
/// inicializaci casovacu (`arch::init_platform`).
pub fn init(boot_info: &BootInfo, platform: &Platform) {
    let fw = &boot_info.firmware_time;
    let origin = if fw.valid == 1 {
        // Firmware cetl pred startem hodin jadra: cas 0 = cteni + uplynuly cas.
        let at_zero = WallTime::new(fw.secs, fw.nanos as i64)
            .add_ns(arch::ns_from_counter_to_start(fw.counter));
        let f = if fw.zone_minutes == FirmwareTime::ZONE_UNSPECIFIED {
            flags::ZONE_ASSUMED_UTC
        } else {
            0
        };
        Some(Origin { at_zero, source: source::UEFI, flags: f })
    } else {
        arch::hardware_clock(platform).map(|t| Origin {
            at_zero: t.add_ns(-(arch::now_ns() as i128)),
            source: source::RTC,
            flags: flags::ZONE_ASSUMED_UTC,
        })
    };
    match origin {
        Some(o) => {
            let c = o.at_zero.add_ns(arch::now_ns() as i128).to_civil();
            log::info!(
                "realny cas: {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC (zdroj {}{})",
                c.year,
                c.month,
                c.day,
                c.hour,
                c.minute,
                c.second,
                if o.source == source::UEFI { "UEFI" } else { "CMOS RTC" },
                if o.flags & flags::ZONE_ASSUMED_UTC != 0 { ", pasmo neuvedeno - bran jako UTC" } else { "" }
            );
            *ORIGIN.lock() = o;
        }
        None => log::warn!("realny cas neni znamy (firmware ani RTC ho nedaly)"),
    }
}

/// Aktualni realny cas; `None` = neni znamy.
pub fn now() -> Option<Reading> {
    let o = ORIGIN.lock();
    if o.source == source::NONE {
        return None;
    }
    Some(Reading {
        time: o.at_zero.add_ns(arch::now_ns() as i128),
        source: o.source,
        flags: o.flags,
    })
}
