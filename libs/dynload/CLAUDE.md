# libs/dynload

Logika zavaděče `ld.so` bez systému (testy na hostiteli): rozložení statického TLS (`tls_layout`, varianta II x86_64 / I aarch64 jako `osrt::tls` a linker), hodnota relokace (`compute`), cesta knihovny v initfs (`initfs_path`). Používá ji `user/ld`. Popis: `docs/cs/dynamicke-linkovani.md`.

## Invarianty

- Program je v rozložení TLS vždy první modul (jeho local-exec spočítal linker); změna vzorce = test proti posunům lld (`tests.rs`).
- Knihovny jen z `/system/lib` (`lib/` v initfs); cesty s `/` a `..` se odmítají.

## Testy

- `cargo test -p dynload`.
