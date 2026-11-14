# user/gui-files

Aplikace Soubory: jen backend `OsBackend` nad relací FS uživatele, kanálem k `files-copy` a službě „otevři cestu“; model a rozhraní jsou v `libs/filesview`. Start: okno, lístek, relace FS, kanál `files-copy`, kanál služby; domov v bloku parametrů (cwd). Manifest: akce `files.*`, `[open]` (složky, reveal), `system = true`, více oken. Popis: `docs/cs/spravce-souboru.md`.

## Invarianty

- Aplikace nedostane capability k ovladači; jen relaci FS uživatele (rozhodují oprávnění).
- Smyčka okna `ui::client::run_channels`: kanál 0 = `files-copy`, 1 = odpovědi služby „otevři cestu“ (trvalý kanál, jeho konec jde s každou žádostí). Pořadí kanálů je pevné; chybějící se nahradí prázdným.
- Schránka (`gui-files: schranka: ...` v logu) jen na fyzický stisk: `Effect::Copy/Paste` nese pořadové číslo stisku (gesto, `clipproto`), ne čas; funguje i v QEMU bez akcelerace.
- Pomalé zpracování (nad 50 ms) se loguje (`gui-files: pomale ...`); výpis nad 200 ms také.

## Testy

- Host: `cargo test -p filesview`. QEMU: `cargo xtask test --only gui` (`user/init/src/tests/gui_files.rs`).
