# libs/fileops

Kopírování a přesun souborů jako úloha po krocích pro správce souborů: stroj `job::Job` nad rozhraním `Vfs` (v OS `FsClient` v `user/files-copy`, v testech `mem::MemFs`), deník úlohy pro obnovu (`Job::encode`/`decode`) a protokol aplikace <-> `files-copy` (`proto`). Popis: `docs/cs/spravce-souboru.md#kopírování-a-přesun`.

## Invarianty

- Cíl nikdy není napůl: položka se skládá v dočasném jménu `.<jméno>.kopie-<úloha>` a přejmenuje se až hotová. Správce dočasná jména skrývá (`Job::is_temp_name`).
- Přepsání nemaže: původní cíl jde do koše. Přesun v jednom datasetu = přejmenování, mezi datasety kopie + zdroj do koše.
- Jeden `step` = omezená práce (jedna okamžitá kopie, jedno přejmenování, nebo blok `CHUNK`); mezi kroky se zpracují zprávy.
- Uložený stav a zprávy jsou nedůvěryhodný vstup: chyba = `INVALID_ARGS`, nikdy panika; nová verze stavu = nové `STATE_VERSION`.
- Kopie čtením a zápisem jen pro `NOT_SUPPORTED` z `clone_path`; jiná chyba je chyba položky.

## Testy

- `cargo test -p fileops`. Klíčový test `resume_after_interruption_*`: přerušení po každém kroku + obnova = stejný výsledek. V QEMU přes `files-copy` (sada `gui`).

## Pasti

- `MemFs` je jen pro testy (i pro `filesview`); nepoužívat jako vzor výkonu.
