# libs/cstd

Knihovna C pro aplikace se std (cíl `*-unknown-magnumopus`): vrstva POSIX nad `std` - `errno`, proudy `FILE` (stdin/stdout/stderr, soubory), `exit`/`atexit`/`getenv`, `assert`. Základ (řetězce, `malloc`, `snprintf`, SSP) je v `libs/cstd-core`. Popis: `docs/cs/cstd.md`.

## Klíčové soubory

- `src/errno.rs` - čísla POSIX, `errno` v TLS (`__errno_location`), převod `io::Error` -> `errno`, `strerror`.
- `src/stdio.rs` - `FILE` (zámek, buffery: stdout po řádcích, stderr bez, soubory 4 KiB), seznam otevřených proudů, vyprázdnění při konci (`ensure_exit_hook`).
- `src/stdlib.rs` - `exit`, `_Exit`, `atexit`, `getenv` (cache řetězců C), `abort`, `__cstd_assert_fail`.
- `include/` - hlavičky POSIX; `stdio.h`, `stdlib.h`, `string.h` doplňují cstd-core přes `#include_next`.
- `build.rs` - `links = "cstd"`: programům v C předá adresáře hlaviček (`DEP_CSTD_INCLUDE`, oddělené `;`).

## Invarianty

- Symboly C jen s `#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]`: na hostiteli by se tloukly se systémovou libc.
- Závislost na cstd-core vždy `default-features = false` (bez `service-io`; stdio je tady).
- Vyprázdnění proudů se registruje do `at_exit` dřív než první funkce `atexit` (běží v obráceném pořadí, tedy po nich).
- Čísla `E*` v `src/errno.rs` a `include/errno.h` musí sedět (test `errno_header_matches`).

## Testy

- Host: `cargo test -p cstd` (režimy `fopen`, errno, soubory přes API C).
- QEMU: `cargo xtask test --only cstd` (program `user/ctest`, v C).

## Pasti

- Program v C je crate `#![no_main]`; `main` z C volá `osrt`. Bez `extern crate cstd;` se knihovna nepřilinkuje.
- `printf` zatím neumí plovoucí čárku (`%f` = `?`).
