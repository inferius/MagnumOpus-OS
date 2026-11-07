//! Kompaktni tabulka pasem (soucasna pravidla, bez historie).
//!
//! Kazde pasmo = standardni posun + nejvys jedno pravidlo letniho casu.
//! Pravidla odpovidaji tzdata pro soucasne roky (overeno testy na znamych
//! prechodech). Pridani pasma = radek v [`ZONES`]; spolecna pravidla jsou
//! konstanty nize.

use crate::{At, Dst, Transition, Zone};

const H: i32 = 3600;
const SUN: u8 = 0;

/// EU (od 1996): posledni nedele v breznu az posledni nedele v rijnu,
/// vzdy v 01:00 UTC (ve vsech pasmech EU naraz).
const EU_START: Transition = Transition::last(3, SUN, H, At::Utc);
const EU_END: Transition = Transition::last(10, SUN, H, At::Utc);

const fn eu(abbrev: &'static str) -> Option<Dst> {
    Some(Dst { save: H, abbrev, start: EU_START, end: EU_END })
}

/// USA a Kanada (od 2007): druha nedele v breznu az prvni nedele v listopadu,
/// ve 02:00 mistniho casu.
const fn us(abbrev: &'static str) -> Option<Dst> {
    Some(Dst {
        save: H,
        abbrev,
        start: Transition::nth(2, 3, SUN, 2 * H, At::Wall),
        end: Transition::nth(1, 11, SUN, 2 * H, At::Wall),
    })
}

/// Jihovychodni Australie (od 2008): prvni nedele v rijnu 02:00 standardniho
/// casu az prvni nedele v dubnu 03:00 letniho (= 02:00 standardniho).
const AU: Option<Dst> = Some(Dst {
    save: H,
    abbrev: "AEDT",
    start: Transition::nth(1, 10, SUN, 2 * H, At::Standard),
    end: Transition::nth(1, 4, SUN, 2 * H, At::Standard),
});

/// Novy Zeland (od 2007): posledni nedele v zari 02:00 az prvni nedele
/// v dubnu 03:00 letniho casu (oba 02:00 standardniho).
const NZ: Option<Dst> = Some(Dst {
    save: H,
    abbrev: "NZDT",
    start: Transition::last(9, SUN, 2 * H, At::Standard),
    end: Transition::nth(1, 4, SUN, 2 * H, At::Standard),
});

const fn zone(name: &'static str, std_offset: i32, std_abbrev: &'static str, dst: Option<Dst>) -> Zone {
    Zone { name, std_offset, std_abbrev, dst }
}

/// Znama pasma. Jmena podle IANA.
pub static ZONES: &[Zone] = &[
    Zone::UTC,
    // Evropa
    zone("Europe/Prague", H, "CET", eu("CEST")),
    zone("Europe/Bratislava", H, "CET", eu("CEST")),
    zone("Europe/Vienna", H, "CET", eu("CEST")),
    zone("Europe/Berlin", H, "CET", eu("CEST")),
    zone("Europe/Warsaw", H, "CET", eu("CEST")),
    zone("Europe/Budapest", H, "CET", eu("CEST")),
    zone("Europe/Zurich", H, "CET", eu("CEST")),
    zone("Europe/Paris", H, "CET", eu("CEST")),
    zone("Europe/Brussels", H, "CET", eu("CEST")),
    zone("Europe/Amsterdam", H, "CET", eu("CEST")),
    zone("Europe/Rome", H, "CET", eu("CEST")),
    zone("Europe/Madrid", H, "CET", eu("CEST")),
    zone("Europe/Copenhagen", H, "CET", eu("CEST")),
    zone("Europe/Stockholm", H, "CET", eu("CEST")),
    zone("Europe/Oslo", H, "CET", eu("CEST")),
    zone("Europe/London", 0, "GMT", eu("BST")),
    zone("Europe/Lisbon", 0, "WET", eu("WEST")),
    zone("Europe/Helsinki", 2 * H, "EET", eu("EEST")),
    zone("Europe/Kyiv", 2 * H, "EET", eu("EEST")),
    zone("Europe/Bucharest", 2 * H, "EET", eu("EEST")),
    zone("Europe/Sofia", 2 * H, "EET", eu("EEST")),
    zone("Europe/Athens", 2 * H, "EET", eu("EEST")),
    zone("Europe/Istanbul", 3 * H, "+03", None),
    zone("Europe/Moscow", 3 * H, "MSK", None),
    // Amerika
    zone("America/New_York", -5 * H, "EST", us("EDT")),
    zone("America/Chicago", -6 * H, "CST", us("CDT")),
    zone("America/Denver", -7 * H, "MST", us("MDT")),
    zone("America/Phoenix", -7 * H, "MST", None),
    zone("America/Los_Angeles", -8 * H, "PST", us("PDT")),
    zone("America/Sao_Paulo", -3 * H, "-03", None),
    // Asie, Oceanie
    zone("Asia/Dubai", 4 * H, "+04", None),
    zone("Asia/Kolkata", 5 * H + 30 * 60, "IST", None),
    zone("Asia/Kathmandu", 5 * H + 45 * 60, "+0545", None),
    zone("Asia/Shanghai", 8 * H, "CST", None),
    zone("Asia/Tokyo", 9 * H, "JST", None),
    zone("Australia/Sydney", 10 * H, "AEST", AU),
    zone("Pacific/Auckland", 12 * H, "NZST", NZ),
];
