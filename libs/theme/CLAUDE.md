# libs/theme

Motiv vzhledu: tokeny design systému jako data. **Jediný zdroj pravdy je `design/tokens.toml`** (barvy, písma, rozestupy, zaoblení, stíny, pohyb, vrstvy kompozitoru). Knihovna ho čte přes `libs/conf`. `DEFAULT` je `design/tokens.toml` vložený při překladu (`include_str!`), `Tokens::parse` čte libovolný text ve stejném formátu, `Tokens::theme(Mode)` dá hodnoty pro světlý, tmavý nebo vysokokontrastní režim. Barvy jsou premultiplied `0xAARRGGBB` (jako `libs/gfx`), rozměry logické pixely, časy v ms. Popis: `docs/cs/vzhled.md`.

## Invarianty

- V kódu prvků ani kompozitoru nejsou žádná čísla vzhledu; přidáš-li hodnotu, přidej token.
- Změna `design/tokens.toml` zasahuje tři místa: tuhle knihovnu, HTML prototypy (`node design/tools/build-tokens.mjs` vygeneruje `design/assets/tokens.css` a `tokens.js` a zkontroluje kontrast WCAG) a docs.
- Kontrast textu musí splňovat WCAG ve všech režimech včetně vysokého kontrastu.

## Testy

- `cargo test -p theme` (`src/tests.rs`, vestavěný test `tokens_builtin` hlídá, že vložený soubor je platný). Po změně tokenů také `cargo test -p comp -p term`, které motiv používají.

## Pasti

- Volba vzhledu `Pref` (`pref.rs`) je uložený formát (`/system/config/vzhled`): nové klíče jen s výchozí hodnotou, neznámý klíč je chyba čtení.
- `tokens.css` a `tokens.js` jsou generované; needituj je ručně.
- Uživatelské motivy (parse za běhu) mají stejné schéma jako vestavěný; přidání povinného tokenu je lámající změna pro ně.
