# libs/ui

Nativní knihovna UI (milník 6c): jedna pro všechny aplikace. Aplikace popíše rozhraní deklarativně (`node`), knihovna ho rozvrhne (`layout`), vykreslí do bufferu (`draw`), převede vstup a akce přístupnosti na zprávy aplikace (`event`) a odvodí strom vrstev a strom přístupnosti. `Ui` (jádro) nezávisí na OS. Běhové prostředí pro aplikace v OS (kanál ke kompozitoru, buffery ve VMO, smyčka) je v `src/client/` za feature `os`. Popis: `docs/cs/gui.md#nativní-knihovna-ui-libsui`.

## Invarianty

- Aplikace nic z toho nepíše ručně: strom přístupnosti a vrstev se odvozuje, ne duplikuje.
- Vzhled jen z tokenů (`libs/theme` / `look.rs`), žádná pevná čísla vzhledu v prvcích.
- Jádro `Ui` zůstává bez závislosti na `libos`; všechno OS-specifické patří do `client/` (feature `os`).
- Knihovna nenabízí aplikacím barvy `trust_*` (odznak důvěry, systémové dialogy) ani `ai_*` (jen agent AI); patří jen systému.
- Protokol s kompozitorem je v `libs/guiproto`; změny drž kompatibilní s `libs/comp`.

## Testy

- `cargo test -p ui` (`src/tests/`: `mod.rs` zakladni prvky, `keys.rs` klice, stav, heslo, `text.rs` editor, `field.rs` textova pole a schranka, `table.rs` tabulka, `controls.rs` prepinac, volba, posuvnik, segmenty, zalozky, graf podilu, `popup.rs` vyber, nabidky, bublina, `shots.rs` snimky vzhledu proti `tests/shots/*.png` (prepis jen `cargo xtask test --update-shots`), `context.rs` kontext okna, `spatial.rs` prostorova navigace fokusu, `suggest.rs` seznam navrhu u pole); `client/` jen s `--features os` pro cíl OS. QEMU: `cargo xtask test --only gui`.

## Pasti

- Aplikace UI vykresluje do vlastního bufferu; kompozitor mu nevěří (viz `libs/comp/CLAUDE.md`), ale i klient musí ošetřit nesmyslné rozměry od kompozitoru.
- `client/mod.rs` má ~730 řádků; při dalším růstu dělit (obnova po pádu je v `restore.rs`, schránka v `clip.rs`).
- Seznam návrhů u pole: jádro bez stromu `complete.rs` (`Completion`, používá ho i gui-term samostatně), napojení na pole `suggest.rs`. Klávesy, které seznam nevezme, musí jít do pole (`Outcome::Ignored`); heslo seznam nikdy nemá.
- Textová pole: editor `text.rs` (grafémy, slova, historie) je bez kreslení a sdílí ho i gui-term; geometrie, klávesy a kreslení pole jsou v `field.rs`. Hodnotu drží aplikace, pole ji po `rebuild` převezme (`sync_fields`).
