# libs/mowin

Rozhraní C pro víc oken kompozitoru v jednom procesu (`mowin_*`, hlavička `include/mowin.h`, v sysrootu C `include/mowin.h`): okna s vlastníkem, popupy (vrstvy `POPOVER`), snímky, události, kurzor, schránka, obnova po pádu kompozitoru. Navíc zvuk `moaudio_*` (`include/moaudio.h`): společné jádro pro ovladač zvuku Wine (`winemoaudio.drv`) i `mosdl_audio_*`. Používá ho grafický ovladač Wine (`winemo.drv`, záplata ve `third_party/wine/`). Je součástí `libc.a` (`libs/cstd-libc`) jako `libs/mosdl`. Popis: `docs/cs/windows-vrstva.md` (kapitola W3).

## Klíčové soubory

- `src/event.rs` - překlad `InputRecord` na `mowin_event` / `mosdl_event` (stejné rozložení, pole `window` = v `mosdl.h` `reserved`) a kopie snímku; **společný modul** s `libs/mosdl` (ten ho jen re-exportuje).
- `src/os/restore.rs` - obnova po pádu kompozitoru: záznam (`guiproto::replay`, plní `send_with`), nová relace přes `osrt::gui::reconnect_raw`, znovu stav (buffery = kopie VMO jen pro čtení), commit ukázaných oken, `EXPOSE`.
- `src/os/audio.rs` - zvuk `moaudio_*` a Rust API (`mowin::audio`, volá ho `libs/mosdl` a `user/gui-probe`): relace z `osrt::audio::take_client_raw`, háček pro relace potomků, streamy `audioproto::conn`.
- `src/os/mod.rs` - spojení s kompozitorem (relace z `osrt::gui::take_session_raw`, jinak kanál okna aplikace), okna, popupy, `pump`/`drain`, čekání na událost mimo zámek; `surface.rs` dvojice bufferů vrstvy; `clip.rs` schránka; `pointer.rs` kurzor.
- `include/mowin.h` - hlavička; konstanty a rozložení hlídá test `header_constants_match`, `layout_matches_header`.

## Invarianty

- Jedno spojení na proces: všechna okna jdou přes něj (vlastník okna musí být okno téhož spojení, `WINDOW_OWNER`).
- Stav za zámkem, volat smí kterékoli vlákno (Wine kreslí z vláken aplikace). `mowin_next_event` čeká **mimo** zámek (duplikát kanálu, událost probuzení, událost `QUEUED`); kdo zařadí událost do fronty, signalizuje `QUEUED`.
- Druhá kopie `libos` (vedle kopie ve std): `set_self_process` dřív, než se cokoli mapuje.
- Zprávy, které kompozitor odkládá do `COMMIT` (`LAYER_CREATE`, titulek), jdou až s prvním snímkem okna či popupu (`first_frame`): skryté okno Wine nesmí nechat v kompozitoru rozpracovanou transakci (blokovala by ustálení). Popup okna, které ještě nemá snímek, = `BAD_STATE` (ovladač snímek zopakuje).
- Schránka posílá číslo posledního fyzického stisku doručeného oknům procesu (pravidlo kompozitoru beze změny); text nikdy do logu.
- Každá zpráva kompozitoru jde přes `send_with` (zaznamená ji pro obnovu); `HELLO` a zprávy obnovy jdou přímo (`raw_send`). Ztráta spojení v `send_with`, `drain` a čekání = `recover()`; bez lístku role Gui `SERVICE_UNAVAILABLE`.
- Zvuk: zápis a čekání na místo bez zámku `AUDIO` (na klonu streamu); `moaudio_put` nečeká ani nevolá službu (ovladač Wine píše pod svým zámkem, `moaudio_start` volá mimo něj).
- Kód je náš (licence jako ostatní `libs`); žádné hlavičky ani kód z Wine. Hranice LGPL je ve složce Wine.

## Testy

- `cargo test -p mowin` (překlad událostí, kopie snímku, shoda s hlavičkou). QEMU: sada `wine` (`--wine`, release; obnova `wintest obnova`, tón `wintest ton`), zvuk a role Audio sada `sdl`.

## Pasti

- Obnova pošle commit každého ukázaného okna v pořadí id (vlastník je vytvořený dřív než vlastněný dialog, kompozitor pak dialogu dá fokus); okno, kterému Wine vlastníka nastavil až později, by fokus nedostalo.
- Popupy (vrstvy `POPOVER`) kreslí kompozitor jen u okna s fokusem: popup vlastníka, kterému fokus vzal dialog, není vidět (test obnovy proto dává popup dialogu).
- Schránku nabízí jen proces, který ji ve Wine změnil (`winemo.drv`, `WM_CLIPBOARDUPDATE`): okna schránky ostatních procesů (explorer) by soupeřila o `OpenClipboard` a kompozitor by je odmítl.
- Snímek se kopíruje celý (dvojité buffery, výřez by nechal starší obsah).
