# user/libd

Služba knihoven: zapečetěné šablony segmentů sdílených knihoven z `/system/lib` (initfs `lib/<jméno>`) a z disku (obsah od zavaděče, `OpenImage`) pro zavaděč `ld.so` a `spawn`; kód sdílí všechny procesy, data jako šablona pro CoW klon. Protokol `libs/libdproto`. Spouští ji init hned po času; je to služba supervisoru s adaptérem a lístky (`user/init/src/libd.rs`, `LibdService`). Popis: `docs/cs/dynamicke-linkovani.md`.

## Invarianty

- Vstup od klientů je nedůvěryhodný: zprávy a jména jen přes `libdproto` (`valid_name`), soubor/segmenty/rozsah s mezemi, ELF přes `libs/elf`; vadná zpráva = `INVALID_ARGS`, nikdy panika.
- Klient nikdy nedostane zápis do šablony: kód `READ | MAP | EXECUTE` (zapečetěný), data `READ | MAP` (klient klonuje).
- Relace tvoří strom: kořen = bootstrap kanál (adaptér v initu), `Connect` z kterékoli relace **bez odpovědi** (rodič nesmí čekat). Meze: `MAX_CHILD_CONNECTIONS` živých z jedné relace (na proces; kořen bez meze), `MAX_CONNECTIONS` celkem; nad mez konec zavřít (klient to pozná přes lístek: `RECONNECT` = `PEER_CLOSED`). Uzavřená relace vrátí počítadlo rodiči.
- Všechny ostatní žádosti jsou idempotentní (klient je po pádu zopakuje).
- libd **nikdy nečte disk a nemá relaci FS** (DL-4c): knihovnu z disku dostane jako VMO od zavaděče (`OpenImage`, soubor přečtený relací FS procesu = s právy aplikace), hned si ji zkopíruje a dál pracuje jen s kopií (hash i šablona). Jméno z `OpenImage` nikdy nejde do `names` (hledání podle jména = jen initfs). Šablony z disku jsou v `disk` s mezí `MAX_DISK_TEMPLATES`/`MAX_DISK_BYTES` a zahazují se podle posledního použití; šablona použitá i z initfs se z `disk` vyřadí a nezahazuje se.

## Testy

- Host: `cargo test -p libdproto`. QEMU: `cargo xtask test --only procesy` (`libd sdili sablony ...`, `obnova po padu libd ...`, `pad libd pred prevzetim listku`, všechny testy `dynamicke linkovani`).

## Pasti

- Nová instance po pádu staví šablony znovu (procesy mapují staré VMO dál; `spawn` drží šablonu `ld.so` v mezipaměti).
