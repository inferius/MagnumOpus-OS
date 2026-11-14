# libs/cstd

Knihovna C pro aplikace se std (cíl `*-unknown-magnumopus`): vrstva POSIX nad `std` a `osrt` - `errno`, čísla fd (`unistd.h`, `fcntl.h`, `sys/stat.h`, `dirent.h`), proudy `FILE` nad fd, `exit`/`atexit`/`getenv`, `assert`. Základ (řetězce, `malloc`, `snprintf`, SSP) je v `libs/cstd-core`. Popis: `docs/cs/cstd.md`.

## Klíčové soubory

- `src/errno.rs` - čísla POSIX, `errno` v TLS (`__errno_location`), převod `io::Error` -> `errno`, `strerror`.
- `src/sys/` - systémová vrstva s chybou = `errno`: `os.rs` nad `std::os::magnumopus::runtime` (= crate `osrt`, tabulka `osrt::fd`), `host.rs` náhrada nad `std::fs` jen pro host testy (stejná pravidla čísel).
- `src/unistd.rs` (`open`, `read`, `dup`, `pipe`, `fcntl`, `fork` = `ENOSYS` ...), `src/stat.rs`, `src/dirent.rs`, `src/flock.rs` (zámky, zatím `ENOLCK`).
- `src/pthread/` - `mod.rs` vlákna nad `sys::spawn` (osrt), `pthread_exit`/zrušení syscallem (`sys::exit_thread`, `panic=abort`); `sync.rs` mutex, podmínka, rwlock, bariéra, once, spin nad futexem; `key.rs` klíče a `pthread_cleanup_*`.
- `src/time.rs` - hodiny (`sys::monotonic_ns`, `sys::realtime`), spánek, kalendář (`split`, `civil_secs` nad `abi::time`), pásmo z `TZ` (`libs/tz`), `strftime` (`format_tm`).
- `src/stdio.rs` - `FILE` nad číslem fd (zámek, buffery: stdout po řádcích, stderr bez, soubory 4 KiB), `fdopen`/`fileno`, `getline`, seznam otevřených proudů, vyprázdnění při konci (`ensure_exit_hook`).
- `src/stdlib.rs` - `exit`, `_Exit`, `atexit`, `getenv` (cache řetězců C), `abort`, `__cstd_assert_fail`.
- `include/` - hlavičky POSIX; `stdio.h`, `stdlib.h`, `string.h` doplňují cstd-core přes `#include_next`.
- `build.rs` - `links = "cstd"`: programům v C předá adresáře hlaviček (`DEP_CSTD_INCLUDE`, oddělené `;`).

## Invarianty

- Symboly C jen s `#[cfg_attr(target_os = "magnumopus", unsafe(no_mangle))]`: na hostiteli by se tloukly se systémovou libc.
- Závislost na cstd-core vždy `default-features = false` (bez `service-io`; stdio je tady).
- Vyprázdnění proudů se registruje do `at_exit` dřív než první funkce `atexit` (běží v obráceném pořadí, tedy po nich).
- Čísla `E*` v `src/errno.rs` a `include/errno.h` musí sedět (test `errno_header_matches`).
- Hodnoty `O_*`, `F_*`, `S_IF*` a rozložení `struct stat`/`dirent`/`flock` v `src/*.rs` a `include/` musí sedět (stejné na obou architekturách).
- Systémová volání jdou jen přes `sys` (ne přímo `std::fs`), aby C i Rust sdílely tabulku fd v osrt.
- Velikosti typů `pthread_*` v `src/pthread/` a `include/pthread.h` musí sedět (test `type_sizes_match_header`).
- Nic v cstd nesmí spoléhat na rozbalení zásobníku (`catch_unwind`): cíl má `panic=abort`.

## Testy

- Host: `cargo test -p cstd` (režimy `fopen`, errno, soubory, pravidla čísel fd nad `sys/host.rs`).
- QEMU: `cargo xtask test --only cstd` (program `user/ctest`, v C).

## Pasti

- Program v C je crate `#![no_main]`; `main` z C volá `osrt`. Bez `extern crate cstd;` se knihovna nepřilinkuje.
- `printf` zatím neumí plovoucí čárku (`%f` = `?`).
- `std::os::magnumopus::runtime` potřebuje `feature(magnumopus_runtime, rustc_private)` (crate ze sysrootu); jen pod `cfg(target_os = "magnumopus")`.
