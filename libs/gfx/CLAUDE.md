# libs/gfx

Softwarové kreslení pro kompozitor a knihovnu UI (`no_std` + `alloc`): `shape` (tvary), `text`, `icon`, `shadow`, `blur`, `backdrop`, `damage` (poškozené oblasti), `png`. Pixel je `u32` 0xAARRGGBB (v paměti bajty B, G, R, A; formát virtio-gpu `B8G8R8A8_UNORM`) s předem vynásobenou alfou. `Canvas` je pohled na cizí paměť (buffer aplikace, snímek kompozitoru); knihovna sama nic nealokuje. Popis: `docs/cs/gui.md#kreslení-a-buffery`, písmo `libs/font`.

## Invarianty

- Všechny operace ořezávají na plochu a na `clip`; žádná nesáhne mimo paměť ani při nesmyslných souřadnicích (vstup od aplikací je nedůvěryhodný).
- Premultiplied alfa všude; barvy z `libs/theme` mají stejný formát.
- Výkon je součást návrhu (efekty v rozpočtu snímku); měř, než přidáš náročný efekt.
- Žádné `f32` na pixel: dřív kvůli soft-float (do FPU-1 každá operace `f32` = volání funkce), dnes kvůli ceně i s FPU a kvůli stejnému výsledku na obou architekturách (otisky snímků). `f32` jen v přípravě (masky stínů, glyfy do mezipaměti, geometrie přechodu) nebo v rohových čtvercích (`shape::round_rect_coverage`); rozmazání a body plochy jsou celočíselné. Celočíselná náhrada má test proti původnímu výpočtu v `f32` (`tests.rs`: `*_matches_float`).

## Testy

- `cargo test -p gfx` (`src/tests.rs`). Měření výkonu uváděj podle `docs/cs/dokumentace.md` (revize, profil, prostředí, velikost úlohy).

## Pasti

- Ikony jsou data z prototypu: objekt `ICONS` v `design/assets/mo.js` se vkládá při překladu (blok mezi `ICONS-START`/`ICONS-END` generuje `design/tools/build-icons.mjs`). Ikonu měň tam, ne v `icon.rs`.
- Záměna pořadí kanálů (RGBA vs BGRA) se projeví jen jako špatné barvy na displeji, ne chybou překladu.
