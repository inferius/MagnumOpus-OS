# design

Klikací HTML prototypy GUI a design systém jako data. `tokens.toml` je **zdroj pravdy** pro vzhled; čte ho `libs/theme` (vkládá se při překladu), z něj se generují `assets/tokens.css` a `tokens.js`. Prototypy: `start`, `pad`, `okna`, `plocha`, `terminal`, `soubory`, `nastaveni`, `prihlaseni`, `instalace`, `design-system`, `prirucka` (příručka grafického jazyka) a další; rozcestník `index.html`, seznam v `README.md`. Popis a rozhodnutí: `docs/cs/vzhled.md`, `docs/en/appearance.md`.

## Klíčové soubory

- `tokens.toml` - barvy, písma, rozestupy, zaoblení, stíny, pohyb, vrstvy kompozitoru; rozměry panelu lišty `size.panel_w`, `panel_row`, `panel_gap`, `panel_edge` (komponenta panel v `libs/ui/src/panel.rs`).
- `tools/build-tokens.mjs` - generuje CSS a JS a kontroluje kontrast WCAG; `tools/build-icons.mjs` - generuje blok `ICONS` v `assets/mo.js` (ze které čerpá i `libs/gfx`).
- `assets/mo.css`, `mo.js` (komponenty, ikony), `assets/logo/`.

## Invarianty

- Žádná čísla vzhledu mimo `tokens.toml`; prototypy i systém čtou stejná data.
- Po změně `tokens.toml` spustit `node design/tools/build-tokens.mjs` a zkontrolovat kontrast (světlý, tmavý, vysoký kontrast).
- Prototyp je návrh, ne popis kódu; rozdíl proti implementaci patří do `docs/cs/vzhled.md` a `docs/cs/gui.md` se značkou stavu.
- `assets/tokens.css`, `tokens.js` jsou generované, needituj ručně; blok `ICONS` v `mo.js` také.

## Testy

- Žádné automatické testy prototypů. Po změně tokenů: `cargo test -p theme -p comp -p ui -p term`. Prohlížení: `python -m http.server 8766 --directory design`. Web docs kopíruje složku do `target/docs/design/` (`cargo xtask docs`).

## Pasti

- Změna tokenu mění vzhled celého systému; ověř světlý i tmavý režim a vysoký kontrast.
- Stránky jsou dvojjazyčné (cs / en); měň obě verze.
