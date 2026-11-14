# user/libd

Služba knihoven: zapečetěné šablony segmentů sdílených knihoven z `/system/lib` (initfs `lib/<jméno>`) pro zavaděč `ld.so` a `spawn`; kód sdílí všechny procesy, data jako šablona pro CoW klon. Protokol `libs/libdproto`. Spouští ji init hned po času (`user/init/src/libd.rs`, restart pod supervisorem). Popis: `docs/cs/dynamicke-linkovani.md`.

## Invarianty

- Vstup od klientů je nedůvěryhodný: zprávy a jména jen přes `libdproto` (`valid_name`), soubor/segmenty/rozsah s mezemi, ELF přes `libs/elf`; vadná zpráva = `INVALID_ARGS`, nikdy panika.
- Klient nikdy nedostane zápis do šablony: kód `READ | MAP | EXECUTE` (zapečetěný), data `READ | MAP` (klient klonuje).
- Meze spojení: `MAX_CHILD_CONNECTIONS` živých z jednoho spojení, `MAX_CONNECTIONS` celkem; uzavřené spojení vrátí počítadlo rodiči.

## Testy

- Host: `cargo test -p libdproto`. QEMU: `cargo xtask test --only procesy` (`libd sdili sablony ...`, všechny testy `dynamicke linkovani`).

## Pasti

- Po restartu libd mají starší procesy zavřené spojení (obnova lístkem zatím není).
