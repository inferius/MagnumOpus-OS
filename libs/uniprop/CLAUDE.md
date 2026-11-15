# libs/uniprop

Vlastnosti znaků Unicode bez stavu a alokace (`no_std`): třídy pro `wctype.h` (`is_alpha`, `is_space`, ...), jednoduché převody velikosti písmen, šířka znaku v terminálu (`width` = `wcwidth`). Používá `libs/cstd` (`wchar/wctype.rs`); připravené i pro terminál. Popis: `docs/cs/cstd.md` (kapitola Široké znaky a locale).

## Klíčové soubory

- `src/lib.rs` - dotazy (binární hledání v rozsazích, ASCII rychlou cestou).
- `src/tables.rs` - **generované** `cargo xtask unicode-tables` (`xtask/src/unicode.rs`) z UCD; ručně needitovat.
- `UNICODE-LICENSE.txt` - licence dat UCD (Unicode-3.0), záznam v `THIRD-PARTY.md`.

## Invarianty

- Verze UCD = `core::char::UNICODE_VERSION` toolchainu projektu (test `version_matches_rust_core`). Nový nightly s jinou verzí Unicode: v `xtask/src/unicode.rs` změnit `VERSION` a SHA-256 čtyř souborů, spustit `cargo xtask unicode-tables`, upravit verzi v `THIRD-PARTY.md` a docs.
- `is_alpha`/`is_upper`/`is_lower` a převody velikosti musí odpovídat `char` v Rustu (test přes všechny kódové body).
- `is_space` a `is_blank` bez mezer bez zalomení (U+00A0, U+2007, U+202F), `is_digit`/`is_xdigit` jen ASCII (standard C).

## Testy

- `cargo test -p uniprop` (celý rozsah Unicode, ~0,2 s); parsery generátoru `cargo test -p xtask unicode`.
