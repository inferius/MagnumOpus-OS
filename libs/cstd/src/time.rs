//! `time.h` a `sys/time.h`: hodiny (`clock_gettime`, `time`,
//! `gettimeofday`), spanek (`nanosleep`, `sleep`), kalendar (`gmtime_r`,
//! `localtime_r`, `mktime`, `timegm`) a `strftime`.
//!
//! Mistni cas: pasmo z promenne `TZ` (jmeno IANA, napr. `Europe/Prague`,
//! nebo pevny posun ve stylu ISO `UTC+2`), jinak UTC. Pravidla pasem jsou
//! v `libs/tz` (bez tzdata).

use core::time::Duration;
use std::sync::Mutex;

use tz::Zone;

use crate::stat::timespec;
use crate::sys;
use crate::{c_char, c_int, c_long, errno, size_t};

pub const CLOCK_REALTIME: c_int = 0;
pub const CLOCK_MONOTONIC: c_int = 1;
pub const CLOCK_PROCESS_CPUTIME_ID: c_int = 2;
pub const CLOCK_THREAD_CPUTIME_ID: c_int = 3;
pub const CLOCK_MONOTONIC_RAW: c_int = 4;
pub const CLOCK_REALTIME_COARSE: c_int = 5;
pub const CLOCK_MONOTONIC_COARSE: c_int = 6;
pub const CLOCK_BOOTTIME: c_int = 7;
pub const TIMER_ABSTIME: c_int = 1;

/// `struct tm` (jako glibc, vcetne `tm_gmtoff` a `tm_zone`).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct tm {
    pub tm_sec: c_int,
    pub tm_min: c_int,
    pub tm_hour: c_int,
    pub tm_mday: c_int,
    pub tm_mon: c_int,
    pub tm_year: c_int,
    pub tm_wday: c_int,
    pub tm_yday: c_int,
    pub tm_isdst: c_int,
    pub tm_gmtoff: c_long,
    pub tm_zone: *const c_char,
}

impl Default for tm {
    fn default() -> Self {
        tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: core::ptr::null(),
        }
    }
}

/// `struct timeval`.
#[repr(C)]
pub struct timeval {
    pub tv_sec: i64,
    pub tv_usec: i64,
}

/// Cas podle hodin v ns; `None` = hodiny nejsou.
pub fn clock_ns(clock: c_int) -> Option<i128> {
    match clock {
        CLOCK_REALTIME | CLOCK_REALTIME_COARSE => {
            sys::realtime().map(|(s, n)| i128::from(s) * 1_000_000_000 + i128::from(n))
        }
        CLOCK_MONOTONIC | CLOCK_MONOTONIC_RAW | CLOCK_MONOTONIC_COARSE | CLOCK_BOOTTIME => {
            Some(i128::from(sys::monotonic_ns()))
        }
        _ => None,
    }
}

pub(crate) fn to_timespec(ns: i128) -> timespec {
    timespec { tv_sec: ns.div_euclid(1_000_000_000) as i64, tv_nsec: ns.rem_euclid(1_000_000_000) as i64 }
}

fn from_timespec(t: &timespec) -> Option<i128> {
    (0..1_000_000_000).contains(&t.tv_nsec).then(|| i128::from(t.tv_sec) * 1_000_000_000 + i128::from(t.tv_nsec))
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn clock_gettime(clock: c_int, out: *mut timespec) -> c_int {
    let Some(ns) = clock_ns(clock) else { return errno::ret(Err(errno::EINVAL)) };
    if out.is_null() {
        return errno::ret(Err(errno::EFAULT));
    }
    // SAFETY: kontrakt C.
    unsafe { *out = to_timespec(ns) };
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn clock_getres(clock: c_int, out: *mut timespec) -> c_int {
    if clock_ns(clock).is_none() {
        return errno::ret(Err(errno::EINVAL));
    }
    if !out.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *out = timespec { tv_sec: 0, tv_nsec: 1 } };
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn time(out: *mut i64) -> i64 {
    let t = sys::realtime().map_or(-1, |(s, _)| s);
    if !out.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *out = t };
    }
    t
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn gettimeofday(tv: *mut timeval, _tz: *mut c_void_t) -> c_int {
    let Some(ns) = clock_ns(CLOCK_REALTIME) else { return errno::ret(Err(errno::EINVAL)) };
    if !tv.is_null() {
        let t = to_timespec(ns);
        // SAFETY: kontrakt C.
        unsafe { *tv = timeval { tv_sec: t.tv_sec, tv_usec: t.tv_nsec / 1000 } };
    }
    0
}

/// `void` pro nepouzity argument `gettimeofday`.
pub type c_void_t = core::ffi::c_void;

/// Spanek; signaly nejsou (rozhodnuti 6), takze se nikdy neprerusi
/// a `rem` je vzdy 0.
fn sleep_for(d: Duration) {
    std::thread::sleep(d);
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn nanosleep(req: *const timespec, rem: *mut timespec) -> c_int {
    // SAFETY: kontrakt C.
    let Some(r) = (unsafe { req.as_ref() }) else { return errno::ret(Err(errno::EFAULT)) };
    let Some(ns) = from_timespec(r).filter(|&n| n >= 0) else { return errno::ret(Err(errno::EINVAL)) };
    crate::pthread::testcancel();
    sleep_for(Duration::from_nanos(ns.min(u64::MAX as i128) as u64));
    if !rem.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *rem = timespec::default() };
    }
    0
}

/// Vraci cislo chyby primo (ne pres errno), jak chce POSIX.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn clock_nanosleep(clock: c_int, flags: c_int, req: *const timespec, rem: *mut timespec) -> c_int {
    // SAFETY: kontrakt C.
    let Some(r) = (unsafe { req.as_ref() }) else { return errno::EFAULT };
    let Some(ns) = from_timespec(r) else { return errno::EINVAL };
    let Some(now) = clock_ns(clock) else { return errno::EINVAL };
    let wait = if flags & TIMER_ABSTIME != 0 { ns - now } else { ns };
    crate::pthread::testcancel();
    if wait > 0 {
        sleep_for(Duration::from_nanos(wait.min(u64::MAX as i128) as u64));
    }
    if !rem.is_null() {
        // SAFETY: kontrakt C.
        unsafe { *rem = timespec::default() };
    }
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn sleep(secs: u32) -> u32 {
    crate::pthread::testcancel();
    sleep_for(Duration::from_secs(u64::from(secs)));
    0
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn usleep(us: u32) -> c_int {
    crate::pthread::testcancel();
    sleep_for(Duration::from_micros(u64::from(us)));
    0
}

/// `clock()`: cas CPU procesu jadro zatim nedava -> `(clock_t)-1` (C to
/// dovoluje).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn clock() -> c_long {
    -1
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn difftime(a: i64, b: i64) -> f64 {
    (a - b) as f64
}

// --- kalendar -------------------------------------------------------------

/// Pasmo procesu (`tzset` z promenne `TZ`) a jeho zkratky jako retezce C.
struct ZoneState {
    zone: Zone,
    std_name: std::ffi::CString,
    dst_name: std::ffi::CString,
}

static ZONE: Mutex<Option<ZoneState>> = Mutex::new(None);

fn utc_zone() -> Zone {
    Zone::fixed(0).expect("posun 0 je platny")
}

/// Pasmo z hodnoty `TZ`: jmeno IANA, nebo posun ve stylu ISO.
pub fn zone_from_tz(tz: Option<&str>) -> Zone {
    tz.map(|s| s.trim_start_matches(':')).and_then(Zone::parse).unwrap_or_else(utc_zone)
}

fn cstring(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap_or_default()
}

fn load_zone() -> std::sync::MutexGuard<'static, Option<ZoneState>> {
    let mut g = ZONE.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let tz = std::env::var("TZ").ok();
        let zone = zone_from_tz(tz.as_deref());
        // Zkratky pro tzname: leden (zimni) a cervenec (letni).
        let std_off = zone.offset_at(0);
        let summer = zone.offset_at(15_638_400);
        let std_name = if std_off.abbrev.is_empty() { "UTC" } else { std_off.abbrev };
        let dst_name = if summer.dst { summer.abbrev } else { std_name };
        *g = Some(ZoneState { zone, std_name: cstring(std_name), dst_name: cstring(dst_name) });
    }
    g
}

/// Rozlozi sekundy UTC na `tm` s posunem `off` (sekundy) a zkratkou.
pub fn split(secs: i64, off: i32, dst: bool, zone: *const c_char) -> Option<tm> {
    let local = secs.checked_add(i64::from(off))?;
    let days = local.div_euclid(86_400);
    let rem = local.rem_euclid(86_400);
    let (y, m, d) = abi::time::civil_from_days(days);
    let jan1 = abi::time::days_from_civil(y, 1, 1)?;
    let year = c_int::try_from(y - 1900).ok()?;
    Some(tm {
        tm_sec: (rem % 60) as c_int,
        tm_min: (rem / 60 % 60) as c_int,
        tm_hour: (rem / 3600) as c_int,
        tm_mday: c_int::from(d),
        tm_mon: c_int::from(m) - 1,
        tm_year: year,
        // 1970-01-01 byl ctvrtek (4).
        tm_wday: (days + 4).rem_euclid(7) as c_int,
        tm_yday: (days - jan1) as c_int,
        tm_isdst: c_int::from(dst),
        tm_gmtoff: c_long::from(off),
        tm_zone: zone,
    })
}

/// Sekundy od 1970 z polozek `tm` (normalizuje mimo rozsah, jako mktime),
/// bez posunu pasma.
pub fn civil_secs(t: &tm) -> Option<i64> {
    let mon = i64::from(t.tm_mon);
    let year = i64::from(t.tm_year) + 1900 + mon.div_euclid(12);
    let month = mon.rem_euclid(12) as u8 + 1;
    let days = abi::time::days_from_civil(year, month, 1)? + i64::from(t.tm_mday) - 1;
    days.checked_mul(86_400)?
        .checked_add(i64::from(t.tm_hour) * 3600 + i64::from(t.tm_min) * 60 + i64::from(t.tm_sec))
}

static UTC_NAME: &[u8] = b"UTC\0";

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn gmtime_r(t: *const i64, out: *mut tm) -> *mut tm {
    // SAFETY: kontrakt C.
    let (Some(&secs), false) = (unsafe { t.as_ref() }, out.is_null()) else { return core::ptr::null_mut() };
    match split(secs, 0, false, UTC_NAME.as_ptr()) {
        Some(v) => {
            // SAFETY: kontrakt C.
            unsafe { *out = v };
            out
        }
        None => {
            errno::set(errno::EOVERFLOW);
            core::ptr::null_mut()
        }
    }
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn localtime_r(t: *const i64, out: *mut tm) -> *mut tm {
    // SAFETY: kontrakt C.
    let (Some(&secs), false) = (unsafe { t.as_ref() }, out.is_null()) else { return core::ptr::null_mut() };
    let g = load_zone();
    let Some(z) = g.as_ref() else { return core::ptr::null_mut() };
    let off = z.zone.offset_at(secs);
    let name = if off.dst { z.dst_name.as_ptr().cast() } else { z.std_name.as_ptr().cast() };
    match split(secs, off.secs, off.dst, name) {
        Some(v) => {
            // SAFETY: kontrakt C.
            unsafe { *out = v };
            out
        }
        None => {
            errno::set(errno::EOVERFLOW);
            core::ptr::null_mut()
        }
    }
}

thread_local! {
    /// Vysledek `gmtime`/`localtime` (staticky per vlakno).
    static TM_BUF: core::cell::UnsafeCell<tm> = core::cell::UnsafeCell::new(tm::default());
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn gmtime(t: *const i64) -> *mut tm {
    // SAFETY: buffer vlakna.
    TM_BUF.with(|b| unsafe { gmtime_r(t, b.get()) })
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn localtime(t: *const i64) -> *mut tm {
    // SAFETY: buffer vlakna.
    TM_BUF.with(|b| unsafe { localtime_r(t, b.get()) })
}

/// `timegm`: `tm` v UTC -> sekundy; `tm` se normalizuje.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn timegm(t: *mut tm) -> i64 {
    // SAFETY: kontrakt C.
    let Some(v) = (unsafe { t.as_mut() }) else { return -1 };
    let Some(secs) = civil_secs(v) else {
        errno::set(errno::EOVERFLOW);
        return -1;
    };
    if let Some(n) = split(secs, 0, false, UTC_NAME.as_ptr()) {
        *v = n;
    }
    secs
}

/// `mktime`: mistni `tm` -> sekundy UTC (pasmo z `TZ`); `tm` se
/// normalizuje. Dvojznacny cas: `tm_isdst` > 0 vybere letni.
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn mktime(t: *mut tm) -> i64 {
    // SAFETY: kontrakt C.
    let Some(v) = (unsafe { t.as_mut() }) else { return -1 };
    let Some(local) = civil_secs(v) else {
        errno::set(errno::EOVERFLOW);
        return -1;
    };
    let utc = {
        let g = load_zone();
        let Some(z) = g.as_ref() else { return -1 };
        match z.zone.local_secs_to_utc(local) {
            tz::LocalResult::Single(s) | tz::LocalResult::Gap(s) => s,
            tz::LocalResult::Ambiguous { earlier, later } => {
                if v.tm_isdst > 0 { earlier } else { later }
            }
        }
    };
    // SAFETY: `v` je platne `tm`.
    unsafe { localtime_r(&utc, v) };
    utc
}

/// Prevede `tm` na sekundy UTC (pro `%s`).
fn tm_utc(t: &tm) -> Option<i64> {
    civil_secs(t).map(|s| s - t.tm_gmtoff)
}

#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub extern "C" fn tzset() {
    *ZONE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    drop(load_zone());
}

const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];

/// ISO 8601 tyden a rok tydne (`%V`, `%G`).
fn iso_week(t: &tm) -> (i64, c_int) {
    let year = i64::from(t.tm_year) + 1900;
    let wday = (t.tm_wday + 6) % 7; // pondeli = 0
    let week = (t.tm_yday - wday + 10) / 7;
    let weeks_in = |y: i64| -> c_int {
        let jan1 = abi::time::days_from_civil(y, 1, 1).unwrap_or(0);
        let d = (jan1 + 3).rem_euclid(7); // pondeli = 0
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        if d == 3 || (d == 2 && leap) { 53 } else { 52 }
    };
    if week < 1 {
        (year - 1, weeks_in(year - 1))
    } else if week > weeks_in(year) {
        (year + 1, 1)
    } else {
        (year, week)
    }
}

/// Formatovani casu (`strftime`), jen locale C.
pub fn format_tm(fmt: &[u8], t: &tm) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let name = |a: &[&str], i: c_int, short: bool| -> String {
        let s = a.get(i.rem_euclid(a.len() as c_int) as usize).copied().unwrap_or("?");
        if short { s[..3].to_string() } else { s.to_string() }
    };
    let hour12 = |h: c_int| if h % 12 == 0 { 12 } else { h % 12 };
    while i < fmt.len() {
        let c = fmt[i];
        i += 1;
        if c != b'%' || i >= fmt.len() {
            out.push(c);
            continue;
        }
        let mut spec = fmt[i];
        i += 1;
        // Modifikatory E a O se ignoruji (locale C).
        if (spec == b'E' || spec == b'O') && i < fmt.len() {
            spec = fmt[i];
            i += 1;
        }
        let year = i64::from(t.tm_year) + 1900;
        let s = match spec {
            b'a' => name(&DAYS, t.tm_wday, true),
            b'A' => name(&DAYS, t.tm_wday, false),
            b'b' | b'h' => name(&MONTHS, t.tm_mon, true),
            b'B' => name(&MONTHS, t.tm_mon, false),
            b'c' => String::from_utf8_lossy(&format_tm(b"%a %b %e %H:%M:%S %Y", t)).into_owned(),
            b'C' => format!("{:02}", year.div_euclid(100)),
            b'd' => format!("{:02}", t.tm_mday),
            b'D' => String::from_utf8_lossy(&format_tm(b"%m/%d/%y", t)).into_owned(),
            b'e' => format!("{:2}", t.tm_mday),
            b'F' => String::from_utf8_lossy(&format_tm(b"%Y-%m-%d", t)).into_owned(),
            b'G' => iso_week(t).0.to_string(),
            b'g' => format!("{:02}", iso_week(t).0.rem_euclid(100)),
            b'H' => format!("{:02}", t.tm_hour),
            b'I' => format!("{:02}", hour12(t.tm_hour)),
            b'j' => format!("{:03}", t.tm_yday + 1),
            b'k' => format!("{:2}", t.tm_hour),
            b'l' => format!("{:2}", hour12(t.tm_hour)),
            b'm' => format!("{:02}", t.tm_mon + 1),
            b'M' => format!("{:02}", t.tm_min),
            b'n' => "\n".to_string(),
            b'p' => (if t.tm_hour < 12 { "AM" } else { "PM" }).to_string(),
            b'P' => (if t.tm_hour < 12 { "am" } else { "pm" }).to_string(),
            b'r' => String::from_utf8_lossy(&format_tm(b"%I:%M:%S %p", t)).into_owned(),
            b'R' => String::from_utf8_lossy(&format_tm(b"%H:%M", t)).into_owned(),
            b's' => tm_utc(t).map_or_else(String::new, |s| s.to_string()),
            b'S' => format!("{:02}", t.tm_sec),
            b't' => "\t".to_string(),
            b'T' => String::from_utf8_lossy(&format_tm(b"%H:%M:%S", t)).into_owned(),
            b'u' => (if t.tm_wday == 0 { 7 } else { t.tm_wday }).to_string(),
            b'U' => format!("{:02}", (t.tm_yday + 7 - t.tm_wday) / 7),
            b'V' => format!("{:02}", iso_week(t).1),
            b'w' => t.tm_wday.to_string(),
            b'W' => format!("{:02}", (t.tm_yday + 7 - (t.tm_wday + 6) % 7) / 7),
            b'x' => String::from_utf8_lossy(&format_tm(b"%m/%d/%y", t)).into_owned(),
            b'X' => String::from_utf8_lossy(&format_tm(b"%H:%M:%S", t)).into_owned(),
            b'y' => format!("{:02}", year.rem_euclid(100)),
            b'Y' => year.to_string(),
            b'z' => {
                let o = t.tm_gmtoff;
                let a = o.abs();
                format!("{}{:02}{:02}", if o < 0 { '-' } else { '+' }, a / 3600, a / 60 % 60)
            }
            b'Z' => {
                if t.tm_zone.is_null() {
                    String::new()
                } else {
                    // SAFETY: tm_zone je retezec C (z gmtime/localtime nebo od volajiciho).
                    String::from_utf8_lossy(unsafe { crate::c_bytes(t.tm_zone) }).into_owned()
                }
            }
            b'%' => "%".to_string(),
            other => {
                // Neznamy specifikator: opsat (glibc dela totez).
                out.push(b'%');
                out.push(other);
                continue;
            }
        };
        out.extend_from_slice(s.as_bytes());
    }
    out
}

/// `strftime`: 0 = vysledek se neveje (obsah `buf` je pak neurcity).
#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]
pub unsafe extern "C" fn strftime(buf: *mut c_char, max: size_t, fmt: *const c_char, t: *const tm) -> size_t {
    // SAFETY: kontrakt C.
    let (Some(t), false, false) = (unsafe { t.as_ref() }, fmt.is_null(), buf.is_null()) else { return 0 };
    // SAFETY: retezec C.
    let out = format_tm(unsafe { crate::c_bytes(fmt) }, t);
    if out.len() + 1 > max {
        return 0;
    }
    // SAFETY: `buf` ma `max` bajtu.
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), buf, out.len());
        *buf.add(out.len()) = 0;
    }
    out.len()
}
