//! Casy FAT (datum a cas bez pasma, rozliseni 2 s, od roku 1980) a NTFS
//! (100 ns od roku 1601, UTC) na sekundy od 1970 UTC a zpet.
//!
//! FAT neuklada pasmo; Windows tam pise mistni cas. Bereme ho jako UTC
//! (rozdil hodin pri vymene s Windows je znamy kompromis, stejne jako
//! u Linuxu bez volby `tz=`).

/// Dny od 1970-01-01 pro datum (gregoriansky kalendar).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Datum (rok, mesic, den) pro dny od 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Cas FAT -> sekundy od 1970.
pub fn from_fat(dt: fatfs::DateTime) -> i64 {
    let days = days_from_civil(dt.date.year as i64, dt.date.month as u32, dt.date.day as u32);
    days * 86400 + dt.time.hour as i64 * 3600 + dt.time.min as i64 * 60 + dt.time.sec as i64
}

/// Sekundy od 1970 -> cas FAT (mimo rozsah 1980..2107 = mez rozsahu).
pub fn to_fat(secs: i64) -> fatfs::DateTime {
    let min = days_from_civil(1980, 1, 1) * 86400;
    let max = days_from_civil(2107, 12, 31) * 86400 + 86399;
    let s = secs.clamp(min, max);
    let (y, m, d) = civil_from_days(s.div_euclid(86400));
    let r = s.rem_euclid(86400);
    fatfs::DateTime {
        date: fatfs::Date { year: y as u16, month: m as u16, day: d as u16 },
        time: fatfs::Time { hour: (r / 3600) as u16, min: (r % 3600 / 60) as u16, sec: (r % 60) as u16, millis: 0 },
    }
}

/// Cas NTFS (100 ns od 1601-01-01) -> sekundy od 1970.
pub fn from_ntfs(t: u64) -> i64 {
    (t / 10_000_000) as i64 - 11_644_473_600
}

/// Hodiny pro fatfs (cas vytvoreni a zmeny souboru): systemovy realny cas.
#[derive(Debug)]
pub struct Clock;

pub static CLOCK: Clock = Clock;

impl fatfs::TimeProvider for Clock {
    fn get_current_date(&self) -> fatfs::Date {
        self.get_current_date_time().date
    }

    fn get_current_date_time(&self) -> fatfs::DateTime {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        to_fat(secs)
    }
}
