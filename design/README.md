# Vzhled MagnumOpus-OS / MagnumOpus-OS look and feel

Klikací prototypy GUI a tokeny design systému. Popis návrhu:
[docs/cs/vzhled.md](../docs/cs/vzhled.md).
Clickable GUI prototypes and design-system tokens. Design description:
[docs/en/appearance.md](../docs/en/appearance.md).

| Soubor / File | Obsah / Contents |
|---|---|
| `start.html`, `start-varianty.html` | startovací obrazovka a její varianty (archiv) / boot screen and its variants (archive) |
| `pad.html`, `pad-varianty.html` | obrazovka kritického pádu a její varianty (archiv) / critical crash screen and its variants (archive) |
| `instalace.html` | živé USB, instalátor, první spuštění / live USB, installer, first run |
| `assets/logo/` | logo (vektory) / logo (vectors) |
| `prirucka.html` | příručka grafického jazyka (cs / en) / visual language handbook (cs / en) |
| `index.html` | rozcestník a rozhodnutí ke stylu / index and style decisions |
| `design-system.html` | barvy, kontrast, písmo, mřížka, ikony, pohyb, komponenty / colors, contrast, type, grid, icons, motion, components |
| `okna.html` | okna, odznak důvěry, režimy, stavy / windows, trust badge, modes, states |
| `plocha.html` | lišta, přepínání oken, oznámení, systémové dialogy, správce úloh / bar, window switcher, notifications, system dialogs, task manager |
| `terminal.html` | terminál / terminal |
| `soubory.html` | průzkumník s verzemi a „zkusit bokem“ / file explorer with versions and "try aside" |
| `nastaveni.html` | nastavení, profily zabezpečení / settings, security profiles |
| `prihlaseni.html` | přihlášení / sign-in |
| `varianty-stylu.html` | archiv: tři varianty, vybrána C / archive: three variants, C chosen |
| `tokens.toml` | tokeny jako data (zdroj pravdy) / tokens as data (source of truth) |
| `tools/build-tokens.mjs` | generátor CSS a kontrola kontrastu WCAG / CSS generator and WCAG contrast check |
| `assets/` | `tokens.css`, `tokens.js` (generované / generated), `mo.css`, `mo.js` (komponenty, ikony / components, icons) |

Spuštění / Running: otevři `index.html` v prohlížeči, nebo spusť
jednoduchý server (open `index.html` in a browser, or run a simple server):

```bash
python -m http.server 8766 --directory design
```

Po změně `tokens.toml` / after editing `tokens.toml`:

```bash
node design/tools/build-tokens.mjs
```

Prototypy nejsou kód systému; písma načítají z Google Fonts jen pro náhled.
The prototypes are not system code; they load fonts from Google Fonts for
preview only.
