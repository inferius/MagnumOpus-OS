# libs/mowin

Rozhraní C pro víc oken kompozitoru v jednom procesu (`mowin_*`, hlavička `include/mowin.h`, v sysrootu C `include/mowin.h`): okna s vlastníkem, popupy (vrstvy `POPOVER`), snímky, události, kurzor, schránka. Používá ho grafický ovladač Wine (`winemo.drv`, záplata ve `third_party/wine/`). Je součástí `libc.a` (`libs/cstd-libc`) jako `libs/mosdl`. Popis: `docs/cs/windows-vrstva.md` (kapitola W3).

## Klíčové soubory

- `src/event.rs` - překlad `InputRecord` na `mowin_event` / `mosdl_event` (stejné rozložení, pole `window` = v `mosdl.h` `reserved`) a kopie snímku; **společný modul** s `libs/mosdl` (ten ho jen re-exportuje).
- `src/os/mod.rs` - spojení s kompozitorem (relace z `osrt::gui::take_session_raw`, jinak kanál okna aplikace), okna, popupy, `pump`/`drain`, čekání na událost mimo zámek; `surface.rs` dvojice bufferů vrstvy; `clip.rs` schránka; `pointer.rs` kurzor.
- `include/mowin.h` - hlavička; konstanty a rozložení hlídá test `header_constants_match`, `layout_matches_header`.

## Invarianty

- Jedno spojení na proces: všechna okna jdou přes něj (vlastník okna musí být okno téhož spojení, `WINDOW_OWNER`).
- Stav za zámkem, volat smí kterékoli vlákno (Wine kreslí z vláken aplikace). `mowin_next_event` čeká **mimo** zámek (duplikát kanálu, událost probuzení, událost `QUEUED`); kdo zařadí událost do fronty, signalizuje `QUEUED`.
- Druhá kopie `libos` (vedle kopie ve std): `set_self_process` dřív, než se cokoli mapuje.
- Zprávy, které kompozitor odkládá do `COMMIT` (`LAYER_CREATE`, titulek), jdou až s prvním snímkem okna či popupu (`first_frame`): skryté okno Wine nesmí nechat v kompozitoru rozpracovanou transakci (blokovala by ustálení). Popup okna, které ještě nemá snímek, = `BAD_STATE` (ovladač snímek zopakuje).
- Schránka posílá číslo posledního fyzického stisku doručeného oknům procesu (pravidlo kompozitoru beze změny); text nikdy do logu.
- Kód je náš (licence jako ostatní `libs`); žádné hlavičky ani kód z Wine. Hranice LGPL je ve složce Wine.

## Testy

- `cargo test -p mowin` (překlad událostí, kopie snímku, shoda s hlavičkou). QEMU: sada `wine` (`--wine`, release).

## Pasti

- Obnova po pádu kompozitoru zatím není (ztráta spojení = `SERVICE_UNAVAILABLE`), na rozdíl od `ui::client`.
- Snímek se kopíruje celý (dvojité buffery, výřez by nechal starší obsah).
