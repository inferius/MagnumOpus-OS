//! Testy userspace jen v testovacim sestaveni (Pevny zaklad, bod 2, krok 0).
//!
//! Init ma testy (`user/init/src/tests/`) za cargo feature `tests`; xtask ji
//! zapina jen pri `cargo xtask test` (`opts.test`). Vsechno ostatni - obraz
//! (`xtask image`), `xtask run`, zive USB, initfs pro instalaci
//! (`os/instfs.img`), balicek aktualizace - se stavi bez ni.
//!
//! Pojistka: v programech kazdeho initfs bez testu se hledaji texty, ktere
//! existuji jen v kodu testu ([`MARKERS`]); nalez je tvrda chyba sestaveni
//! (`crate::build`), ne varovani. Stejne jako `devmode.rs` pro rezim
//! vyvojare.

use crate::Result;

/// Programy s feature `tests`.
pub const PACKAGES: &[&str] = &["init"];

/// Texty jen z kodu testu (souhrn behu, kontrola stavu po sade, zaverecny
/// uklid sad). V produkcnim initfs nesmi byt v zadnem programu.
pub const MARKERS: &[&str] = &["testu userspace proslo", "stav po sade", "uklid: pracovni soubory sady"];

/// Argument `--features` pro `cargo build` userspace (`None` = bez testu).
pub fn features(opts: &crate::Options, packages: &[&str]) -> Option<String> {
    if !opts.test {
        return None;
    }
    let list: Vec<String> = PACKAGES.iter().filter(|p| packages.contains(p)).map(|p| format!("{p}/tests")).collect();
    (!list.is_empty()).then(|| list.join(","))
}

/// Initfs bez testu: zadny program nesmi obsahovat znacku testu.
pub fn check_release<'a>(what: &str, files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<()> {
    for (name, data) in files {
        for m in MARKERS {
            if data.windows(m.len()).any(|w| w == m.as_bytes()) {
                return Err(format!("{what}: program {name} obsahuje kod testu ('{m}'); produkcni sestaveni ho mit nesmi").into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "testcode_tests.rs"]
mod tests;
