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

/// Nejdelsi jmeno ve znacich Unicode (po NFC).
pub const NAME_MAX_CHARS: usize = 1024;
/// Nejdelsi jmeno v bajtech UTF-8 (po NFC).
pub const NAME_MAX: usize = 4096;

/// Ridici znaky smeru textu: vlozeni a prepsani smeru (U+202A-202E),
/// izolace (U+2066-2069) a znacky smeru LRM, RLM, ALM (U+200E, U+200F,
/// U+061C). Umi zmenit poradi zobrazeni: `faktura[U+202E]fdp.exe` se ukaze
/// jako `fakturaexe.pdf`.
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// Overi jmeno polozky adresare. Delka se meri na NFC tvaru, ktery se
/// opravdu ulozi (NFC muze jmeno i prodlouzit).
///
/// Zakazane: prazdne, `.`, `..`, `/`, ridici znaky (vcetne NUL), ridici
/// znaky smeru textu. Zbytek (napr. `:` nebo `\\` pro Windows) resi
/// ovladace jinych systemu prekladem.
pub fn validate_name(name: &str) -> Result<()> {
    let nfc = stored_name(name);
    let ok = !nfc.is_empty()
        && nfc.len() <= NAME_MAX
        && nfc.chars().count() <= NAME_MAX_CHARS
        && nfc != "."
        && nfc != ".."
        && !nfc.chars().any(|c| c == '/' || c.is_control() || is_bidi_control(c));
    if ok { Ok(()) } else { Err(Error::InvalidArgs) }
}

/// Levna kontrola jmena, ktere uz je ulozene (NFC uz bylo pri ulozeni):
/// delka a zakazane znaky, bez normalizace. Pro kontrolu konzistence.
pub fn valid_stored(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name != "."
        && name != ".."
        && !name.chars().any(|c| c == '/' || c.is_control() || is_bidi_control(c))
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
        assert!(validate_name(&"x".repeat(1024)).is_ok());
        assert!(validate_name(&"x".repeat(1025)).is_err());
        // 1024 ctyrbajtovych znaku = 4096 B: jeste projde.
        assert!(validate_name(&"\u{1F600}".repeat(1024)).is_ok());
        assert!(validate_name("faktura\u{202E}fdp.exe").is_err());
        assert!(validate_name("a\u{2067}b").is_err());
        assert!(validate_name("a\u{200F}.exe").is_err());
        assert!(validate_name("a\u{061C}b").is_err());
        assert!(validate_name("a\u{0}b").is_err());
    }
}
