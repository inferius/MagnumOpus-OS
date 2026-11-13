# user/files-copy

Pomocný proces správce souborů: kopírování a přesun jako úlohy relace (stroj `libs/fileops`), producent `taskd`, stav úloh v `<domov>/apps/org.magnumopus.files/ulohy`. Init ho spouští při otevření relace (`user/init/src/files.rs`): handly relace FS, řídicí kanál (`client` + kanál okna), volitelně producent taskd; domov v bloku parametrů. Popis: `docs/cs/spravce-souboru.md#kopírování-a-přesun`.

## Invarianty

- Úloha patří relaci, ne oknu: přežije zavření okna; konflikt může rozhodnout kterékoli okno relace.
- Stav se ukládá po položce a nejpozději po `SAVE_EVERY`; zápis přes dočasný soubor a `RENAME_REPLACE`. `Vfs::sync` je prázdné, protože FS commituje atomicky v pořadí (stav nepředběhne data).
- Složku stavu tvoří jen pod existujícím domovem (relace uživatele nesmí tvořit v `/` ani `/home`).
- Mezi kroky nejvýš `SLICE_NS` práce, pak zprávy.

## Testy

- QEMU: `cargo xtask test --only gui` (`files-copy: uloha s konfliktem prezije restart procesu`, test správce souborů).
