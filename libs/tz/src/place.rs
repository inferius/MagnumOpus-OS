//! Misto pasma: souradnice mesta, podle ktereho se pasmo jmenuje (IANA
//! `zone1970.tab`, zaokrouhleno na setiny stupne). Slouzi jen k odhadu
//! vychodu a zapadu slunce (vzhled podle denni doby, `docs/cs/vzhled.md`);
//! poloha pocitace to neni a system ji nezjistuje. Pevny posun (`UTC+2`)
//! misto nema.

/// Zemepisna sirka a delka ve stupnich (sever a vychod kladne).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub lat: f32,
    pub lon: f32,
}

const fn p(lat: f32, lon: f32) -> Place {
    Place { lat, lon }
}

/// Souradnice pasem z [`crate::ZONES`] (UTC zadne).
static PLACES: &[(&str, Place)] = &[
    ("Europe/Prague", p(50.08, 14.43)),
    ("Europe/Bratislava", p(48.15, 17.12)),
    ("Europe/Vienna", p(48.22, 16.33)),
    ("Europe/Berlin", p(52.50, 13.37)),
    ("Europe/Warsaw", p(52.25, 21.00)),
    ("Europe/Budapest", p(47.50, 19.08)),
    ("Europe/Zurich", p(47.38, 8.53)),
    ("Europe/Paris", p(48.87, 2.33)),
    ("Europe/Brussels", p(50.83, 4.33)),
    ("Europe/Amsterdam", p(52.37, 4.90)),
    ("Europe/Rome", p(41.90, 12.48)),
    ("Europe/Madrid", p(40.40, -3.68)),
    ("Europe/Copenhagen", p(55.67, 12.58)),
    ("Europe/Stockholm", p(59.33, 18.05)),
    ("Europe/Oslo", p(59.92, 10.75)),
    ("Europe/London", p(51.51, -0.13)),
    ("Europe/Lisbon", p(38.72, -9.13)),
    ("Europe/Helsinki", p(60.17, 24.97)),
    ("Europe/Kyiv", p(50.43, 30.52)),
    ("Europe/Bucharest", p(44.43, 26.10)),
    ("Europe/Sofia", p(42.68, 23.32)),
    ("Europe/Athens", p(37.97, 23.72)),
    ("Europe/Istanbul", p(41.02, 28.97)),
    ("Europe/Moscow", p(55.76, 37.62)),
    ("America/New_York", p(40.71, -74.01)),
    ("America/Chicago", p(41.85, -87.65)),
    ("America/Denver", p(39.74, -104.98)),
    ("America/Phoenix", p(33.45, -112.07)),
    ("America/Los_Angeles", p(34.05, -118.24)),
    ("America/Sao_Paulo", p(-23.53, -46.62)),
    ("Asia/Dubai", p(25.30, 55.30)),
    ("Asia/Kolkata", p(22.53, 88.37)),
    ("Asia/Kathmandu", p(27.72, 85.32)),
    ("Asia/Shanghai", p(31.23, 121.47)),
    ("Asia/Tokyo", p(35.65, 139.74)),
    ("Australia/Sydney", p(-33.87, 151.22)),
    ("Pacific/Auckland", p(-36.87, 174.77)),
];

impl crate::Zone {
    /// Misto pasma z tabulky; `None` pro UTC a pevny posun.
    pub fn place(&self) -> Option<Place> {
        PLACES.iter().find(|(n, _)| *n == self.name).map(|(_, p)| *p)
    }
}

#[cfg(test)]
#[test]
fn every_named_zone_has_place() {
    for z in crate::ZONES {
        assert_eq!(z.place().is_some(), z.name != "UTC", "{}", z.name);
    }
    let p = crate::Zone::by_name("Europe/Prague").unwrap().place().unwrap();
    assert!((p.lat - 50.08).abs() < 0.01 && (p.lon - 14.43).abs() < 0.01);
    assert_eq!(crate::Zone::parse("UTC+2").unwrap().place(), None);
}
