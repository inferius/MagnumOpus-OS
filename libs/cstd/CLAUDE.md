# libs/cstd

Knihovna C pro aplikace se std (cíl `*-unknown-magnumopus`): vrstva POSIX nad `std` a `osrt` - `errno`, čísla fd (`unistd.h`, `fcntl.h`, `sys/stat.h`, `dirent.h`), proudy `FILE` nad fd, `exit`/`atexit`/`getenv`, `assert`. Základ (řetězce, `malloc`, `snprintf`, SSP) je v `libs/cstd-core`. Popis: `docs/cs/cstd.md`.

## Klíčové soubory

- `src/errno.rs` - čísla POSIX, `errno` v TLS (`__errno_location`), převod `io::Error` -> `errno`, `strerror`.
- `src/sys/` - systémová vrstva s chybou = `errno`: `os.rs` nad `std::os::magnumopus::runtime` (= crate `osrt`, tabulka `osrt::fd`), `host.rs` náhrada nad `std::fs` jen pro host testy (stejná pravidla čísel).
- `src/unistd.rs` (`open`, `read`, `dup`, `pipe`, `fcntl`, `fork` = `ENOSYS` ...), `src/stat.rs`, `src/dirent.rs`, `src/flock.rs` (zámky `fcntl`/`flock`/`lockf` přes `sys::file_lock` -> zpráva fsproto `POSIX_LOCK`; čekání opakovaným zkoušením).
- `src/pthread/` - `mod.rs` vlákna nad `sys::spawn` (osrt), `pthread_exit`/zrušení syscallem (`sys::exit_thread`, `panic=abort`); `sched.h` (jen `SCHED_OTHER`, test `sched_policy_rules`) v `mod.rs`; `sync.rs` mutex, podmínka, rwlock, bariéra, once, spin nad futexem; `key.rs` klíče a `pthread_cleanup_*`.
- `src/ucontext.rs` - `ucontext_t` pro obsluhy `SA_SIGINFO` (rozložení jako Linux, x86_64 i aarch64): `from_frame` z rámce výjimky, `to_frame` zápis změn po návratu (`signal.rs`, `on_exception`); test `ucontext_tests` hlídá offsety glibc a shodu s `include/sys/ucontext.h`.
- `src/fenv.rs` - `fenv.h` (x86_64 x87 + MXCSR, aarch64 FPCR/FPSR; hodnoty a `fenv_t` jako glibc, test `fenv_header_matches`); pasti výjimek nikdy nezapínat.
- `src/time.rs` - hodiny (`sys::monotonic_ns`, `sys::realtime`), spánek, kalendář (`split`, `civil_secs` nad `abi::time`), pásmo z `TZ` (`libs/tz`), `strftime` (`format_tm`).
- `src/math.rs` - `math.h` nad crate `libm`, `strtod`/`strtof`/`atof`; `c/math_ld.c` - funkce `*l` a `strtold` v C (ABI `long double`), překládá `build.rs` jen pro cíl magnumopus.
- `src/socket.rs` - sockety BSD, adresy, `getaddrinfo`; `src/poll.rs` - `poll`, `select`, `epoll` (množina = objekt `User` v tabulce fd). Logika socketů a čekání je v `osrt::sock` a `osrt::poll`.
- `src/mman.rs` - `mmap` a spol. nad `sys::vm_*` (`osrt::vm` = obálka `libos::vm`); `check` = čistá kontrola argumentů.
- `src/signal.rs` - signály (6A): synchronní z obsluhy výjimek (`osrt::exception`), `raise`/`kill` hned, časovač ve vlákně signálů; asynchronní signál z jiného procesu = přerušení vlákna (`THREAD_INTERRUPT`, druh výjimky `INTERRUPT`, číslo signálu v `Report::code`, `sync::interrupt`), blokovaný čeká v `PENDING`; `src/setjmp.rs` - nahé funkce `setjmp`/`longjmp` pro obě architektury (`jmp_buf` 320 B, maska od offsetu 176).
- `src/misc.rs` - další funkce pro porty (řetězce, `qsort`, `rand`, `pwd.h`, `ioctl`, `uname`, `getrusage`); `src/scanf.rs` - rodina `scanf` (motor `scan_units` nad znaky `u32` v bajtovém i širokém režimu, `Input`/`Sink`).
- `src/locale/` - `mod.rs` stav locale (globální maska `GLOBAL` + locale vlákna v TLS z `uselocale`), `setlocale`, `newlocale`/`duplocale`/`freelocale`, `__ctype_get_mb_cur_max`; `langinfo.rs` `localeconv`, `nl_langinfo` (čísla položek = modul `nl`, test `langinfo_header_matches`); `xl.rs` varianty `*_l`.
- `src/wchar/` - `conv.rs` převody (`mbrtowc`, `mbsnrtowcs`, ...), `wcs.rs` řetězce, `num.rs` `wcstol`/`wcstod`/`wcsftime`, `wctype.rs` třídy nad `libs/uniprop`, `wio.rs` široké proudy (`FILE` má orientaci a stav, `stdio::Locked`), `wprintf.rs`, `wscanf.rs`; `c/math_ld.c` `wcstold`, `strtold_l`.
- `src/spawn/` - `mod.rs` `posix_spawn(p)`, akce souborů (`plan`: jen fd 0-2, rozhodnutí 4A), atributy, hledání programu (`PATH`, jinak holý název pro launchd); `wait.rs` tabulka potomků (`pid_t` z cstd), `waitpid`/`wait4`/`waitid`, stav z kódu konce (`status_of`), `kill` na potomka (`kill_child`, jen ukončení). Systémová část `sys/os_proc.rs` nad `osrt::process`; host má náhradní potomky (`exit:<kód>`, `sleep:<ms>:<kód>`) v `sys/host.rs`.
- `src/semaphore.rs` (nepojmenované nad futexem, `sem_open` = `ENOSYS`), `src/pthread/attr.rs` (atributy vláken, vlastní zásobník přes `sys::spawn_on`), `src/statfs.rs` (`statfs`/`fstatfs` nad `sys::volume_space`), `src/ifaddrs.rs` (`getifaddrs`, `if_nameindex`: `lo` + `eth0` z netd), `src/ifreq.rs` (`ioctl` `SIOCGIF*` na socketu).
- `src/memfd.rs` - `memfd_create`, `mo_memfd_*` a `mo_start_handle` (`include/magnumopus/memfd.h`), pečetě v `fcntl`; systémová část `sys/os_mem.rs` (`mmap` MAP_SHARED pametoveho fd jde v `mman.rs::map` primo na objekt, ne pres kopii). Host: náhrady `ENOSYS` (objekt paměti jádra na hostiteli není), test jen v QEMU.
- `src/termios.rs` - `termios.h` nad řídicím kanálem terminálu (`libs/ttyproto`, `osrt::tty`), `ioctl` terminálu (`TCGETS` ... `TIOCSWINSZ`), vlákno `SIGWINCH`; `isatty` = stdio **a** kanál terminálu. Host: terminál v paměti v `sys/host.rs`.
- `src/dlfcn.rs` - `dlopen` a spol. nad tabulkou zavaděče (`osrt::dl`); statický program: `dl_iterate_phdr` popíše jediný modul ze symbolů `user/user-std.ld` (`static_phdrs`, pro libunwind); `crt/crt1.c` - start programu PIE nad `libc.so` (`_start` -> `__libc_start`) a skrytý `__dso_handle`; `crt/crtdso.c` - `__dso_handle` každé `.so` (`csysroot::ldflags_so`).
- `src/cxxsup.rs` - funkce pro běh C++ ([docs/cs/cpp.md](../../docs/cs/cpp.md)): `getentropy`, `timespec_get`, `lldiv`, `__cxa_atexit`/`__cxa_finalize` (jeden seznam s `atexit`), `__cxa_thread_atexit_impl`, `__dso_handle` statického programu.
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
- Locale: `C`/`POSIX` je jednobajtové (bajty 0x80-0xFF <-> U+DF80-U+DFFF), ostatní jména UTF-8, jméno s cizím kódováním se odmítne. Kódování se bere vždy z locale vlákna (`locale::ctype()`), nikdy z globální proměnné přímo. Čísla `LC_*` a velikost `struct lconv` hlídá test `locale_header_matches`.
- `wchar_t` je jako na Linuxu: x86_64 `i32`, aarch64 `u32` (`wchar::wchar_t`); hodnoty se zpracovávají jako `u32`.
- Nový symbol definovaný v C (`c/*.c`) patří i do `libs/cstd-so/export-c.txt` (jinak ho `libc.so` neexportuje) a do objektu, na který něco odkazuje (`c/math_ld.c`): `libc.so` vtahuje z archivu C jen odkazované objekty, samostatný nový soubor by v ní chyběl.
- Nic v cstd nesmí spoléhat na rozbalení zásobníku (`catch_unwind`): cíl má `panic=abort`. Výjimka C++ přes funkci v Rustu (zpětné volání z `qsort`) = `std::terminate`.
- `atexit` i `__cxa_atexit` jdou do jednoho seznamu v `cxxsup.rs`, spouští ho jedna funkce v `osrt::at_exit` registrovaná po vyprázdnění stdio; nejdřív destruktory `thread_local` volajícího vlákna. Konec vlákna z `pthread_create`: `run_thread_dtors` před destruktory klíčů.
- `malloc` (cstd-core) má v hlavičce i zarovnání: `aligned_alloc`, `posix_memalign` a `memalign` (cstd-core) uvolňuje obyčejný `free`.
- Čísla `errno` jsou celá tabulka Linuxu `asm-generic` (1-133); texty v `errno::message` pro každé číslo (test `errno_table_is_complete`).
- Rozložení `posix_spawn_file_actions_t` (80 B), `posix_spawnattr_t` (336 B), `sem_t` (32 B), `struct statfs` (120 B) a `siginfo_t` (128 B) jako glibc na 64 bitech; hlídají testy v `posix_tests.rs` a `spawn_tests.rs`.
- Potomek `posix_spawn` dostane jen fd 0-2; akce souborů se vyhodnotí v rodiči (`spawn::plan`), nepodporovaná akce = `ENOTSUP` hned v `posix_spawn`, nikdy tichá změna chování.

## Testy

- Host: `cargo test -p cstd` (režimy `fopen`, errno, soubory, pravidla čísel fd nad `sys/host.rs`, locale a široké znaky v `locale_tests.rs` a `wchar_tests.rs` - UTF-8 jen přes `uselocale`, globální locale testy nemění).
- QEMU: `cargo xtask test --only cstd` (program `user/ctest`, v C; krok `posix2` = `c/spawncheck.c`, ctest spouští sám sebe v režimu `child`, proto dostává kanál ke launchd); sockety `--only "cstd sit"` (`ctest net`, pomocné servery hostitele jako `std sit`).

## Pasti

- Host testy běží souběžně nad jednou tabulkou fd (`sys/host.rs`, nové číslo = nejnižší volné): po `close` nízkého čísla ho může hned dostat jiný test. Na zavřené číslo (druhé `close`, `EBADF`) a pevná čísla (`dup2`) jen vysoko (`fd_tests::HIGH` = 4000).
- `isatty` je pravda jen s řídicím kanálem terminálu: program spuštěný initem s kanály proudů má stdout plně bufferovaný (výstup se vyprázdní při konci; při pádu se může ztratit).
- Host testy, které mění pracovní adresář (`chdir`, `fchdir`), by rozbily souběžné testy s relativními cestami: úspěšné `fchdir` se testuje jen v QEMU.
- `c_long` je na hostiteli Windows 32bitový: host testy s `long` jen s hodnotami do 31 bitů.
- Program v C je crate `#![no_main]`; `main` z C volá `osrt`. Bez `extern crate cstd;` se knihovna nepřilinkuje.
- Deklarace v hlavičkách jen s implementací: CMake/configure portů by funkci našly a link by selhal.
- `locale_t` se v hlavičkách definuje pod strážcem `_CSTD_LOCALE_T` (je v `locale.h`, `wchar.h`, `wctype.h`, `ctype.h`, `string.h`, `stdlib.h`, `time.h`, `langinfo.h`).
- `include/limits.h` je z větve cstd-wchar (`MB_LEN_MAX` 4, `PATH_MAX`, `NAME_MAX`).
- `%Lf` v `printf` vypíše `?` (`long double` nejde číst z `VaList`); `*l` funkce mají přesnost `double`.
- `std::os::magnumopus::runtime` potřebuje `feature(rustc_private)` (crate ze sysrootu; atribut `unstable` na re-exportu se nevynucuje); jen pod `cfg(target_os = "magnumopus")`.
