//! Zpomaleni po neuspesnych pokusech (tabulka v `docs/cs/ucty.md`).
//!
//! Trvale zamceni uctu se nepouziva (kdokoli u klavesnice by ucet
//! majiteli zablokoval); po 7 pokusech je to 4 pokusy za hodinu.

/// Pokusu PINu, po kterych se PIN zablokuje (do prihlaseni heslem nebo
/// klicem).
pub const PIN_ATTEMPTS: u8 = 5;

/// Cekani po `failures` neuspesnych pokusech po sobe (sekundy).
pub const fn delay_secs(failures: u32) -> u64 {
    match failures {
        0..=2 => 0,
        3 => 30,
        4 => 60,
        5 => 120,
        6 => 300,
        _ => 900,
    }
}

/// Kolik ms jeste cekat (`last` = cas posledni chyby, `now` = ted,
/// sekundy realneho casu). Cas jdouci pozpatku (nastaveni hodin) =
/// cekat celou dobu.
pub fn wait_ms(failures: u32, last: i64, now: i64) -> u64 {
    let d = delay_secs(failures);
    if d == 0 {
        return 0;
    }
    let elapsed = if now >= last { (now - last) as u64 } else { 0 };
    d.saturating_sub(elapsed) * 1000
}
