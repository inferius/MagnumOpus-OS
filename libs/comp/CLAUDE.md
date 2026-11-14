# libs/comp

Jádro kompozitoru `compd` bez závislosti na jádru OS (`no_std` + `alloc`, testy na hostiteli). Služba `user/compd` jen převádí kanály a objekty paměti na volání `Comp` a rozesílá zprávy z `Comp::take_out`. Obsah: `scene` (klienti, okna, buffery, vrstvy, transakce), `render` (skládání snímku: dekorace, odznak důvěry, lišta, kurzor), `style` (vzhled z tokenů), `input` (rozdělení vstupu, přesun a změna velikosti), `dialog` (systémové dialogy, bezpečná obrazovka), `greeter` (přihlášení a zámek), `frame` (takt, animace, skupiny transakcí), `gpu` (seznam operací pro GPU backend `libs/vkcomp`), `menu`, `flyout`, `clipboard` (schránka, protokol `libs/clipproto`), `pointer` (kurzor aplikace, celá obrazovka na žádost, zámek ukazatele s upozorněním), `sysui` (paleta a spouštěč nad `libs/ui`), `services` (akce aplikací, inspekce), `output` (změna režimu displeje). Popis: `docs/cs/gui.md`, vzhled `docs/cs/vzhled.md`.

## Invarianty

- Obsahu bufferu aplikace se nikdy nevěří: rozměry se kontrolují při vytvoření, čtení je omezené na VMO; aplikace smí buffer měnit i během čtení (nejhorší důsledek je roztržené její vlastní okno).
- Priorita vstupu před skládáním (smyčka v `user/compd`); na displeji je vždy nejvýš jeden snímek.
- Skládají se jen poškozené oblasti a nic se nekreslí, když se nic nemění (výkon softwarového skládání, `docs/cs/gui.md`). Pod neprůhledným jádrem okna (bez rohů) se plocha ani nižší okna nekreslí (`render::visible_parts`); test `culling_keeps_pixels` hlídá stejné pixely.
- Žádná čísla vzhledu v kódu: hodnoty jsou z tokenů `design/tokens.toml` přes `libs/theme`. Vzhled a svižnost jsou stejně důležité; efekty musí vejít do rozpočtu snímku.
- Systémové dialogy a odznak důvěry kreslí systém, aplikace je nemůže zakrýt ani zfalšovat.
- Celá obrazovka na žádost aplikace a zámek ukazatele jen po fyzickém stisku v okně s fokusem (`clip_take_gesture`, jednorázové gesto); zámek vždy zruší Super, ztráta fokusu, zavření okna, dialog a přihlášení (`pointer.rs`, testy `tests/pointer.rs`).
- Po pádu kompozitoru se stav obnoví (`docs/cs/gui.md#obnova-po-pádu-kompozitoru`, `user/compd/src/snapshot.rs`).

## Testy

- `cargo test -p comp` (velké testy v `src/tests.rs`, `src/tests/`, `output_tests.rs`). V QEMU sady `gui` (další: `displej`, `vulkan`): `cargo xtask test --only gui`.
- Testy ukazují jména, na která odkazují docs (např. `command_palette_search_run_and_recent`); nepřejmenovávej bez úpravy docs.

## Pasti

- Čas skládání po částech měří `prof.rs` (hodiny dodá `compd`, výpis v logu `compd: snimek cele obrazovky ...`). Novou drahou část kresli uvnitř změřené části nebo přidej vlastní `Part`.
- `src/tests.rs` má přes 1700 řádků (nad limitem ~1500 se nesmí dál růst): nové testy dávej do `src/tests/`.
- Poškození po transakci (`scene.rs`, `apply`): normálně celé okno; změna jen viditelnosti nebo polohy vrstvy s bufferem bez potomků v kořeni okna (kurzor pole, `small_layer_change`) poškodí jen její starý a nový obdélník (test `tests/caret.rs`). Nová vlastnost vrstvy, která mění pixely, nesmí do této výjimky spadnout.
- Protokol okna a displeje je v `libs/guiproto`; změna zasahuje i `libs/ui` a `user/compd`.
