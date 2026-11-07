//! Realny cas (hodiny na zdi): reprezentace a prevod na kalendarni datum.
//!
//! Cas = sekundy od 1970-01-01 00:00:00 UTC (se znamenkem, takze i pred
//! rokem 1970) plus nanosekundy uvnitr sekundy (0..1e9). Rozsah i64 sekund
//! je +/- 292 miliard let; pro kalendar (proleptic gregoriansky) se pouziva
//! jen rozsah, kde se rok vejde do i64.
//!
//! Prestupne sekundy se nepocitaji (jako POSIX): kazdy den ma 86400 s.

/// Zdroj realneho casu.
pub mod source {
    /// Cas neni znamy (zadne hodiny).
    pub const NONE: u16 = 0;
    /// Z firmwaru UEFI (`GetTime`) pri startu.
    pub const UEFI: u16 = 1;
    /// Z hodin CMOS (RTC) primo jadrem.
    pub const RTC: u16 = 2;
    /// Nastaveny za behu (`CLOCK_SET`: sluzba casu, synchronizace podle site).
    pub const SET: u16 = 3;
}

/// Priznaky realneho casu.
pub mod flags {
    /// Hodiny firmwaru nehlasi casove pasmo; cas byl bran jako UTC.
    /// (Windows ma v RTC casto mistni cas - pak je cas posunuty o pasmo.)
    pub const ZONE_ASSUMED_UTC: u16 = 1;
}

/// Realny cas: sekundy a nanosekundy od 1970-01-01 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct WallTime {
    /// Sekundy od 1970-01-01 UTC (zaporne = drive).
    pub secs: i64,
    /// Nanosekundy uvnitr sekundy, vzdy 0..1_000_000_000.
    pub nanos: u32,
}

impl WallTime {
    pub const UNIX_EPOCH: Self = Self { secs: 0, nanos: 0 };

    /// Normalizovany cas: nanosekundy mimo rozsah se prevedou na sekundy.
    pub const fn new(secs: i64, nanos: i64) -> Self {
        let s = secs.saturating_add(nanos.div_euclid(1_000_000_000));
        Self {
            secs: s,
            nanos: nanos.rem_euclid(1_000_000_000) as u32,
        }
    }

    /// Posun o `ns` nanosekund (se znamenkem), saturuje.
    pub const fn add_ns(self, ns: i128) -> Self {
        let total = self.nanos as i128 + ns;
        let secs = (self.secs as i128).saturating_add(total.div_euclid(1_000_000_000));
        let secs = if secs > i64::MAX as i128 {
            i64::MAX
        } else if secs < i64::MIN as i128 {
            i64::MIN
        } else {
            secs as i64
        };
        Self {
            secs,
            nanos: total.rem_euclid(1_000_000_000) as u32,
        }
    }

    /// Nanosekundy od 1970 jako i128 (bez ztraty pro cely rozsah).
    pub const fn as_nanos(self) -> i128 {
        self.secs as i128 * 1_000_000_000 + self.nanos as i128
    }

    /// Kalendarni datum a cas v UTC.
    pub fn to_civil(self) -> Civil {
        let days = self.secs.div_euclid(86_400);
        let rest = self.secs.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(days);
        Civil {
            year,
            month,
            day,
            hour: (rest / 3600) as u8,
            minute: (rest / 60 % 60) as u8,
            second: (rest % 60) as u8,
            nanos: self.nanos,
        }
    }

    /// Z kalendarniho data v UTC; `None` = neplatne datum nebo mimo rozsah.
    pub fn from_civil(c: &Civil) -> Option<Self> {
        if !c.is_valid() {
            return None;
        }
        let days = days_from_civil(c.year, c.month, c.day)?;
        let secs = days
            .checked_mul(86_400)?
            .checked_add(c.hour as i64 * 3600 + c.minute as i64 * 60 + c.second as i64)?;
        Some(Self { secs, nanos: c.nanos })
    }
}

/// Odpoved `CLOCK_REALTIME` (16 bajtu, little endian: sekundy i64,
/// nanosekundy u32, zdroj u16 z [`source`], priznaky u16 z [`flags`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub time: WallTime,
    pub source: u16,
    pub flags: u16,
}

impl Reading {
    pub const SIZE: usize = 16;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..8].copy_from_slice(&self.time.secs.to_le_bytes());
        b[8..12].copy_from_slice(&self.time.nanos.to_le_bytes());
        b[12..14].copy_from_slice(&self.source.to_le_bytes());
        b[14..16].copy_from_slice(&self.flags.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let secs = i64::from_le_bytes(b[0..8].try_into().unwrap());
        let nanos = u32::from_le_bytes(b[8..12].try_into().unwrap());
        Self {
            time: WallTime::new(secs, nanos as i64),
            source: u16::from_le_bytes([b[12], b[13]]),
            flags: u16::from_le_bytes([b[14], b[15]]),
        }
    }
}

/// Priznaky syscallu `CLOCK_SET`.
pub mod set_flags {
    /// Zapsat cas i do hardwarovych hodin (x86: CMOS RTC), aby platil i po
    /// restartu. Kdyz platforma hodiny nema (aarch64 zatim), nastavi se jen
    /// cas systemu a syscall vrati 0.
    pub const WRITE_HARDWARE: u64 = 1;
    /// Vsechny zname priznaky (ostatni bity = `INVALID_ARGS`).
    pub const ALL: u64 = WRITE_HARDWARE;
}

/// Plynula korekce realneho casu (`CLOCK_ADJUST`).
///
/// Realny cas bezi rychlosti `1 + (frekvence + slew) / 10^9` vuci
/// monotonnim hodinam:
/// - **frekvence** = trvala oprava rychlosti krystalu (hodiny trvale
///   predbihaji nebo se zpozduji),
/// - **slew** = docasne zrychleni nebo zpomaleni, dokud se nedorovna
///   zadany posun; pak skonci samo.
///
/// Obe slozky jsou omezene, takze realny cas nikdy nejde pozpatku ani se
/// nezastavi (na rozdil od `CLOCK_SET`, ktery skoci).
pub mod adjust {
    /// Zacit dorovnavat posun `offset_ns` (nahradi zbytek predchoziho).
    pub const OFFSET: u64 = 1;
    /// Nastavit trvalou opravu frekvence `freq_ppb`.
    pub const FREQUENCY: u64 = 2;
    pub const ALL: u64 = OFFSET | FREQUENCY;
    /// Nejvetsi rychlost dorovnavani: 500 ppm (0,5 ms za sekundu; jako
    /// `adjtime` v Unixu). Posun 1 s se dorovna za 2000 s.
    pub const MAX_SLEW_PPB: i64 = 500_000;
    /// Nejvetsi trvala oprava frekvence: 500 ppm (bezny krystal je pod
    /// 100 ppm; vic znamena vadne hodiny nebo chybne mereni).
    pub const MAX_FREQ_PPB: i64 = 500_000;
    /// Nejvetsi posun pro dorovnani: 1 hodina (vetsi = `CLOCK_SET`).
    pub const MAX_OFFSET_NS: i64 = 3_600_000_000_000;
}

/// Stav plynule korekce (vystup `CLOCK_ADJUST`, 24 B LE: zbyva dorovnat
/// ns i64, frekvence ppb i64, aktualni slew ppb i64).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdjustStatus {
    pub remaining_ns: i64,
    pub freq_ppb: i64,
    pub slew_ppb: i64,
}

impl AdjustStatus {
    pub const SIZE: usize = 24;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..8].copy_from_slice(&self.remaining_ns.to_le_bytes());
        b[8..16].copy_from_slice(&self.freq_ppb.to_le_bytes());
        b[16..24].copy_from_slice(&self.slew_ppb.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8; Self::SIZE]) -> Self {
        let v = |i: usize| i64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        Self {
            remaining_ns: v(0),
            freq_ppb: v(8),
            slew_ppb: v(16),
        }
    }
}

/// Nejvetsi posun casoveho pasma od UTC (absolutne): 18 hodin. Mez pro
/// `SetRequest::rtc_offset` i pevne posuny v knihovne pasem.
pub const MAX_ZONE_OFFSET: i32 = 18 * 3600;

/// Vstup `CLOCK_SET` (16 B, little endian: sekundy i64, nanosekundy u32,
/// posun hardwarovych hodin i32).
///
/// `rtc_offset` = sekundy pridane k UTC pri zapisu do hardwarovych hodin.
/// Jadro drzi jen UTC a pasma nezna; kdyz ma RTC byt v mistnim case
/// (dual-boot s Windows), posle sluzba casu posun pasma v danem okamziku
/// (napr. +7200 v lete v Praze). Pro RTC v UTC je 0.
///
/// Platny vstup: cas v [`SetRequest::MIN_SECS`]..=[`SetRequest::MAX_SECS`]
/// (roky 1970..=9999), nanosekundy pod 10^9, `|rtc_offset|` nejvys
/// [`MAX_ZONE_OFFSET`]. Jinak `INVALID_ARGS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetRequest {
    pub time: WallTime,
    pub rtc_offset: i32,
}

impl SetRequest {
    pub const SIZE: usize = 16;
    /// Nejmensi nastavitelny cas: 1970-01-01 00:00:00 UTC.
    pub const MIN_SECS: i64 = 0;
    /// Nejvetsi nastavitelny cas: 9999-12-31 23:59:59 UTC.
    pub const MAX_SECS: i64 = 253_402_300_799;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..8].copy_from_slice(&self.time.secs.to_le_bytes());
        b[8..12].copy_from_slice(&self.time.nanos.to_le_bytes());
        b[12..16].copy_from_slice(&self.rtc_offset.to_le_bytes());
        b
    }

    /// `None` = neplatny vstup (viz popis typu: rozsah casu, nanosekund
    /// a posunu).
    pub fn decode(b: &[u8; Self::SIZE]) -> Option<Self> {
        let secs = i64::from_le_bytes(b[0..8].try_into().unwrap());
        let nanos = u32::from_le_bytes(b[8..12].try_into().unwrap());
        let rtc_offset = i32::from_le_bytes(b[12..16].try_into().unwrap());
        let valid = (Self::MIN_SECS..=Self::MAX_SECS).contains(&secs)
            && nanos < 1_000_000_000
            && rtc_offset.unsigned_abs() <= MAX_ZONE_OFFSET as u32;
        valid.then_some(Self {
            time: WallTime { secs, nanos },
            rtc_offset,
        })
    }
}

/// Kalendarni datum a cas (proleptic gregoriansky kalendar, rok 0 = 1 pr. n. l.).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Civil {
    pub year: i64,
    /// 1..=12
    pub month: u8,
    /// 1..=31
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanos: u32,
}

impl Civil {
    pub fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
            && self.nanos < 1_000_000_000
    }
}

pub const fn is_leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub const fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Pocet dni od 1970-01-01 (algoritmus H. Hinnanta "days_from_civil").
/// `None` = vysledek se nevejde do i64.
pub fn days_from_civil(year: i64, month: u8, day: u8) -> Option<i64> {
    let y = if month <= 2 { year.checked_sub(1)? } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400); // 0..=399
    let m = month as i64;
    let mp = if m > 2 { m - 3 } else { m + 9 }; // brezen = 0
    let doy = (153 * mp + 2) / 5 + day as i64 - 1; // 0..=365
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // 0..=146096
    era.checked_mul(146_097)?.checked_add(doe - 719_468)
}

/// Den v tydnu dne `days` od 1970-01-01 (ctvrtek); 0 = nedele .. 6 = sobota.
pub const fn weekday(days: i64) -> u8 {
    (days.rem_euclid(7) as u8 + 4) % 7
}

/// Opak [`days_from_civil`]: (rok, mesic, den).
pub fn civil_from_days(days: i64) -> (i64, u8, u8) {
    // Posun na 0000-03-01 v i128, aby nepretekl ani krajni rozsah i64.
    let z = days as i128 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // 0..=146096
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // 0..=399
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // 0..=365
    let mp = (5 * doy + 2) / 153; // 0..=11
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year as i64, month, day)
}
