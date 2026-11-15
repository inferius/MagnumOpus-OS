# libs/mosys

Objekty jádra pro programy v C: hlavička `include/magnumopus/sys.h` (v sysrootu C `include/magnumopus/sys.h`), symboly `mo_*` (kanály, VMO, ploty, události, čekání na víc objektů, bootstrap kanál, handly startu). Součást `libc.a` (`libs/cstd-libc`) i `libc.so` (`libs/cstd-so`) jako `mosdl`: jedna kopie std v procesu. Používá ho proces dekodéru médií `user/ffdec`. Popis: `docs/cs/multimedia.md`.

## Invarianty

- Konstanty hlavičky (kódy chyb, signály, termíny) = hodnoty jádra (`abi`): host test `src/tests.rs`.
- Funkce vrací 0 / index nebo záporný kód chyby jádra; nikdy nepanikaří na vstupu z C (délky 0 s NULL jsou platné).
- Druhá kopie `libos` (vedle std) nemá vlastní start: před mapováním `ensure_process` (handle procesu z osrt).
- Bootstrap kanál patří běhovému prostředí (`mo_bootstrap` ho nepředává do vlastnictví).

## Testy

- `cargo test -p mosys` (hlavička proti jádru). QEMU přes `ffdec`: `cargo xtask test --only "media ffmpeg"`.
