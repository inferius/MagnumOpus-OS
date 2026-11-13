# libs/openwith

Asociace typů souborů s aplikacemi: sekce `[open]` v manifestu (akce s povinným parametrem `path` typu `path`, `extensions`, `folders`, `reveal`), výběr aplikace pro cestu (`Registry::resolve`) a textové zprávy služby „otevři cestu“ (`msg`). Službu obsluhuje init (`user/init/src/open.rs`). Popis: `docs/cs/spravce-souboru.md#otevírání-souborů-a-asociace-typů`.

## Invarianty

- Vadné `[open]` (nedeklarovaná akce, parametr jiného typu) se ignoruje a hlásí, nikdy nepovolí volání bez deklarace.
- Pořadí: složka -> `folders`; soubor -> první aplikace s příponou; jinak `reveal`.

## Testy

- `cargo test -p openwith`; v QEMU část „otevri cestu“ testu správce souborů (sada `gui`).
