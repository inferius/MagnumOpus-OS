# libs/dynload

Logika zavaděče `ld.so` bez systému (testy na hostiteli): rozložení statického TLS (`tls_layout`, varianta II x86_64 / I aarch64 jako `osrt::tls` a linker), hodnota relokace (`compute`), cesta knihovny v initfs (`initfs_path`), hledání knihoven z disku (`search`, DL-4c). Používá ji `user/ld`. Popis: `docs/cs/dynamicke-linkovani.md`.

## Invarianty

- Program je v rozložení TLS vždy první modul (jeho local-exec spočítal linker); změna vzorce = test proti posunům lld (`tests.rs`).
- `system_name`: jméno v `/system/lib` (`lib/` v initfs); cesty s `/` a `..` se odmítají.
- `search` (DL-4c): kde hledat knihovnu - absolutní cesta (normalizovaná) nebo `DT_RUNPATH` s `$ORIGIN`, pak `/system/lib`; relativní cesta s lomítkem a neznámé proměnné (`$LIB`) se odmítají / přeskočí. Testy v `search_tests.rs`.

## Testy

- `cargo test -p dynload`.
