# libs/cstd

Knihovna C pro aplikace se std (cíl `*-unknown-magnumopus`): vrstva POSIX nad `std` a `osrt` - `errno`, čísla fd (`unistd.h`, `fcntl.h`, `sys/stat.h`, `dirent.h`), proudy `FILE` nad fd, `exit`/`atexit`/`getenv`, `assert`. Základ (řetězce, `malloc`, `snprintf`, SSP) je v `libs/cstd-core`. Popis: `docs/cs/cstd.md`.

## Klíčové soubory

- `src/errno.rs` - čísla POSIX, `errno` v TLS (`__errno_location`), převod `io::Error` -> `errno`, `strerror`.
- `src/sys/` - systémová vrstva s chybou = `errno`: `os.rs` nad `std::os::magnumopus::runtime` (= crate `osrt`, tabulka `osrt::fd`), `host.rs` náhrada nad `std::fs` jen pro host testy (stejná pravidla čísel).
- `src/unistd.rs` (`open`, `read`, `dup`, `pipe`, `fcntl`, `fork` = `ENOSYS` ...), `src/stat.rs`, `src/dirent.rs`, `src/flock.rs` (zámky `fcntl`/`flock`/`lockf` přes `sys::file_lock` -> zpráva fsproto `POSIX_LOCK`; čekání opakovaným zkoušením).
- `src/pthread/` - `mod.rs` vlákna nad `sys::spawn` (osrt), `pthread_exit`/zrušení syscallem (`sys::exit_thread`, `panic=abort`); `sched.h` (jen `SCHED_OTHER`, test `sched_policy_rules`) v `mod.rs`; `sync.rs` mutex, podmínka, rwlock, bariéra, once, spin nad futexem; `key.rs` klíče a `pthread_cleanup_*`.
- `src/fenv.rs` - `fenv.h` (x86_64 x87 + MXCSR, aarch64 FPCR/FPSR; hodnoty a `fenv_t` jako glibc, test `fenv_header_matches`); pasti výjimek nikdy nezapínat.
- `src/time.rs` - hodiny (`sys::monotonic_ns`, `sys::realtime`), spánek, kalendář (`split`, `civil_secs` nad `abi::time`), pásmo z `TZ` (`libs/tz`), `strftime` (`format_tm`).
- `src/math.rs` - `math.h` nad crate `libm`, `strtod`/`strtof`/`atof`; `c/math_ld.c` - funkce `*l` a `strtold` v C (ABI `long double`), překládá `build.rs` jen pro cíl magnumopus.
- `src/socket.rs` - sockety BSD, adresy, `getaddrinfo`; `src/poll.rs` - `poll`, `select`, `epoll` (množina = objekt `User` v tabulce fd). Logika socketů a čekání je v `osrt::sock` a `osrt::poll`.
- `src/mman.rs` - `mmap` a spol. nad `sys::vm_*` (`osrt::vm` = obálka `libos::vm`); `check` = čistá kontrola argumentů.
- `src/signal.rs` - signály (6A): synchronní z obsluhy výjimek (`osrt::exception`), `raise`/`kill` hned, časovač ve vlákně signálů; `src/setjmp.rs` - nahé funkce `setjmp`/`longjmp` pro obě architektury (`jmp_buf` 320 B, maska od offsetu 176).
- `src/misc.rs` - další funkce pro porty (řetězce, `qsort`, `rand`, `pwd.h`, `ioctl`, `locale.h`, `langinfo.h` (čísla položek = modul `nl`, test `langinfo_header_matches`), `uname`, `getrusage`); `src/scanf.rs` - rodina `scanf` (motor `scan` nad `Input`/`Sink`).
- `src/dlfcn.rs` - `dlopen` a spol. nad tabulkou zavaděče (`osrt::dl`); statický program: `dl_iterate_phdr` popíše jediný modul ze symbolů `user/user-std.ld` (`static_phdrs`, pro libunwind); `crt/crt1.c` - start programu PIE nad `libc.so` (`_start` -> `__libc_start`) a skrytý `__dso_handle`; `crt/crtdso.c` - `__dso_handle` každé `.so` (`csysroot::ldflags_so`).
- `src/cxxsup.rs` - funkce pro běh C++ ([docs/cs/cpp.md](../../docs/cs/cpp.md)): `getentropy`, `aligned_alloc`, `posix_memalign`, `timespec_get`, `lldiv`, `__cxa_atexit`/`__cxa_finalize` (jeden seznam s `atexit`), `__cxa_thread_atexit_impl`, `__dso_handle` statického programu.
- `src/fsat.rs` - `openat`/`unlinkat`/`mkdirat`/`fstatat`/`fchmodat` relativně k adresáři (cesta z `osrt::fs::File::path`), `realpath`, `pathconf`, `sendfile`; `src/statvfs.rs` - `statvfs` nad `fsproto` INFO.
- `libs/cstd-so` - `libc.so` (cdylib nad sysrootem std jako PIC, `stdsys::build_libc_so_locked`).
- `libs/cstd-libc` - `libc.a` (staticlib cstd + std + osrt + rozhraní SDL `libs/mosdl`) pro sysroot C (`xtask/src/csysroot.rs`), porty v C `xtask/src/cports/`. `environ` je v `osrt` (`env.rs`), ne v cstd.
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
- Nic v cstd nesmí spoléhat na rozbalení zásobníku (`catch_unwind`): cíl má `panic=abort`. Výjimka C++ přes funkci v Rustu (zpětné volání z `qsort`) = `std::terminate`.
- `atexit` i `__cxa_atexit` jdou do jednoho seznamu v `cxxsup.rs`, spouští ho jedna funkce v `osrt::at_exit` registrovaná po vyprázdnění stdio; nejdřív destruktory `thread_local` volajícího vlákna. Konec vlákna z `pthread_create`: `run_thread_dtors` před destruktory klíčů.
- `malloc` (cstd-core) má v hlavičce i zarovnání: `aligned_alloc` a `posix_memalign` uvolňuje obyčejný `free`.

## Testy

- Host: `cargo test -p cstd` (režimy `fopen`, errno, soubory, pravidla čísel fd nad `sys/host.rs`).
- QEMU: `cargo xtask test --only cstd` (program `user/ctest`, v C); sockety `--only "cstd sit"` (`ctest net`, pomocné servery hostitele jako `std sit`).

## Pasti

- Program v C je crate `#![no_main]`; `main` z C volá `osrt`. Bez `extern crate cstd;` se knihovna nepřilinkuje.
- `wchar.h` má zatím jen typy: nepřidávej deklarace `wcs*` bez implementace (CMake/configure portů by je našly a link by selhal).
- `include/limits.h` (`PATH_MAX`, `NAME_MAX`) vzniká i ve větvi cstd-wchar (`MB_LEN_MAX`): při slučování spojit obsah.
- `%Lf` v `printf` vypíše `?` (`long double` nejde číst z `VaList`); `*l` funkce mají přesnost `double`.
- `std::os::magnumopus::runtime` potřebuje `feature(rustc_private)` (crate ze sysrootu; atribut `unstable` na re-exportu se nevynucuje); jen pod `cfg(target_os = "magnumopus")`.
