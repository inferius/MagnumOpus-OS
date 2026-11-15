//! Vychod a zapad slunce pro vzhled podle denni doby (`Scheme::Auto`).
//!
//! Zjednoduseny vypocet NOAA (rovnice casu a deklinace z Fourierovy rady,
//! zenit 90,833 stupne = horni okraj slunce s lomem svetla). Chyba je do
//! par minut, na prepnuti motivu staci. Misto je mesto casoveho pasma
//! (`tz::Place`), ne poloha pocitace.
//!
//! Bez mista (pevny posun `UTC+2`) a za polarnim kruhem, kde slunce
//! nevyjde nebo nezapadne, plati pevny den 7-19 h mistniho casu: polarni
//! noc by jinak znamenala tmavy motiv i v poledne.

use core::f32::consts::PI;

/// Den podle pevnych hodin (bez mista, polarni den a noc).
pub const FIXED_DAY: (u32, u32) = (7, 19);

/// Vychod a zapad (sekundy UTC) mistniho dne `day` (dny od 1970-01-01
/// podle mistniho casu) na miste `lat`, `lon` (stupne). `None` = slunce ten
/// den nevyjde nebo nezapadne.
pub fn rise_set(lat: f32, lon: f32, day: i64) -> Option<(i64, i64)> {
    // Den v roce (od 0) bez kalendare: delka tropickeho roku, 2000-01-01 =
    // den 10957. Odchylka do jednoho dne nevadi.
    let d = (day - 10957) as f64;
    let doy = (d - libm::floor(d / 365.2422) * 365.2422) as f32;
    let g = 2.0 * PI / 365.2422 * (doy + 0.5);
    let (s1, c1) = (libm::sinf(g), libm::cosf(g));
    let (s2, c2) = (libm::sinf(2.0 * g), libm::cosf(2.0 * g));
    let (s3, c3) = (libm::sinf(3.0 * g), libm::cosf(3.0 * g));
    // Rovnice casu (minuty) a deklinace (radiany).
    let eqt = 229.18 * (0.000_075 + 0.001_868 * c1 - 0.032_077 * s1 - 0.014_615 * c2 - 0.040_849 * s2);
    let decl = 0.006_918 - 0.399_912 * c1 + 0.070_257 * s1 - 0.006_758 * c2 + 0.000_907 * s2 - 0.002_697 * c3 + 0.001_48 * s3;
    let phi = lat * PI / 180.0;
    let cos_h = libm::cosf(90.833 * PI / 180.0) / (libm::cosf(phi) * libm::cosf(decl)) - libm::tanf(phi) * libm::tanf(decl);
    if !(-1.0..=1.0).contains(&cos_h) {
        return None;
    }
    let ha = libm::acosf(cos_h) * 180.0 / PI;
    // Minuty od pulnoci UTC dne `day` (muze byt mimo 0..1440).
    let rise = 720.0 - 4.0 * (lon + ha) - eqt;
    let set = 720.0 - 4.0 * (lon - ha) - eqt;
    let base = day * 86_400;
    Some((base + (rise * 60.0) as i64, base + (set * 60.0) as i64))
}

/// Je den? `utc` = sekundy UTC, `offset` = mistni posun (s), `place` =
/// misto pasma. Vraci i okamzik (UTC), kdy se to zmeni, at sluzba nemusi
/// hadat.
pub fn is_day(utc: i64, offset: i32, place: Option<(f32, f32)>) -> (bool, i64) {
    let local = utc + offset as i64;
    let day = local.div_euclid(86_400);
    if let Some((lat, lon)) = place
        && let Some((rise, set)) = rise_set(lat, lon, day)
    {
        return if utc < rise {
            (false, rise)
        } else if utc < set {
            (true, set)
        } else {
            // Zitrejsi vychod (nebo pevne hodiny, kdyz zitra nevyjde).
            let next = rise_set(lat, lon, day + 1).map_or((day + 1) * 86_400 - offset as i64 + FIXED_DAY.0 as i64 * 3600, |r| r.0);
            (false, next)
        };
    }
    let sec = local.rem_euclid(86_400);
    let (a, b) = (FIXED_DAY.0 as i64 * 3600, FIXED_DAY.1 as i64 * 3600);
    let midnight = utc - sec;
    if sec < a {
        (false, midnight + a)
    } else if sec < b {
        (true, midnight + b)
    } else {
        (false, midnight + 86_400 + a)
    }
}
