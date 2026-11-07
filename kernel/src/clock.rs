//! Realny cas (hodiny na zdi).
//!
//! Jadro si pamatuje realny cas v jednom okamziku monotonnich hodin (zaklad)
//! a od nej ho pocita: uplynuly monotonni cas, trvala oprava frekvence
//! a docasne dorovnavani posunu (slew), viz [`Timeline`] a
//! `abi::time::adjust`. Zdroj pri startu:
//!
//! 1. UEFI `GetTime` (precetl loader tesne pred ukoncenim boot services,
//!    i s hodnotou TSC - posun do startu jadra se dopocita),
//! 2. jinak hardwarove hodiny platformy (`arch::hardware_clock`, x86: CMOS
//!    RTC, kdyz ho ACPI nehlasi jako chybejici),
//! 3. jinak cas neni znamy (`CLOCK_REALTIME` vrati `NOT_SUPPORTED`).
//!
//! Presnost: firmware i RTC maji rozliseni 1 s, takze pocatek je presny jen
//! na sekundu. Za behu ho nastavi sluzba casu (`CLOCK_SET`, zdroj
//! `source::SET`), pozdeji podle site.
//!
//! Jadro drzi jen UTC a casova pasma nezna: prevod na mistni cas a oprava
//! "RTC v mistnim case" (dual-boot s Windows) jsou v userspace (libos).

use abi::time::{AdjustStatus, Reading, SetRequest, WallTime, adjust as adj, flags, source};
use boot_info::{BootInfo, FirmwareTime};

use crate::acpi::Platform;
use crate::arch;
use crate::log;
use crate::sync::SpinLock;

/// Prubeh realneho casu vuci monotonnim hodinam.
///
/// `real(m) = zaklad + d + d * freq / 10^9 + slew * min(d, trvani) / trvani`,
/// kde `d = m - zaklad_m`. Rychlost je vzdy aspon `1 - 1000 ppm`, takze cas
/// nikdy nejde pozpatku: kazdy nanosekundovy krok `d` prida 1 a slozky
/// uberou nejvys 1 (zaokrouhleni dolu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeline {
    /// Monotonni cas zakladu (ns od startu).
    pub base_mono: i128,
    /// Realny cas v zakladu.
    pub base_real: WallTime,
    /// Trvala oprava frekvence (ppb).
    pub freq_ppb: i64,
    /// Posun, ktery se od zakladu dorovnava (ns), a za jak dlouho.
    pub slew_offset: i128,
    pub slew_dur: i128,
}

impl Timeline {
    pub const fn new(base_mono: i128, base_real: WallTime) -> Self {
        Self {
            base_mono,
            base_real,
            freq_ppb: 0,
            slew_offset: 0,
            slew_dur: 0,
        }
    }

    fn slew_part(&self, d: i128) -> i128 {
        if self.slew_dur <= 0 {
            return 0;
        }
        self.slew_offset * d.clamp(0, self.slew_dur) / self.slew_dur
    }

    /// Realny cas v monotonnim okamziku `mono`.
    pub fn real_at(&self, mono: i128) -> WallTime {
        let d = mono - self.base_mono;
        let freq = d * self.freq_ppb as i128 / 1_000_000_000;
        self.base_real.add_ns(d + freq + self.slew_part(d))
    }

    /// Kolik posunu jeste zbyva dorovnat v okamziku `mono`.
    pub fn remaining(&self, mono: i128) -> i128 {
        self.slew_offset - self.slew_part(mono - self.base_mono)
    }

    /// Presune zaklad na `mono` (prubeh casu se nezmeni).
    fn rebase(&mut self, mono: i128) {
        let d = mono - self.base_mono;
        let real = self.real_at(mono);
        let left = self.remaining(mono);
        self.slew_dur = (self.slew_dur - d).max(0);
        self.slew_offset = if self.slew_dur == 0 { 0 } else { left };
        self.base_real = real;
        self.base_mono = mono;
    }

    /// Plynula korekce od okamziku `mono`: novy posun k dorovnani (nahradi
    /// zbytek) a/nebo nova frekvence. Meze overil volajici.
    pub fn adjust(&mut self, mono: i128, offset: Option<i64>, freq: Option<i64>) {
        self.rebase(mono);
        if let Some(f) = freq {
            self.freq_ppb = f;
        }
        if let Some(o) = offset {
            let o = o as i128;
            self.slew_offset = o;
            // Nejvyssi povolenou rychlosti, zaokrouhleno nahoru (rychlost
            // pak nikdy neprekroci mez).
            let rate = adj::MAX_SLEW_PPB as i128;
            self.slew_dur = (o.abs() * 1_000_000_000 + rate - 1) / rate;
            if o == 0 {
                self.slew_dur = 0;
            }
        }
    }

    /// Skok na `real` v okamziku `mono` (`CLOCK_SET`): dorovnavani skonci,
    /// frekvence zustava (vlastnost krystalu se skokem nezmeni).
    pub fn step(&mut self, mono: i128, real: WallTime) {
        *self = Self {
            freq_ppb: self.freq_ppb,
            ..Self::new(mono, real)
        };
    }

    pub fn status(&self, mono: i128) -> AdjustStatus {
        let left = self.remaining(mono);
        let slew = if left != 0 && self.slew_dur > 0 {
            (self.slew_offset * 1_000_000_000 / self.slew_dur) as i64
        } else {
            0
        };
        AdjustStatus {
            remaining_ns: left as i64,
            freq_ppb: self.freq_ppb,
            slew_ppb: slew,
        }
    }
}

struct Origin {
    time: Timeline,
    source: u16,
    flags: u16,
}

static ORIGIN: SpinLock<Origin> = SpinLock::new(Origin {
    time: Timeline::new(0, WallTime::UNIX_EPOCH),
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
        Some(Origin { time: Timeline::new(0, at_zero), source: source::UEFI, flags: f })
    } else {
        arch::hardware_clock(platform).map(|t| Origin {
            time: Timeline::new(arch::now_ns() as i128, t),
            source: source::RTC,
            flags: flags::ZONE_ASSUMED_UTC,
        })
    };
    match origin {
        Some(o) => {
            let c = o.time.real_at(arch::now_ns() as i128).to_civil();
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
        time: o.time.real_at(arch::now_ns() as i128),
        source: o.source,
        flags: o.flags,
    })
}

/// Zamek zapisu do hardwarovych hodin: cas pro hodiny se cte az pod nim,
/// takze posledni zapis ma vzdy nejnovejsi cas systemu (i kdyz soubezne
/// `CLOCK_SET` dobehnou v jinem poradi, nez zacaly).
static HW_WRITE: SpinLock<()> = SpinLock::new(());

/// Nastavi realny cas (`CLOCK_SET`); prava overil volajici, `by` = kdo
/// nastavuje (do logu). Vraci true, kdyz se cas zapsal i do hardwarovych
/// hodin (`write_hardware`).
///
/// Do hardwarovych hodin jde `UTC + rtc_offset` (jadro pasmo nezna, posun
/// posila sluzba casu). Zapis ceka (spankem, nejvys 1 s) na zlomek sekundy
/// [`arch::HARDWARE_CLOCK_PHASE_NS`], aby hodiny tikaly na hranici sekund,
/// a zapisuje aktualni cas systemu v tom okamziku.
pub fn set(req: SetRequest, write_hardware: bool, by: core::fmt::Arguments<'_>) -> bool {
    let now = arch::now_ns() as i128;
    let old = {
        let mut o = ORIGIN.lock();
        let old = (o.source != source::NONE).then(|| o.time.real_at(now));
        o.time.step(now, req.time);
        o.source = source::SET;
        o.flags = 0;
        old
    };
    let c = req.time.to_civil();
    let shift_ms = old.map(|old| (req.time.as_nanos() - old.as_nanos()) / 1_000_000);
    log::info!(
        "{by} nastavil realny cas: {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC (posun {})",
        c.year,
        c.month,
        c.day,
        c.hour,
        c.minute,
        c.second,
        ShiftMs(shift_ms)
    );
    if !write_hardware {
        return false;
    }
    let Some(platform) = crate::PLATFORM.get() else {
        return false;
    };
    let offset_ns = req.rtc_offset as i128 * 1_000_000_000;
    let hw_now = || {
        let t = ORIGIN.lock().time;
        t.real_at(arch::now_ns() as i128).add_ns(offset_ns)
    };
    let frac = hw_now().nanos;
    let wait = (arch::HARDWARE_CLOCK_PHASE_NS as i64 - frac as i64).rem_euclid(1_000_000_000);
    if wait > 0 {
        crate::object::wait::sleep(core::time::Duration::from_nanos(wait as u64));
    }
    let ok = {
        let _guard = HW_WRITE.lock();
        arch::set_hardware_clock(platform, hw_now())
    };
    if !ok {
        log::warn!("hardwarove hodiny nejde nastavit (nejsou, nebo je cas mimo jejich rozsah)");
    }
    ok
}

/// Plynula korekce (`CLOCK_ADJUST`); prava a meze overil volajici.
/// Realny cas, ktery jeste neni znamy, nejde dorovnavat (`BAD_STATE`).
pub fn adjust(
    offset: Option<i64>,
    freq: Option<i64>,
    by: core::fmt::Arguments<'_>,
) -> Result<AdjustStatus, crate::error::Error> {
    let now = arch::now_ns() as i128;
    let status = {
        let mut o = ORIGIN.lock();
        if o.source == source::NONE {
            return Err(crate::error::Error::BadState);
        }
        o.time.adjust(now, offset, freq);
        o.time.status(now)
    };
    // Jen zmena (dotaz na stav bez priznaku se neloguje - ntpd a testy
    // se ptaji casto).
    if offset.is_some() || freq.is_some() {
        log::debug!(
            "{by} doladuje realny cas: zbyva {} us, frekvence {} ppb, slew {} ppb",
            status.remaining_ns / 1000,
            status.freq_ppb,
            status.slew_ppb
        );
    }
    Ok(status)
}

/// Posun hodin pro log; `None` = cas predtim nebyl znamy.
struct ShiftMs(Option<i128>);

impl core::fmt::Display for ShiftMs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(ms) => write!(f, "{ms:+} ms"),
            None => f.write_str("predtim neznamy"),
        }
    }
}
