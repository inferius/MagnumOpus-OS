# user/wintest

Konzolový testovací program Windows (PE) pro Windows vrstvu (W1): `printf`, argumenty, prostředí, soubory v textovém režimu, vlákno, kód konce 42 (jinak 100 + bity chyb). Režim `procesy` (W2): spustí sám sebe jako `dite`, pojmenované události, `Read`/`WriteProcessMemory`, kód dítěte 77. Režim `sekce` (W2): sdílená sekce nad souborem s dítětem `sekce-dite`, koherence s `ReadFile`/`WriteFile`. Režimy `gdi`, `klavesy` (vypisuje `WM_KEYDOWN` a `WM_CHAR`) a `ramy` (W3, `gui.c`): okno s obdélníky z `FillRect` (test čte pixely v kompozitoru) a dvě okna - běžné s rámem od systému a okno, které si neklientskou oblast zruší (`WM_NCCALCSIZE`); po nakreslení vypíší řádek `... nakresleno`, končí zavřením oken nebo za 120 s. Není to crate: překládá ho `tools/wsl/build_wine.sh` nástroji sestavení Wine (`clang -target x86_64-windows`, link `winegcc` s `winecrt0`, `ucrtbase`, `user32`, `gdi32`, `kernel32`) do `target/wine/<arch>/tests`, `xtask --wine` ho dá na datový disk do `/apps/org.magnumopus.wintest`. Test: sada `wine` (`user/init/src/tests/wine.rs`), `cargo xtask test --release --only wine --wine`. Popis: `docs/cs/windows-vrstva.md`.

## Pasti

- Kód v C bez diakritiky a jen ASCII; hlavičky jsou Wine (`include/msvcrt`), ne cstd.
- Textový režim CRT: `\n` je na disku `\r\n` (kontrola velikosti souboru s tím počítá).
