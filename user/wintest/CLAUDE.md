# user/wintest

Konzolový testovací program Windows (PE) pro Windows vrstvu (W1): `printf`, argumenty, prostředí, soubory v textovém režimu, vlákno, kód konce 42 (jinak 100 + bity chyb). Režim `procesy` (W2): spustí sám sebe jako `dite`, pojmenované události, `Read`/`WriteProcessMemory`, kód dítěte 77. Není to crate: překládá ho `tools/wsl/build_wine.sh` nástroji sestavení Wine (`clang -target x86_64-windows`, link `winegcc` s `winecrt0`, `ucrtbase`, `kernel32`) do `target/wine/<arch>/tests`, `xtask --wine` ho dá na datový disk do `/apps/org.magnumopus.wintest`. Test: sada `wine` (`user/init/src/tests/wine.rs`), `cargo xtask test --release --only wine --wine`. Popis: `docs/cs/windows-vrstva.md`.

## Pasti

- Kód v C bez diakritiky a jen ASCII; hlavičky jsou Wine (`include/msvcrt`), ne cstd.
- Textový režim CRT: `\n` je na disku `\r\n` (kontrola velikosti souboru s tím počítá).
