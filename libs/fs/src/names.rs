//! Jmena souboru: kontrola, normalizace a porovnani bez ohledu na velikost
//! pismen.
//!
//! - ulozene jmeno = NFC tvar toho, co zadal uzivatel (velikost pismen
//!   zustava),
//! - klic pro porovnani = NFC(mala pismena(NFC(jmeno))): `Foto.JPG`,
//!   `foto.jpg` i `fóto` zapsane rozlozene (o + carka) jsou pro porovnani
//!   stejne jako jejich NFC protejsky,
//! - prevod na mala pismena nezavisi na jazyku (zadna turecka pravidla).
//!
//! Verze tabulek Unicode se uklada do superbloku. TODO: pri pripojeni
//! systemu s jinou verzi prepocitat hashe jmen.

use alloc::string::String;

use abi::{Error, Result};
use unicode_normalization::UnicodeNormalization;

/// Verze Unicode tabulek (major << 16 | minor << 8 | update).
pub const UNICODE_VERSION: u32 = {
    let (a, b, c) = unicode_normalization::UNICODE_VERSION;
    (a as u32) << 16 | (b as u32) << 8 | c as u32
};

/// Nejdelsi jmeno v bajtech UTF-8.
pub const NAME_MAX: usize = 255;

/// Overi jmeno polozky adresare.
pub fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= NAME_MAX
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c == '/' || c.is_control());
    if ok { Ok(()) } else { Err(Error::InvalidArgs) }
}

/// Ulozeny tvar jmena (NFC, velikost pismen zachovana).
pub fn stored_name(name: &str) -> String {
    name.nfc().collect()
}

/// Klic pro porovnani jmen.
pub fn fold_name(name: &str) -> String {
    let lower: String = name.nfc().flat_map(char::to_lowercase).collect();
    lower.nfc().collect()
}

/// Hash slozeneho jmena (FNV-1a 64) - offset polozky adresare ve strome.
pub fn name_hash(folded: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in folded.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        assert_eq!(fold_name("Foto.JPG"), fold_name("foto.jpg"));
        assert_eq!(fold_name("PŘÍLIŠ"), fold_name("příliš"));
        // e + kombinujici carka = e s carkou (NFC).
        assert_eq!(fold_name("Cafe\u{301}"), fold_name("café"));
        assert_ne!(fold_name("a"), fold_name("b"));
        assert_eq!(stored_name("Cafe\u{301}"), "Café");
    }

    #[test]
    fn validation() {
        assert!(validate_name("soubor.txt").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("a\nb").is_err());
        assert!(validate_name(&"x".repeat(256)).is_err());
    }
}
