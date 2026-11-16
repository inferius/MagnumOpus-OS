# libs/mosdl

Rozhraní C pro port SDL (`mosdl_*`, hlavička `include/mosdl.h`, v sysrootu C `include/mosdl.h`): okno kompozitoru, snímek, události, probuzení. Je součástí `libc.a` (`libs/cstd-libc`): dvě staticlib v Rustu v jednom programu by měly každá svou kopii std. Ovladač videa SDL v C, který ho volá: `third_party/c-ports/sdl3/overlay/src/video/magnumopus/`. Popis: `docs/cs/sdl.md`.

## Klíčové soubory

- `src/event.rs` - jen re-export společného modulu `libs/mowin/src/event.rs` (překlad `InputRecord`, kopie snímku; testy tam). `mosdl_event` má stejné rozložení jako `mowin_event` (`reserved` = `window`, vždy 0).
- `src/os/` - symboly C nad `ui::client::Window` (jen `target_os = "magnumopus"`): `mod.rs` okno, snímek, události (fronta `pending` pro události přijaté při čekání na schránku), `clip.rs` schránka (líné vložení s čekáním 1 s), `pointer.rs` kurzor, zámek ukazatele, celá obrazovka, `audio.rs` zvuk: jen obálky `mosdl_audio_*` nad společným jádrem `libs/mowin/src/os/audio.rs` (`moaudio`, relace z `osrt::audio`).
- `include/mosdl.h` - hlavička; konstanty a rozložení musí sedět s `event.rs` (test `header_constants_match`, `layout_matches_header`).

## Invarianty

- Kanál okna a lístek = první dva netagované handly startovní zprávy; bere je `osrt::start::take_handles` (jen jednou).
- Druhá kopie `libos` (běžná závislost vedle kopie ve std) nemá vlastní start: `os.rs` musí nastavit `libos::rt::set_self_process` dřív, než cokoli mapuje (buffery okna).
- `libos` i `ui` bez feature `rt` (vstupní bod, alokátor a panic handler dává std).
- Jedno okno na proces (`mosdl_window_open` podruhé = `NOT_SUPPORTED`).
- Událost probuzení je mimo zámek stavu (`mosdl_wake` nesmí čekat na vlákno, které čeká na události).

## Testy

- `cargo test -p mosdl` (překlad událostí, kopie snímku vč. nepřátelských rozměrů, shoda s hlavičkou).
- QEMU: `cargo xtask test --only sdl` (`user/init/src/tests/sdl.rs`: `testsprite` pixely, `checkkeys` umělý vstup).

## Pasti

- Program bez SDL nesmí nic z mosdl táhnout: symboly jen `#[no_mangle]` funkce, žádné konstruktory; ověření `llvm-nm` na `minigzip`.
- Snímek se kopíruje celý (buffery okna jsou dvojité, výřez by nechal starší obsah).
