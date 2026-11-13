//! Odhad zbyvajiciho casu: citelny (vyhlazeny, jen klesa, zaokrouhleny
//! podle radu), prvni chvili "odhaduji...", kratke ulohy bez odhadu.

use crate::ui::progress::Track;

const S: i64 = 1_000_000_000;

/// Stopa s rovnomernou rychlosti `rate`/s od casu 0 do `secs`.
fn steady(rate: u64, secs: i64) -> Track {
    let mut t = Track::default();
    for k in 0..=secs {
        t.push(k * S, rate * k as u64);
    }
    t
}

#[test]
fn warmup_short_and_rounding() {
    let t = steady(10, 1);
    assert_eq!(t.eta_text(S, 1000).as_deref(), Some("odhaduji\u{2026}"), "prvni 2 s");
    // Kratka uloha: zbyva par sekund -> nic.
    let t = steady(10, 3);
    assert_eq!(t.eta_text(3 * S, 60), None);
    // Zaokrouhleni: do 10 s po sekunde, do minuty po 5 s, pak minuty, hodiny.
    let t = steady(10, 3);
    assert_eq!(t.eta_text(3 * S, 30 + 80).as_deref(), Some("8 s"));
    let t = steady(10, 3);
    assert_eq!(t.eta_text(3 * S, 30 + 470).as_deref(), Some("45 s"));
    let t = steady(10, 3);
    assert_eq!(t.eta_text(3 * S, 30 + 1800).as_deref(), Some("~3 min"));
    let t = steady(10, 3);
    assert_eq!(t.eta_text(3 * S, 30 + 48_000).as_deref(), Some("~1 h 20 min"));
}

#[test]
fn only_counts_down_and_refreshes_by_length() {
    // 10/s, celkem 3030: po 3 s zbyva 3000 = 5 min.
    let mut t = steady(10, 3);
    assert_eq!(t.eta_text(3 * S, 3030).as_deref(), Some("~5 min"));
    // Kratke zpomaleni: odhad nahoru neskoci.
    t.push(4 * S, 35);
    assert_eq!(t.eta_text(4 * S, 3030).as_deref(), Some("~5 min"));
    // Obnova az po ~1/10 zbyvajiciho casu (nejvys 5 s): mezitim jen ubyva.
    assert_eq!(t.eta_text(6 * S, 3030).as_deref(), Some("~5 min"));
    // Velka zmena rychlosti (desetkrat pomaleji) odhad zvedne.
    for k in 5..40 {
        t.push(k * S, 35 + (k as u64 - 4));
    }
    let up = t.eta_text(40 * S, 3030).unwrap();
    assert!(up.contains("h") || up.contains("min") && up != "~5 min", "{up}");
}
