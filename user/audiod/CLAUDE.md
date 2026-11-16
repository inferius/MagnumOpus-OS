# user/audiod

Zvuková služba: mix streamů klientů, převod formátů a frekvencí, rozvod nahrávání, hlavní hlasitost, výběr zvukové karty. Protokol `libs/audioproto` (`client`, `device`, `ring`, `session`), logika mixu `libs/audiomix`. Popis: `docs/cs/zvuk.md`.

## Klíčové soubory

- `src/main.rs` (start), `service.rs` (smyčka, požadavky, mix), `device.rs` (zařízení, takt), `input.rs` (nahrávání), `streams.rs` (streamy klientů), `perm.rs` (relace aplikací s právy podle receptu, dotaz na povolení nahrávání u `accountd`, seznam nahrávajících pro lištu).

## Invarianty

- Relace aplikace (`Client::app`) nesmí `MASTER_VOLUME`, `MUTE`, `SELECT`, `CONNECT` (`session::app_allowed`); systémové relace (prázdný recept) ano.
- Nahrávání aplikace jen s `Capture::Ask` a po rozhodnutí služby účtů; bez kanálu `permissions` zamítnout. Odpověď na `OPEN` vstupu čeká (`pending`), klienti se drží podle stálého `Client::id` (indexy se při `reap` mění).
- Limity relací (`MAX_IDENTITY_SESSIONS`) a streamů (`MAX_IDENTITY_STREAMS`) aplikací se počítají na identitu (`app_id`), ne na relaci (role `Audio`). `revoke\0<app_id>` od initu zavře všechny relace identity (odpověď = počet).
- Pořadí položek čekání ve smyčce: klienti, zařízení, záznam, pak kanál povolení (indexy výše jsou pevné).

## Testy

- Host: `cargo test -p audioproto` (`session_tests.rs`), `-p audiomix`. QEMU: `cargo xtask test --only zvuk`, nahrávání s povolením `--only sdl`.

## Pasti

- Privilegované zprávy initu jsou text na bootstrap kanálu (`device`, `crash-driver`, `perm`, `app\0` + recept, `revoke\0` + identita); nový druh = init (`user/init/src/audio.rs`) i docs.
