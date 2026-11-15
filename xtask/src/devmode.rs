//! Rezim vyvojare jen ve vyvojovem sestaveni (`docs/cs/ucty.md`, kapitola
//! Rezim vyvojare): cargo feature `dev-mode` zapina xtask jen pro obrazy
//! s vyvojovym profilem (soubor `accounts-dev` v initfs: `xtask run`,
//! zive USB, `xtask vmware`). Obraz pro uzivatele (`xtask image`), testy
//! (`xtask test`) a initfs pro instalaci (`os/instfs.img`) se stavi bez
//! ni, takze kod rezimu v jejich programech vubec neni.
//!
//! Pojistka: v programech kazdeho initfs bez vyvojoveho profilu (i initfs
//! pro instalaci) se hledaji texty, ktere existuji jen v kodu za feature
//! ([`MARKERS`]); nalez = chyba sestaveni (`crate::build`).

use crate::Result;

/// Programy s feature `dev-mode`: politika potvrzeni (accountd), pruh
/// v liste (compd), prikaz `devmode` (shsess) a jeho predani (init).
pub const PACKAGES: &[&str] = &["accountd", "compd", "shsess", "init"];

/// Texty jen z kodu za feature (log accountd, pruh v liste, podpis
/// prikazu `devmode`). Ve vydani nesmi byt v zadnem programu.
pub const MARKERS: &[&str] = &["accountd: rezim vyvojare", "Režim vývojáře - systém není chráněný", "system-dev-mode"];

/// Ma obraz vyvojovy profil (`accounts-dev` v initfs)?
pub fn dev_profile(opts: &crate::Options) -> bool {
    opts.extra_initfs.iter().any(|(n, _)| n == crate::usb::DEV_PROFILE.0)
}

/// Argument `--features` pro `cargo build` userspace (`None` = vydani).
pub fn features(opts: &crate::Options, packages: &[&str]) -> Option<String> {
    if !dev_profile(opts) {
        return None;
    }
    let list: Vec<String> = PACKAGES.iter().filter(|p| packages.contains(p)).map(|p| format!("{p}/dev-mode")).collect();
    (!list.is_empty()).then(|| list.join(","))
}

/// Najde znacku rezimu vyvojare v programech (`(jmeno, obsah)`).
pub fn find_marker<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Option<(String, &'static str)> {
    for (name, data) in files {
        for m in MARKERS {
            if data.windows(m.len()).any(|w| w == m.as_bytes()) {
                return Some((name.to_string(), m));
            }
        }
    }
    None
}

/// Vydani bez rezimu vyvojare: zadny program nesmi obsahovat znacku.
pub fn check_release<'a>(what: &str, files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Result<()> {
    match find_marker(files) {
        Some((name, m)) => {
            Err(format!("{what}: program {name} obsahuje kod rezimu vyvojare ('{m}'); vydani ho mit nesmi").into())
        }
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn znacka_rezimu_ve_vydani_je_chyba() {
        let clean: &[u8] = b"accountd: bezim; ukon 6 dialog";
        assert!(check_release("test", [("accountd", clean)]).is_ok());
        let dev = "xx Režim vývojáře - systém není chráněný yy".as_bytes();
        let e = check_release("test", [("accountd", clean), ("compd", dev)]).unwrap_err().to_string();
        assert!(e.contains("compd"), "{e}");
    }
}
