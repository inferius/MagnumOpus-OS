# Knihovna C (cstd)

Vlastní knihovna C systému MagnumOpus, napsaná v Rustu s ABI C. Má dvě
části: **`cstd-core`** (základ bez systémových služeb pro služby `no_std`,
např. ACPICA v `acpid`) a **`cstd`** (vrstva POSIX nad [Rust std](rust-std.md)
pro aplikace). Program v C tak běží na stejném runtime jako program
v Rustu: jeden alokátor, jedno TLS, jedno stdio. Překlad C (clang,
příznaky, ochrany) popisuje [Překlad C](preklad-c.md).

> **Stav:** část 1 **[ověřeno v QEMU]** na x86_64 i aarch64 (test
> `ctest: stdio, soubory, errno, getenv, atexit`, sada `cstd`): `errno`,
> `FILE` (stdin/stdout/stderr, soubory), `exit`/`atexit`, `getenv`.
> Část 2a **[ověřeno v QEMU]** na x86_64 i aarch64 (krok `posix` téhož
> testu): čísla fd, `unistd.h`, `fcntl.h`, `sys/stat.h`, `dirent.h`, `FILE`
> nad čísly fd. Část 2b **[ověřeno v QEMU]** (krok `threads`): `pthread`.
> Část 2c **[ověřeno v QEMU]** (krok `time`): čas, kalendář, `setenv`.
> Část 2d **[ověřeno v QEMU]** (krok `math`): `math.h`, plovoucí čárka
> v `printf`, `strtod`. Část 2e **[ověřeno v QEMU]** (sada `cstd sit`):
> sockety BSD, `getaddrinfo`, `poll`, `select`, `epoll`. Část 2f
> **[ověřeno v QEMU]** (krok `mmap`): `mmap`, `mprotect`, `madvise`,
> `mseal`. Část 2g **[ověřeno v QEMU]** (krok `signal`): signály,
> `setjmp`. Zámky souborů **[ověřeno v QEMU]** (test `ctest lock`: dva
> procesy, `EDEADLK`). Část 2i **[ověřeno v QEMU]** (sada `cstd porty`):
> `libc.a`, sysroot C a porty zlib a sqlite. `libc.so` **[ověřeno v QEMU]**
> (sada `cstd dyn`: program PIE v C, vlákna, TLS, `dlopen` pluginu).
> Široké znaky a locale **[ověřeno v QEMU]** (krok `wchar`, kapitola
> [Široké znaky a locale](#široké-znaky-a-locale)). Doplněk POSIX
> (`posix_spawn`, `waitpid`, semafory, `termios`, `getifaddrs`, `statfs` ...)
> **[ověřeno v QEMU]** (krok `posix2`, kapitola
> [Doplněk POSIX](#doplněk-posix-pro-porty-a-windows-vrstvu)). Další části
> (signály od jiných procesů, `iconv`) **[navrženo]**, viz [Pokračování](#pokračování-úplná-libc-a-posix-části-2-8).

## Vrstvy

<figure class="diagram">
<svg viewBox="0 0 780 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy knihovny C">
<defs><marker id="cs1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="ok" x="10" y="10" width="370" height="50" rx="6"/><text class="t b" x="195" y="32" text-anchor="middle">aplikace v C (+ Rust)</text><text class="m" x="195" y="50" text-anchor="middle">user/ctest, později zlib, FFmpeg (proces)</text>
<rect class="box" x="400" y="10" width="370" height="50" rx="6"/><text class="t b" x="585" y="32" text-anchor="middle">služba no_std s kódem v C</text><text class="m" x="585" y="50" text-anchor="middle">acpid (ACPICA)</text>
<path class="ln" d="M195 60 L195 88" marker-end="url(#cs1)"/>
<rect class="acc" x="10" y="90" width="370" height="60" rx="6"/><text class="t b" x="195" y="113" text-anchor="middle">libs/cstd (POSIX nad std)</text><text class="m" x="195" y="131" text-anchor="middle">errno, FILE, exit/atexit, getenv</text>
<path class="ln" d="M195 150 L195 178" marker-end="url(#cs1)"/>
<path class="ln" d="M380 120 L560 178" marker-end="url(#cs1)"/>
<path class="ln" d="M585 60 L585 178" marker-end="url(#cs1)"/>
<rect class="box" x="10" y="180" width="370" height="50" rx="6"/><text class="t b" x="195" y="202" text-anchor="middle">std + osrt</text><text class="m" x="195" y="220" text-anchor="middle">soubory, stdio, at_exit, TLS</text>
<rect class="acc" x="400" y="180" width="370" height="50" rx="6"/><text class="t b" x="585" y="202" text-anchor="middle">libs/cstd-core (no_std)</text><text class="m" x="585" y="220" text-anchor="middle">string, ctype, malloc, snprintf, SSP</text>
<path class="ln" d="M195 230 L195 258" marker-end="url(#cs1)"/>
<rect class="box" x="10" y="260" width="760" height="34" rx="6"/><text class="t b" x="390" y="282" text-anchor="middle">libos + služby (fsd, ...)</text>
</svg>
<figcaption>cstd používá std (jeden runtime s Rustem) a sdílí základ cstd-core se službami no_std.</figcaption>
</figure>

- **`libs/cstd-core`** **[implementováno]**: řetězce, `ctype`, `malloc`
  (heap programu), `strtol` a spol., `snprintf`, ochrana zásobníku. Feature
  `service-io` (výchozí) přidá `printf`/`puts`/`abort` s výstupem do logu
  služby; aplikace se std ji vypínají (stdio má `cstd`).
- **`libs/cstd`** **[ověřeno v QEMU]**: hlavičky POSIX a funkce nad std.
  Symboly C exportuje jen na cíli `*-unknown-magnumopus`; na hostiteli se
  testuje logika (`cargo test -p cstd`), symboly by se tloukly se
  systémovou libc.
- **Funkce bez parametru délky** (`sprintf`, `vsprintf`, `strtok`) jsou
  pro porty k dispozici **s pojistkou, kde ji jde dat**: `sprintf`
  a `vsprintf` hlavička v C mapuje na `__sprintf_chk` /
  `__vsprintf_chk` s `__builtin_object_size` (jako `_FORTIFY_SOURCE`), takže
  přetečení bufferu známé velikosti = `abort` se zprávou místo přepsání
  paměti; `strtok` má stav per vlákno. `gets` není (pojistka nejde). Služby
  `no_std` (`cstd-core`) je nemají; nový kód má `snprintf` a `strtok_r`.

## Program v C

Program je crate `#![no_main]`, který přilinkuje `cstd` a v `build.rs`
přeloží zdroje C. Funkci `main(argc, argv)` z C zavolá při startu `osrt`
a její návratovou hodnotu předá do `exit`.

```rust
// src/main.rs
#![no_main]
extern crate cstd;
```

```rust
// build.rs: hlavičky předá cstd (links = "cstd") v DEP_CSTD_INCLUDE
fn main() {
    let dirs = std::env::var("DEP_CSTD_INCLUDE").unwrap();
    let mut b = cbuild::Build::new("app").trust(cbuild::Trust::Own).file("c/app.c");
    for d in dirs.split(';') {
        b = b.include(d);
    }
    b.compile();
}
```

Program se staví jako ostatní aplikace se std (`STD_PROGRAMS`
v `xtask/src/stdsys.rs`). `cbuild` pro cíle `*-unknown-magnumopus` překládá
hard-float (SSE2 / NEON jako cíl Rustu) a s TLS modelem `local-exec`
(`_Thread_local` a `errno` sdílí bloky TLS s Rustem).

Hlavičky `cstd` (`stdio.h`, `stdlib.h`, `string.h`) doplňují základní
hlavičky `cstd-core` přes `#include_next`; `errno.h` je jen v `cstd`.

## Funkce (část 1)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `errno.h` | `errno` (`__errno_location`), konstanty `E*` | per vlákno (TLS); čísla jako Linux/POSIX (`ENOENT` = 2, ...) |
| `stdio.h` | `stdin`, `stdout`, `stderr`, `fopen`, `fclose`, `fflush` (i `NULL`), `fread`, `fwrite`, `fgetc`, `getc`, `getchar`, `ungetc`, `fgets`, `fputc`, `putc`, `putchar`, `fputs`, `puts`, `printf`, `vprintf`, `fprintf`, `vfprintf`, `fseek`, `ftell`, `rewind`, `feof`, `ferror`, `clearerr`, `setvbuf`, `perror`, `remove`, `rename` | `snprintf` a formátovací stroj z `cstd-core` |
| `stdlib.h` | `exit`, `_Exit`, `atexit`, `getenv`, `abort` | `EXIT_SUCCESS`, `EXIT_FAILURE`; `atexit` sdílí seznam s `__cxa_atexit` ([Funkce pro C++](#funkce-pro-c-krok-7a)) |
| `string.h` | `strerror` | zbytek v `cstd-core` |
| `assert.h` | `assert` | selhání vypíše soubor a řádek na stderr a ukončí proces |

## Funkce (část 2a: čísla fd a soubory)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `unistd.h` | `close`, `read`, `write`, `pread`, `pwrite`, `lseek`, `dup`, `dup2`, `dup3`, `pipe`, `pipe2`, `isatty`, `fsync`, `fdatasync`, `ftruncate`, `truncate`, `unlink`, `rmdir`, `getcwd`, `chdir`, `access`, `sysconf`, `getpagesize` | `fork`, `vfork`, `exec*` vrací `ENOSYS` (viz [Bez fork](#procesy-locale-wchar)) |
| `fcntl.h` | `open`, `openat` (i relativně k adresáři, viz [Funkce pro C++](#funkce-pro-c-krok-7a)), `creat`, `fcntl` (`F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`, `F_SETFD`, `F_GETFL`, `F_SETFL`) | hodnoty `O_*` jako Linux, `O_DIRECTORY` stejné na obou architekturách; zámky `F_GETLK`, `F_SETLK`, `F_SETLKW` viz Zámky souborů |
| `sys/stat.h` | `stat`, `lstat` (= `stat`), `fstat`, `mkdir`, `chmod`, `fchmod`, `umask` | `st_mode` z druhu a účinných práv relace; `chmod` nemá účinek (práva jsou granty FS) |
| `sys/uio.h` | `readv`, `writev` | `writev` jedním zápisem |
| `dirent.h` | `opendir`, `fdopendir`, `readdir`, `closedir`, `rewinddir`, `dirfd` | první dvě položky `.` a `..` |
| `stdio.h` | `fdopen`, `fileno`, `fseeko`, `ftello`, `getline`, `getdelim` | `FILE` stojí nad číslem fd |

## Funkce (část 2b: vlákna)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `pthread.h` | `pthread_create`, `join`, `detach`, `self`, `equal`, `exit`, `cancel`, `testcancel`, `setcancelstate`, `setcanceltype`; atributy (`stacksize`, `detachstate`) | vlákna `osrt` (TLS jako std); zásobník výchozí 2 MiB, nejméně 16 KiB |
| `pthread.h` | `pthread_mutex_*` (normální, rekurzivní, s kontrolou chyb, `timedlock`), `pthread_cond_*` (`CLOCK_REALTIME` i `CLOCK_MONOTONIC`), `pthread_rwlock_*`, `pthread_barrier_*`, `pthread_spin_*`, `pthread_once` | nad futexem; typy s vlastním pevným rozložením, statické inicializátory = nuly |
| `pthread.h` | `pthread_key_*`, `pthread_getspecific`, `pthread_setspecific`, `pthread_cleanup_push`/`pop` | 1024 klíčů, destruktory nejvýš 4 kola |
| `sched.h` | `sched_yield` | |

- Cíl má `panic=abort`, proto `pthread_exit` a zrušení vlákno ukončí
  syscallem (`osrt::thread::exit_current`) bez rozbalení zásobníku: uklízecí
  funkce (`pthread_cleanup_push`) a destruktory klíčů proběhnou,
  destruktory `thread_local!` v Rustu toho vlákna ne.
- Zrušení je jen odložené (rozhodnutí 5): body zrušení jsou
  `pthread_testcancel`, `pthread_join` a čekání na podmínku (po kusech
  `CANCEL_SLICE`, sto milisekund; o tolik se zrušení může opozdit).
- `pthread_exit` v hlavním vlákně ukončí proces kódem 0 (ostatní vlákna
  nečeká).
- `PTHREAD_PROCESS_SHARED` není (futex je soukromý procesu).

## Funkce (část 2c: čas)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `time.h` | `clock_gettime`, `clock_getres` (`REALTIME`, `MONOTONIC`, `BOOTTIME`, `*_COARSE`, `MONOTONIC_RAW`), `nanosleep`, `clock_nanosleep` (i `TIMER_ABSTIME`), `time`, `clock`, `difftime` | čas CPU (`*_CPUTIME_ID`, `clock`) jádro nedává: `EINVAL`, `clock()` = -1 |
| `time.h` | `gmtime_r`, `localtime_r`, `gmtime`, `localtime`, `mktime`, `timegm`, `strftime`, `tzset` | `struct tm` s `tm_gmtoff` a `tm_zone` (jako glibc); `strftime` v locale C vč. ISO týdne (`%G`, `%V`) |
| `sys/time.h` | `gettimeofday` | |
| `unistd.h` | `sleep`, `usleep` | |
| `stdlib.h` | `setenv`, `unsetenv` | |

- **Místní čas:** pásmo z proměnné `TZ` - jméno IANA (`Europe/Prague`)
  nebo pevný posun ve stylu ISO (`UTC+2`), jinak UTC. Pravidla pásem
  jsou v `libs/tz` (bez tzdata). Řetězce POSIX typu `CET-1CEST` se
  nečtou. Předání pásma systému (nastavení času) do `TZ` při spuštění
  programu **[navrženo]**.
- Spánek se nepřeruší (signály nejsou, `EINTR` nevzniká), `rem` je vždy 0;
  `nanosleep`, `sleep` a `usleep` jsou body zrušení vlákna.

## Funkce (část 2d: matematika a plovoucí čárka)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `math.h` | goniometrické, hyperbolické, `exp*`, `log*`, `pow`, `sqrt`, `cbrt`, `hypot`, zaokrouhlení (`ceil`, `floor`, `trunc`, `round`, `rint`, `lround`, ...), `fmod`, `remainder`, `remquo`, `frexp`, `ldexp`, `modf`, `fma`, `erf`, `tgamma`, `lgamma`, Besselovy funkce; varianty `*f` a `*l`; makra `isnan`, `fpclassify`, `M_PI` ... | crate `libm` (port musl libm do Rustu, MIT/Apache); `errno` a výjimky FPU funkce nenastavují (`math_errhandling` = 0) |
| `stdio.h` | `%f %F %e %E %g %G %a %A` v rodině `printf` | číslice z `core::fmt` (přesné zaokrouhlení binární hodnoty), výstup jako glibc |
| `stdlib.h` | `strtod`, `strtof`, `strtold`, `atof` | desítkový i šestnáctkový tvar, `inf`, `nan(...)`; `ERANGE` při přetečení i podtečení |

- **`long double`** má ABI jako Linux (x86_64 80 bitů, aarch64 128 bitů),
  funkce `*l` a `strtold` ale počítají v přesnosti `double` (rozhodnutí 9).
  Jsou napsané v C (`libs/cstd/c/math_ld.c`, přeloží je `build.rs` jen pro
  cíl magnumopus), protože Rust typ `long double` nezná.
- `%Lf` (`long double` v `printf`) zatím neumíme přečíst z `VaList`:
  vypíše `?` a argument nespotřebuje.
- `scanf` **[navrženo]**.

## Funkce (část 2e: sockety a čekání)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `sys/socket.h` | `socket` (`AF_INET`, `AF_INET6`; `SOCK_STREAM`, `SOCK_DGRAM`, `SOCK_NONBLOCK`, `SOCK_CLOEXEC`), `bind`, `listen`, `accept`, `accept4`, `connect`, `send`, `recv`, `sendto`, `recvfrom`, `sendmsg`, `recvmsg` (bez řídicích zpráv), `shutdown`, `getsockname`, `getpeername`, `setsockopt`, `getsockopt` | `AF_UNIX` a `socketpair` = `EAFNOSUPPORT`; `MSG_PEEK` = `ENOTSUP` |
| `netinet/in.h`, `arpa/inet.h` | `htons` ..., `inet_pton`, `inet_ntop`, `inet_addr`, `inet_ntoa`, `in6addr_any` | |
| `netdb.h` | `getaddrinfo`, `freeaddrinfo`, `gai_strerror`, `getnameinfo` | jména přes resolver netd; `getnameinfo` jen číselně |
| `poll.h`, `sys/select.h`, `sys/epoll.h` | `poll`, `select`, `epoll_create`, `epoll_create1`, `epoll_ctl`, `epoll_wait` | nad `osrt::poll` |

- Socket je objekt v tabulce fd (`osrt::sock`): po `socket()` ještě nemá
  spojení u netd, `connect`/`listen`/`bind` ho převedou na spojení TCP,
  naslouchání nebo socket UDP. `read`, `write`, `fdopen` a `close` fungují
  i nad socketem.
- Co program smí (cíle, porty, naslouchání), určuje relace sítě
  z manifestu: zakázaný cíl nebo port = `EACCES`. `SO_REUSEADDR`,
  `SO_KEEPALIVE`, `TCP_NODELAY` a velikosti bufferů se přijmou bez účinku,
  `SO_RCVTIMEO`/`SO_SNDTIMEO` platí.
- `O_NONBLOCK` (`SOCK_NONBLOCK`, `MSG_DONTWAIT`) = termín `TRY` u netd,
  bez dat `EAGAIN`. `connect` se dokončí hned i v neblokujícím režimu
  (POSIX to dovoluje), `SO_ERROR` je proto vždy 0.
- **Čekání** (`osrt::poll`): soubory jsou vždy připravené, stdin a roury
  čekají na signál kanálu (`READABLE`, `PEER_CLOSED`), sockety na událost
  připravenosti netd (netproto v2). Kanál nemá signál „lze zapisovat“,
  proto stdout a zapisovací konec roury hlásí `POLLOUT` vždy. Do 64 objektů
  `OBJECT_WAIT_MANY`, víc přes port.
- **`epoll`** je množina v tabulce fd; úrovňový režim přesně, `EPOLLET`
  hlásí jen nově nastavené bity, `EPOLLONESHOT` položku vypne do
  `EPOLL_CTL_MOD`. Backend mio přímo nad připraveností **[navrženo]**.

## Funkce (část 2f: mapování paměti)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `sys/mman.h` | `mmap`, `munmap` (i část), `mprotect`, `madvise`, `posix_madvise`, `mseal`, `msync`, `mlock`, `munlock` | nad `libos::vm` (přes `osrt::vm`) |
| `unistd.h` | `sysconf(_SC_PAGESIZE)`, `getpagesize` | velikost stránky od jádra (ne konstanta) |

- **Anonymní paměť** má stránky na požádání (první zápis). `PROT_NONE` =
  jen rezervace adres bez paměti; `mprotect` s právy ji doplní (vzor GC
  a JIT: rezervovat velký rozsah, používat po částech). `MAP_FIXED`
  nahradí, co v rozsahu bylo; `MAP_FIXED_NOREPLACE` jen volné místo.
- **`MAP_SHARED` anonymní** je v rámci procesu totéž co soukromá (`fork`
  není).
- **Soubor:** obsah se načte do anonymní paměti (`MAP_PRIVATE`, nebo
  `MAP_SHARED` jen pro čtení - pozdější změny souboru se neprojeví).
  `MAP_SHARED` se zápisem = `ENOTSUP`, dokud fsd nedá objekt paměti
  souboru **[navrženo]**. Roura nebo socket = `ENODEV`.
- **W^X:** `PROT_EXEC` na nezapečetěné paměti jen s oprávněním JIT
  z manifestu ([sprava-pameti.md](sprava-pameti.md)), jinak `EACCES`.
- `madvise(MADV_DONTNEED | MADV_FREE)` vrátí paměť a další čtení vrátí
  nuly; ostatní rady se přijmou bez účinku. `mseal` zapečetí rozsah se
  současnými právy (pak nejde odmapovat ani změnit práva).

## Funkce (část 2g: signály a nelokální skoky)

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `signal.h` | `signal`, `sigaction` (`SA_SIGINFO`, `SA_RESETHAND`, `SA_NODEFER`), `raise`, `kill`, `sigprocmask`, `pthread_sigmask`, `sigpending`, `sigsuspend`, `sig*set`, `sigaltstack`, `strsignal`, `psignal` | čísla jako Linux |
| `unistd.h`, `sys/time.h` | `alarm`, `pause`, `setitimer` (`ITIMER_REAL`), `getpid`, `getppid` | |
| `setjmp.h` | `setjmp`, `longjmp`, `sigsetjmp`, `siglongjmp`, `_setjmp`, `_longjmp` | `jmp_buf` 320 B na obou architekturách; nahé funkce v asembleru |

Rozhodnutí 6A - signály bez podpory jádra:

- **Synchronní** (`SIGSEGV`, `SIGBUS`, `SIGFPE`, `SIGILL`, `SIGTRAP`):
  první `sigaction` s obsluhou zaregistruje obsluhu výjimek ve vlákně
  ([vyjimky.md](vyjimky.md)); ta zavolá obsluhu signálu v témže vlákně se
  `si_addr` z výjimky. Návrat z obsluhy = pokračovat (instrukce se
  zopakuje - vzor GC a JIT: `mprotect` v obsluze), `SIG_DFL` = další
  obsluha a konec procesu. Odchod `siglongjmp` funguje. `ucontext_t`
  (třetí argument) zatím vyplněný není (nuly) **[navrženo]**. Na aarch64
  dělení nulou `SIGFPE` nedá (CPU ho nehlásí).
- **`raise`, `kill` na vlastní proces:** obsluha běží hned ve volajícím
  vlákně; blokovaný signál čeká a doručí se po odblokování.
- **Časovače** (`alarm`, `setitimer`): obsluha `SIGALRM` běží ve zvláštním
  vlákně signálů (jako obsluha Ctrl+C konzole ve Windows); `EINTR`
  nevzniká, `pause` a `sigsuspend` se probudí.
- **Výchozí akce:** konec procesu kódem 128 + číslo signálu (`SIGABRT` =
  `abort`), `SIGCHLD`, `SIGCONT`, `SIGWINCH` a zastavovací signály se
  ignorují.
- **Nejsou:** signály od jiných procesů (terminál, rodič; událost `SIGNAL`
  v bloku parametrů **[navrženo]**), `SIGPIPE` (zápis do zavřené roury
  vrátí jen `EPIPE`).
- **`getpid`:** jádro programům číslo procesu nedává; `getpid` vrací
  náhodné kladné číslo, stálé po dobu běhu (jména dočasných souborů,
  `kill` na sebe). `getppid` = 1.

### Zámky souborů

`fcntl(F_GETLK/F_SETLK/F_SETLKW)`, `flock` (`sys/file.h`) a `lockf`
**[ověřeno v QEMU]** (sada `cstd`, test `ctest lock`: dva procesy nad
jedním souborem, konflikt, `F_GETLK`, uváznutí, uvolnění koncem procesu).
Zámky drží fsd (zpráva `POSIX_LOCK`, [souborovy-system.md](souborovy-system.md)):
vlastník `fcntl` je proces (relace FS), `flock` otevřený soubor. Uvolní je
zavření posledního fd procesu na souboru (`flock`: zavření fd) a konec
procesu. `F_SETLKW` a `flock` bez `LOCK_NB` čekají opakovaným zkoušením;
fsd přitom hlídá cyklus čekání (`EDEADLK`). `l_pid` z `F_GETLK` je -1
(čísla procesů fsd nezná).

## Funkce pro C++ (krok 7a)

**[ověřeno v QEMU]** x86_64 i aarch64 (sada `cpp`, [běh C++](cpp.md)); host testy
`cxxsup_tests`, `fsat_tests`.

| Hlavička | Funkce | Poznámky |
|---|---|---|
| `stdlib.h` | `lldiv`, `realpath` | zarovnaná paměť (`aligned_alloc`, `posix_memalign`, `memalign`) je v cstd-core: blok uvolní obyčejný `free` |
| `unistd.h`, `sys/random.h` | `getentropy` (nejvýš 256 B), `unlinkat`, `pathconf`, `fpathconf` | `getentropy` čeká, dokud generátor jádra není nasazený (jako Linux) |
| `time.h` | `timespec_get` (`TIME_UTC`, `TIME_MONOTONIC`) | |
| `fcntl.h`, `sys/stat.h` | `openat`, `mkdirat`, `fstatat`, `fchmodat` relativně k adresáři; `AT_REMOVEDIR`, `AT_SYMLINK_NOFOLLOW` | cesta adresáře zapamatovaná při otevření + jméno (FS pracuje s cestami: bez ochrany před přejmenováním adresáře mezi otevřením a voláním) |
| `sys/statvfs.h` | `statvfs`, `fstatvfs` | velikost a volné bloky svazku relace fsd; počty inodů 0 |
| `sys/sendfile.h` | `sendfile` | kopie přes buffer 64 KiB v procesu |
| `stdio.h` | `setbuf` | jako `setvbuf` (`NULL` = bez bufferu); `std::filebuf` |
| `limits.h` | `PATH_MAX` (4096), `NAME_MAX` (255) | |
| `link.h`, `elf.h` | `dl_iterate_phdr`, `struct dl_phdr_info`, typy ELF64 | statický program: jeden modul ze symbolů linker skriptu (`PT_LOAD`, `PT_GNU_EH_FRAME`) |
| (ABI C++) | `__cxa_atexit`, `__cxa_finalize`, `__cxa_thread_atexit_impl`, `__dso_handle` | `atexit` a `__cxa_atexit` mají jeden seznam (společné obrácené pořadí) |

Sysroot C překládá s `-funwind-tables` (výjimky C++ projdou rámci C)
a linkuje s `--eh-frame-hdr`; dynamická část má `lib/crtdso.o` se skrytým
`__dso_handle` pro každou sdílenou knihovnu.

## Sysroot C a porty (část 2i)

**[ověřeno v QEMU]** x86_64 i aarch64 (sada `cstd porty`).

<figure class="diagram">
<svg viewBox="0 0 780 170" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sestavení portu v C proti sysrootu">
<defs><marker id="cs3" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="220" height="60" rx="6"/><text class="t b" x="120" y="34" text-anchor="middle">zdroje portu</text><text class="m" x="120" y="52" text-anchor="middle">HTTPS + SHA-256, tar</text>
<rect class="acc" x="280" y="10" width="220" height="60" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">clang + sysroot C</text><text class="m" x="390" y="52" text-anchor="middle">include, clang.cfg</text>
<rect class="acc" x="550" y="10" width="220" height="60" rx="6"/><text class="t b" x="660" y="34" text-anchor="middle">ld.lld</text><text class="m" x="660" y="52" text-anchor="middle">libc.a, magnumopus.ld</text>
<path class="ln" d="M230 40 L276 40" marker-end="url(#cs3)"/>
<path class="ln" d="M500 40 L546 40" marker-end="url(#cs3)"/>
<path class="ln" d="M660 70 L660 98" marker-end="url(#cs3)"/>
<rect class="ok" x="400" y="100" width="370" height="60" rx="6"/><text class="t b" x="585" y="124" text-anchor="middle">statický program ET_EXEC v initfs</text><text class="m" x="585" y="142" text-anchor="middle">zlib-example, minigzip, sqlite3, sqltest</text>
<rect class="box" x="10" y="100" width="370" height="60" rx="6"/><text class="t b" x="195" y="124" text-anchor="middle">libc.a = cstd + std + osrt</text><text class="m" x="195" y="142" text-anchor="middle">crate libs/cstd-libc (staticlib), _start z osrt</text>
</svg>
<figcaption>Port v C se přeloží clangem proti sysrootu a slinkuje s libc.a; _start z osrt zavolá jeho main.</figcaption>
</figure>

- **`libc.a`** (crate `libs/cstd-libc`, `staticlib`): `cstd`, `std`
  a `osrt` včetně `_start`, který zavolá `main` programu v C. Staví se se
  std aplikacemi (`stdsys::STD_LIBS`).
- **Sysroot C** `target/c-sysroot/<arch>/` (`xtask/src/csysroot.rs`):
  `include/` (cstd) a `include-core/` (cstd-core), `lib/libc.a`,
  `lib/magnumopus.ld` (linker skript programu se std) a `clang.cfg`
  s příznaky překladu i linku pro `clang --config=...` - cizí build
  systémy (`configure`, CMake, Meson) ho použijí jako křížový překlad
  **[implementováno]** (porty níže ho používají přes stejné příznaky).
- **Porty** (`xtask/src/cports/`, mechanismus popisuje [Porty v C](porty-c.md)): zdroje z oficiálních míst přes HTTPS
  (`curl`), ověřený SHA-256 (zlib shodný s hashem vydání; sqlite připnutý
  z prvního stažení ze sqlite.org), rozbalení `tar`, překlad clangem
  s `-w` a link s `libc.a`. Objekty se překládají znovu jen při změně.
  - **zlib 1.3.1** (licence zlib): `zlib-example` (vlastní test balíku)
    a `minigzip`; test: všechny kroky `example`, komprese souboru a zpět
    se shodným obsahem.
  - **sqlite 3.46.1** (public domain): shell `sqlite3` a `sqltest`
    (`user/ctest/c/sqltest.c`); výchozí unixové VFS se zámky `fcntl`
    ve fsd. Bez WAL a mmap I/O (`SQLITE_OMIT_WAL`,
    `SQLITE_MAX_MMAP_SIZE=0`: sdílená paměť souboru zatím není), bez
    `load_extension` (statický program). Test: shell nad souborem
    a trvalost, dva procesy nad jednou databází - zápis při cizí
    exkluzivní transakci = `SQLITE_BUSY`, s `busy_timeout` uspěje po
    `COMMIT` druhého procesu.
- Doplněné funkce pro porty: `strspn`, `strcspn`, `strpbrk`, `strtok_r`,
  `strdup`, `strndup`, `strcasecmp`, `strerror_r`, `qsort`, `bsearch`,
  `rand`, `div`, `atol`, rodina `scanf`, `getpwuid` (uživatel z `USER`
  a `HOME`), `getuid` a spol. (0), `fchown` a `utimes` bez účinku,
  `readlink`/`symlink`/`link` (symlinky FS zatím nejsou), `ioctl`
  (`ENOTTY`), `setlocale`/`localeconv` (viz [Široké znaky a locale](#široké-znaky-a-locale)), `uname`,
  `getrusage`/`getrlimit`, `system` (`ENOSYS`; spouštění procesů
  `posix_spawn`, viz [Doplněk POSIX](#procesy-posix_spawn-a-waitpid)).
- **`libc.so`**: viz kapitola [libc.so](#libcso) níže.

## libc.so

**[ověřeno v QEMU]** x86_64 i aarch64 (sada `cstd dyn`).

- **Druhý cíl std** `*-unknown-magnumopus-dyn`
  (`third_party/rust-std/targets/*-dyn.json`): `relocation-model: pic`,
  `tls-model: initial-exec`, `dynamic-linking`, `env: dyn`. Vlastní sysroot
  vedle statického (`stdsys::build_sysroot_dyn_locked`).
- **`osrt` pod zavaděčem** (`cfg(target_env = "dyn")`): bez `_start`
  a bez vlastního bloku TLS (bloky TLS všech modulů včetně `libc.so` dělá
  `ld.so`); start je `__libc_start(main, bootstrap, sp, magic)`, který
  zavolá `_start` programu z `crt1.o`; `exit` po funkcích `atexit` spustí
  finalizátory modulů (`run_fini`).
- **`libc.so`** (crate `libs/cstd-so`, `cdylib`, `SONAME libc.so`): cstd +
  std + osrt, `BIND_NOW`, plné RELRO, bez TEXTREL, segmenty 64 KiB; TLS
  initial-exec (knihovna je ve statickém TLS od startu). Žádné nedefinované
  symboly, `dlopen` a spol. jdou přes tabulku zavaděče (`dlfcn.h`).
- **Sysroot C pro dynamické linkování:** `lib/libc.so`, `lib/crt1.o`
  (`libs/cstd/crt/crt1.c`) a `clang-dyn.cfg` (`-fPIE`, `-pie`,
  `PT_INTERP /system/lib/ld.so`, `-Wl,-Bdynamic -l:libc.so` - ovladač clangu
  pro baremetal by jinak linkoval staticky). Statický `clang.cfg` zůstává.
- **Test** (sada `cstd dyn`): `cdyn` (PIE přes cbuild) - start přes
  `crt1.o`, stdio, matematika, 4 vlákna s vlastním TLS a `errno`,
  `dlopen("libcplug.so")` s TLS pluginu po vláknech (dynamické TLS přes
  DTV), `malloc` v pluginu a `free` v programu (jeden heap), soubory,
  `atexit`; `cdyn-hello` přeložený jen přes `clang --config=clang-dyn.cfg`
  (cesta pro cizí build systémy).
- Programy v Rustu zůstávají statické; procesy v C jsou buď statické
  (`libc.a`), nebo dynamické (`libc.so`), vždy s jedním runtime.

Čas sestavení std (čistý překlad, x86_64, hostitel s 32 vlákny; druhý
sysroot se staví jen při změně std nebo osrt):

| Co | Čas | Revize |
|---|---|---|
| std statický, stav před `libc.so` | 20 s | `eade6bf` |
| std statický | 19 s | `fcbc153` |
| std jako PIC (druhý cíl) | 17 s | `fcbc153` |
| `libc.so` nad sysrootem PIC | 3 s | `fcbc153` |

### Čísla fd

- Tabulka je v `osrt` (`osrt::fd`), jedna na proces; `cstd` ji vidí přes
  `std::os::magnumopus::runtime` (re-export crate `osrt` ve std, nestabilní
  feature `magnumopus_runtime`, jen pro knihovny nad std).
- Číslo ukazuje na popis otevření: soubor relace FS s pozicí, stdio procesu
  (0, 1, 2 od startu) nebo konec roury. `dup` sdílí popis i pozici.
- Nejvýš 4096 čísel (`EMFILE`). `FD_CLOEXEC` se ukládá (pro
  `posix_spawn`, potomek dostává jen 0-2).
- Roura je dvojice kanálů; čtecí konec nejde psát (`EBADF`), roura nemá
  pozici (`ESPIPE`).
- Na hostiteli (`cargo test -p cstd`) stejná pravidla nad `std::fs`
  (`sys/host.rs`), aby šla logika `FILE` a fd testovat bez QEMU.
- `std` čísla fd zatím nevidí (`AsRawFd`/`FromRawFd` pro `File` v Rustu
  **[navrženo]**).

### Proudy a buffery

- `stdout` po řádcích, `stderr` bez bufferu, soubory 4 KiB (`BUFSIZ`).
  `setvbuf` mění režim, vlastní buffer volajícího se nepoužije.
- Před čtením `stdin` se vyprázdní `stdout` (výzva bez konce řádku).
- Každý `FILE` má vlastní zámek; vlákna smí sdílet proud.
- Při konci procesu (`exit`, návrat z `main`) se vyprázdní všechny proudy:
  `cstd` zaregistruje vyprázdnění přes `std::os::magnumopus::rt::at_exit`
  (funkce `osrt`), stejný seznam používá `atexit`. Funkce se volají
  v obráceném pořadí registrace; vyprázdnění je registrované jako první,
  takže proběhne po všech funkcích `atexit` (ty smí ještě psát).
- `_Exit` končí bez `atexit` a bez vyprázdnění (`rt::exit_now`).

### Chyby

Chyba systému (`abi::Error`, [chyby.md](chyby.md)) se převádí na `errno`
(`errno::from_os`: `NOT_FOUND` -> `ENOENT`, `ACCESS_DENIED` -> `EACCES`,
`BAD_HANDLE` -> `EBADF`, `PEER_CLOSED` -> `EPIPE`, ...; neznámá `EIO`),
chyba `std::io::Error` podle druhu (`errno::from_io`). Neplatný režim
`fopen` = `EINVAL`.

## Omezení části 1

- Části 2a-2d výše doplnily čísla fd, vlákna, čas, `math.h`
  a plovoucí čárku; sockety a další přináší pokračování níže.

## Pokračování: úplná libc a POSIX (části 2-8)

**[navrženo]**, schváleno 2026-11-14 (krok 2 v pořadí převzetí cizího
softwaru, [architektura.md](architektura.md#bezpečnost)); část 2a hotová
(výše). Cíl: přeložit beze změn zdrojů zlib, sqlite, pak závislosti SDL
a FFmpeg (LGPL, samostatný proces). Rozhodnutí jsou v kapitole
[Rozhodnutí](#rozhodnutí).

### Dva druhy procesů, jeden runtime

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Statický a dynamický proces s knihovnou C">
<defs><marker id="cs2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<text class="t b" x="195" y="20" text-anchor="middle">statický (ET_EXEC)</text>
<text class="t b" x="585" y="20" text-anchor="middle">dynamický (PIE + ld.so)</text>
<rect class="ok" x="10" y="30" width="370" height="44" rx="6"/><text class="t b" x="195" y="50" text-anchor="middle">program v Rustu + statické C</text><text class="m" x="195" y="66" text-anchor="middle">přehrávač s dekodérem, ctest, zlib v Rustu</text>
<rect class="ok" x="400" y="30" width="370" height="44" rx="6"/><text class="t b" x="585" y="50" text-anchor="middle">program v C/C++ + knihovny .so</text><text class="m" x="585" y="66" text-anchor="middle">FFmpeg (proces), sqlite3, porty z configure/cmake</text>
<path class="ln" d="M195 74 L195 98" marker-end="url(#cs2)"/>
<path class="ln" d="M585 74 L585 98" marker-end="url(#cs2)"/>
<rect class="acc" x="10" y="100" width="370" height="60" rx="6"/><text class="t b" x="195" y="122" text-anchor="middle">libc.a = cstd + std + osrt</text><text class="m" x="195" y="140" text-anchor="middle">přilinkováno do programu</text>
<rect class="acc" x="400" y="100" width="370" height="60" rx="6"/><text class="t b" x="585" y="122" text-anchor="middle">libc.so = cstd + std + osrt</text><text class="m" x="585" y="140" text-anchor="middle">jedna kopie na proces, sdílený kód přes libd</text>
<path class="ln" d="M195 160 L195 188" marker-end="url(#cs2)"/>
<path class="ln" d="M585 160 L585 188" marker-end="url(#cs2)"/>
<rect class="box" x="10" y="190" width="760" height="50" rx="6"/><text class="t b" x="390" y="212" text-anchor="middle">libos + služby (fsd, netd, launchd, libd)</text><text class="m" x="390" y="230" text-anchor="middle">tabulka fd, vlákna, heap, TLS a at_exit jsou v osrt: C i Rust vidí totéž</text>
</svg>
<figcaption>Každý proces má právě jeden runtime (osrt): staticky v programu, nebo jako libc.so pod zavaděčem.</figcaption>
</figure>

- **Statický proces** (dnešní stav): program v Rustu se std, C přilinkované
  staticky (`libc.a`). Rust zůstává statický
  ([dynamicke-linkovani.md](dynamicke-linkovani.md)).
- **Dynamický proces**: program v C/C++ jako PIE, `libc.so` z `/system/lib`.
  `libc.so` obsahuje `cstd`, `std` i `osrt` (jeden `cdylib`), takže i tady
  je jeden heap, jedna tabulka fd a jedno TLS.
- ⚠️ **Omezení:** proces nesmí mít dva runtime. Plugin v Rustu se std
  (`cdylib`) načtený přes `dlopen` do procesu v C by přinesl druhý std
  (druhý heap, druhé `at_exit`). Pluginy v Rustu pro procesy v C budou
  `no_std` nad C ABI, nebo se std vyřeší až s Rust std jako `.so`.

### Co je vlastní a co převzaté

Systémová vrstva je **vlastní v Rustu** (napojení na libos a služby,
v musl nejde převzít): stdio, fd, vlákna, čas, sockety, `mmap`, signály,
`posix_spawn`. Čistě algoritmické části se **převezmou** (MIT/BSD, žádná
GPL/LGPL): matematika, převody čísel, `qsort`, regulární výrazy, `getopt`.

| Oblast | Řešení | Odhad |
|---|---|---|
| stdio, `unistd`, `fcntl`, `stat`, `dirent`, `pipe`, `dup` | vlastní (osrt + cstd) | ~3 500 řádků Rustu |
| `pthread` (vlákna, mutex, cond, rwlock, barrier, spin, once, klíče) | vlastní nad `osrt::thread` a futexem | ~1 500 |
| čas (`clock_gettime`, `nanosleep`, `localtime_r`, `strftime`) | vlastní nad `libos::time` a `libs/tz` | ~900 |
| sockety BSD, `getaddrinfo`, `poll`, `select`, `epoll` | vlastní nad netproto v2 a readiness | ~2 000 |
| `mmap`, `mprotect`, `munmap`, `madvise` | vlastní nad `libos::vm` | ~400 |
| signály (`sigaction`, `raise`, `kill`, `sigprocmask`) | vlastní nad `libos::exception` | ~800 |
| `math.h` | převzato (otázka 3) | ~10 000 převzatých |
| `printf` s plovoucí čárkou, `strtod`, `scanf` | vlastní nad `core` (přesné převody Rustu) | ~800 |
| `qsort`, `bsearch`, `getopt`, `regex.h` (TRE), `fnmatch`, `glob` | převzato z musl (MIT) | ~4 000 převzatých |
| `wchar.h`, `wctype.h`, `locale.h`, `iconv` (jen UTF-8/16/32, Latin-1) | vlastní | ~900 |

Vlastního kódu ~11 000 řádků Rustu, převzatého ~14 000 řádků. Celá musl má
~90 000 řádků; převzetí celé by znamenalo přepsat její systémovou vrstvu
(syscally Linuxu) a dva runtime vedle Rustu.

### Části

1. **pthread.** Vlákna přes `osrt::thread` (pod zavaděčem TLS bloky dělá
   `ld.so`, [DL-3](dynamicke-linkovani.md#vlákna-fini-a-debugger-dl-3)),
   takže `_Thread_local`, `errno` i `thread_local!` v Rustu fungují ve
   vláknech z C. Mutex (normální, rekurzivní, s kontrolou chyb),
   `cond` (i `CLOCK_MONOTONIC` přes `pthread_condattr_setclock`), rwlock,
   barrier, spin, once a klíče (1024, destruktory při konci vlákna) nad
   futexem. Velikost zásobníku z atributu, ochranná oblast pod zásobníkem
   (`map_stack`). `PTHREAD_PROCESS_SHARED` ne (futex je soukromý procesu).
   Zrušení vlákna: otázka 5.
2. **Soubory a fd.** Tabulka fd v `osrt` (`osrt::fd`): číslo -> popis
   otevření (soubor fsproto s pozicí, proud stdio, konec roury, socket,
   adresář). `dup` sdílí popis i pozici jako POSIX. `std` dostane fd jen
   na požádání (`AsRawFd`, `FromRawFd`, `IntoRawFd` v
   `std::os::magnumopus::io`), běžný `File` v Rustu číslo nezabírá.
   `open` (`O_CREAT`, `O_EXCL`, `O_TRUNC`, `O_APPEND`, `O_NONBLOCK`,
   `O_CLOEXEC`, `O_DIRECTORY`), `read`/`write`/`pread`/`pwrite`/`readv`/
   `writev`, `lseek`, `ftruncate`, `fsync`, `stat`/`fstat`/`lstat`
   (`st_mode` z druhu a účinných práv, `st_uid` z identity relace),
   `opendir`/`readdir`, `mkdir`, `unlink`, `rmdir`, `rename`, `access`,
   `getcwd`/`chdir`, `pipe` (dvojice kanálů), `isatty`.
   `chmod` bez účinku (práva jsou granty FS, ne bity), `link`/
   `symlink` = `ENOTSUP` do FS v3. Zámky `fcntl`/`flock` drží fsd
   (rozhodnutí 8).
3. **`math.h`** (otázka 3) a plovoucí čárka v `printf`/`scanf`/`strtod`.
   Hard-float je všude (x86-64-v2, NEON). `long double`: otázka 9.
4. **Čas a signály.** `clock_gettime` (`REALTIME`, `MONOTONIC`,
   `BOOTTIME`; `PROCESS_CPUTIME_ID`/`THREAD_CPUTIME_ID` jen pokud jádro dá
   čas CPU, jinak `EINVAL`), `nanosleep`, `clock_nanosleep`, `time`,
   `gettimeofday`, `localtime_r`/`mktime`/`strftime` se zónou systému
   (`libs/tz`). Synchronní signály (`SIGSEGV`, `SIGBUS`, `SIGFPE`,
   `SIGILL`, `SIGTRAP`) nad obsluhou výjimek ve vlákně
   ([vyjimky.md](vyjimky.md)): `sigaction` s `SA_SIGINFO`, `ucontext_t`
   z `Frame`, `sigaltstack`. Asynchronní signály (`SIGINT`, `SIGTERM`,
   `SIGCHLD`, `SIGALRM`, `SIGPIPE`): otázka 6. Na aarch64 dělení nulou
   `SIGFPE` nedá (CPU ho nehlásí).
5. **Sockety a čekání.** `socket` (`AF_INET`, `AF_INET6`; `SOCK_STREAM`,
   `SOCK_DGRAM`), `bind`, `listen`, `accept`, `connect`, `send`/`recv`/
   `sendto`/`recvfrom`/`sendmsg`/`recvmsg` (bez řídicích zpráv),
   `shutdown`, `getsockname`/`getpeername`, `setsockopt` (podmnožina:
   `SO_REUSEADDR`, `TCP_NODELAY`, `SO_RCVTIMEO`, `SO_SNDTIMEO`,
   `SO_KEEPALIVE`), `getaddrinfo`/`getnameinfo` přes resolver netd,
   `O_NONBLOCK` (termín `TRY`). `poll` a `select` přes `OBJECT_WAIT_MANY`
   (do 64 objektů), nad tím přes port (`libos::port`): soubory jsou vždy
   připravené, kanály mají `READABLE`/`PEER_CLOSED`, sockety události
   připravenosti (netproto v2). `AF_UNIX` ne (místní IPC jsou kanály).
   `epoll`: otázka 7.
6. **Paměť.** `mmap` anonymní (`MAP_PRIVATE`/`MAP_SHARED` v procesu,
   `MAP_FIXED`, `MAP_NORESERVE` = rezervace se stránkami na požádání),
   `munmap` i části, `mprotect`, `madvise(DONTNEED)` = decommit,
   `mseal` = pečeť. `PROT_EXEC` na nezapečetěné paměti jen s oprávněním
   JIT z manifestu (W^X, [sprava-pameti.md](sprava-pameti.md)), jinak
   `EACCES`. `mmap` souboru: `MAP_PRIVATE` jen pro čtení načtením do
   anonymní paměti; `MAP_SHARED` souboru = `ENOTSUP`, dokud fsd nedá VMO
   souboru.
7. **libc.a a libc.so** (otázky 1 a 2): `libc.a` (Rust `staticlib`)
   a `libc.so` (`cdylib`, `SONAME` `libc.so`), `crt1.o`/`crti.o`/`crtn.o`,
   hlavičky. xtask z nich složí **sysroot C** (`target/c-sysroot/<arch>`),
   takže porty se staví vlastním sestavením (`configure`, CMake, Meson)
   jako křížový překlad s `--sysroot`.
8. **Porty jako test:** zlib (vlastní testy balíku v QEMU), sqlite
   (amalgamace, testovací shell s `:memory:` i soubory), pak závislosti
   SDL a FFmpeg (otázka 10).

### Procesy, locale, wchar

- **Bez `fork`** (rozhodnutí uživatele, ani omezený `fork`): procesy jen
  přes `spawn` ([rust-std.md](rust-std.md)). `fork()` vrací -1 s `errno`
  = `ENOSYS`. `posix_spawn` a `posix_spawnp` s
  `posix_spawn_file_actions_*` a `posix_spawnattr_*`: aplikace
  s manifestem přes `launchd`, pomocné programy přes `spawn` (jako
  `std::process`), akce souborů pro fd 0-2 (otázka 4), `waitpid` nad
  handlem procesu (hotovo, viz [Doplněk POSIX](#procesy-posix_spawn-a-waitpid)),
  `system`/`popen` přes shell systému **[navrženo]**. Spuštění programu
  PIE (dynamický proces) potřebuje u spouštějícího relaci s libd
  (`spawn::Options::libd`, DL-4b); `cstd` ji předá, má-li ji proces sám.
- **`vfork`** (otázka 11): návrh A **(doporučeno)** také -1 s `ENOSYS`.
  Alias na pomocnou funkci spawn nejde: `vfork` vrací dvakrát a dítě do
  `exec*` běží v paměti rodiče, což bez `fork` v jádře nesplníme; tichý
  alias by vedl ke špatnému chování místo jasné chyby. `exec*` (`execv`,
  `execvp`, ...) bez předchozího `fork` = nahrazení procesu, nemáme ho
  také: `ENOSYS`.
- **Hlavičky upozorní už při překladu:** `fork`, `vfork` a `exec*` jsou
  v `unistd.h` deklarované s `__attribute__((deprecated("MagnumOpus: use
  posix_spawn")))`. Symboly existují (link projde, testy `configure`
  na křížový překlad je najdou), volání za běhu vrátí `ENOSYS`.

#### Jak upravit portovaný program

| V programu | Úprava |
|---|---|
| `fork()` + `exec*()` v dítěti (spuštění příkazu) | `posix_spawn`/`posix_spawnp`; přesměrování (`dup2` v dítěti) -> `posix_spawn_file_actions_adddup2`, zavření -> `..._addclose`, `chdir` -> `..._addchdir` |
| `fork()` + `exec` přes shell, roura na výstup | `popen` / `system` |
| `fork()` pro paralelní práci (pracovníci) | vlákna (`pthread`), nebo spuštění sebe sama s argumentem role přes `posix_spawn` a předání dat rourou |
| `fork()` pro démona (odpojení od terminálu) | vynechat: služby spouští a hlídá init/launchd |
| `vfork()` + `exec*()` | stejně jako `fork` + `exec` |
| `fork()` pro izolaci (sandbox dítěte) | `posix_spawn` jiného programu s vlastním manifestem (izolaci dává manifest, ne kopie paměti) |

Úpravy se ukládají jako patche portu (`third_party/`), u programů s
konfigurací často stačí volba (`--disable-fork`, `HAVE_FORK=0`); FFmpeg,
sqlite, zlib a SDL `fork` nepotřebují.
- **Locale a wchar:** hotové, viz [Široké znaky a locale](#široké-znaky-a-locale)
  (`C` jednobajtové, `C.UTF-8` a ostatní jména UTF-8, kategorie zatím
  nelokalizované; `wchar_t` 32 bitů, plné `wchar.h` a `wctype.h` včetně
  širokých proudů).

### Rozhodnutí

Schváleno 2026-11-14: otázky 1-7 a 9-11 podle doporučení (A), `fork`
nebude ani omezený. **Otázka 8 = B:** skutečné zámky ve fsd už v této
práci - nová zpráva fsproto pro zámky rozsahu bajtů (`fcntl` `F_SETLK`,
`F_SETLKW`, `F_GETLK`) a `flock`; vlastník = proces (relace), uvolnění při
konci procesu nebo zavření posledního fd, čekání s termínem, uváznutí
(aspoň přímý cyklus) = `EDEADLK`. sqlite běží s výchozím unixovým VFS.
Změna fsd a fsproto je samostatný commit s host testy `libs/fs` a testem
dvou procesů nad jednou databází v QEMU.

Původní znění otázek s variantami:

1. **Obsah libc.so.** A **(doporučeno)** jeden `cdylib` `cstd + std +
   osrt` (jeden runtime, žádný nový kód; ~1,5 MB, kód sdílený přes libd).
   B cstd přepsat nad `osrt` bez `std` (menší `libc.so`, ale druhá
   implementace stdio a souborů vedle std). C jen statická `libc.a`,
   dynamické až s FFmpegem.
2. **Zavaděč a libc.** Původní návrh DL ([dynamicke-linkovani.md](dynamicke-linkovani.md))
   počítal s `ld.so` „zároveň `libc.so` ve stylu musl“; s libc nad std to
   přehodnocuji. A **(doporučeno)** `ld.so` zůstane samostatný
   a malý (Rust, dnešní stav, `dl*` vestavěné), `libc.so` je běžná
   knihovna z `DT_NEEDED`. B spojit `ld.so` a `libc.so` do jednoho souboru
   jako musl (rychlejší start, ale std v zavaděči a větší zavaděč).
3. **`math.h`.** A **(doporučeno)** crate `libm` (Rust, MIT/Apache, port
   musl libm, stejná přesnost, bez překladu C; volá ho i
   `compiler-builtins`). B musl libm v C přes `cbuild`. C openlibm
   (MIT/BSD/ISC, víc funkcí pro `long double`, větší).
4. **`posix_spawn` a fd.** A **(doporučeno)** potomek dostane jen fd 0, 1, 2
   (blok parametrů je umí), ostatní akce souborů `ENOTSUP`. B rozšířit
   blok parametrů o libovolné fd (nový štítek, změna `abi::params`).
5. **Zrušení vlákna.** A **(doporučeno)** jen odložené zrušení
   (`pthread_cancel` + body zrušení v našich blokujících funkcích),
   asynchronní ne. B `pthread_cancel` = `ENOTSUP`. C i asynchronní přes
   `THREAD_SUSPEND` a přesměrování PC.
6. **Asynchronní signály.** A **(doporučeno teď)** obsluha běží ve
   zvláštním vlákně signálů (jako obsluha Ctrl+C konzole ve Windows);
   doručení přes událost v bloku parametrů (`SIGNAL`), kterou drží rodič,
   termd nebo launchd; `kill` jen na potomky, `EINTR` nevzniká.
   B emulace v cstd: vlákno signálů pozastaví cílové vlákno
   (`THREAD_SUSPEND`) a přesměruje ho do obsluhy; blokující funkce cstd
   čekají i na událost signálu a vrací `EINTR`. C signály v jádře (nový
   syscall, přerušitelné syscally s `EINTR`).
7. **`epoll`.** A **(doporučeno)** `poll` a `select` teď, `epoll_*` (API
   Linuxu, nad portem s jednorázovou registrací) hned v části 5, protože
   ho chtějí libuv a mio. B jen `poll` a `select`.
8. **Zámky souborů** (sqlite, `fcntl(F_SETLK)`, `flock`). A **(doporučeno)**
   zámky v procesu (sqlite v jednom procesu funguje, mezi procesy
   nechráněno, výslovně v docs) a skutečné zámky až s fsd. B zámky ve fsd
   teď (nová zpráva fsproto, mimo tuto větev). C `ENOLCK` a sqlite bez
   zámků (`unix-none`).
9. **`long double`.** A **(doporučeno)** ABI jako Linux: x86_64 80 bitů,
   aarch64 128 bitů; funkce `*l` zatím počítají v `double` (přesnost
   `double`, výslovně v docs). B `long double` = `double` na obou
   (`-mlong-double-64`, jiné ABI než Linux, převzatý kód s `long double`
   to snese). C plná přesnost (openlibm, softwarová 128bitová aritmetika
   na aarch64).
10. **Rozsah portů v této větvi.** A **(doporučeno)** zlib a sqlite
    s testy v QEMU; SDL a FFmpeg samostatné větve. B i minimální FFmpeg
    (`--disable-everything`, pár dekodérů) jako test `libc.so`.
11. **`vfork`.** A **(doporučeno)** -1 s `ENOSYS` jako `fork`. B `vfork`
    vrací chybu, ale `vfork` + `exec*` v jednom makru převede hlavička na
    `posix_spawn` (jen jednoduchý vzor bez kódu mezi nimi, křehké).

### Jádro: AVX a XSAVE

**[ověřeno v QEMU]** Jádro ukládá stav FPU/SIMD přes XSAVE (x87, SSE, AVX
a AVX-512 podle CPUID, bez XSAVE FXSAVE), viz [Jádro](jadro.md). Kód v C
se překládá pro základní úroveň CPU (x86-64-v2, NEON,
[pozadavky-hw.md](pozadavky-hw.md)); AVX/AVX2 a výš volí knihovny
(FFmpeg, kodeky) za běhu podle CPUID.

## SDL (krok 6)

**[ověřeno v QEMU]** x86_64 i aarch64 (sada `sdl`, [SDL](sdl.md)). Co port
SDL potřeboval a doplnilo se:

| Co | Jak |
|---|---|
| `environ` | pole `JMENO=HODNOTA` zakončené NULL v `osrt` (`env.rs`); přestaví se při startu a při každém `setenv`/`unsetenv` (i z Rustu `std::env::set_var`). Stará pole se neuvolňují (ukazatele na ně mohl vzít jiný kód, jako v glibc) |
| `nl_langinfo` (`langinfo.h`) | texty locale C (formáty data a času, dny, měsíce); `CODESET` je `UTF-8` v `C.UTF-8`, jinak `ANSI_X3.4-1968` |
| `sched.h`, `pthread_getschedparam`/`setschedparam` | jen `SCHED_OTHER` s prioritou 0; `SCHED_FIFO`/`SCHED_RR` = `EPERM` (jako Linux bez oprávnění), nesmysl = `EINVAL` |
| `fenv.h` | výjimky, zaokrouhlení, `fegetenv`/`feholdexcept`/`fesetenv`/`feupdateenv`, `FE_DFL_ENV`; x86_64 nad x87 a MXCSR, aarch64 nad FPCR a FPSR (`src/fenv.rs`), hodnoty a `fenv_t` jako glibc. Pasti výjimek se nezapínají. Test: krok `math` (`ctest`) na obou architekturách, host test na x86_64 (SDL převádí zvuk přes NEON s `feholdexcept`) |
| `SIGVTALRM`, `SIGPROF` | čísla jako v Linuxu (26, 27) pro masky signálů; časovače `setitimer` zatím nejsou |
| `wchar.h` | úplné, viz [Široké znaky a locale](#široké-znaky-a-locale) |
| `inttypes.h` | makra `PRI*`/`SCN*` (LP64), `imaxabs`, `imaxdiv`, `strtoimax`, `strtoumax` |
| `libm.a`, `libpthread.a`, `librt.a` | prázdné archivy v sysrootu (vše je v `libc.a`, jako musl); build systémy linkují `-lm -lpthread` |

`dlfcn.h` přinese `libc.so` (samostatná větev); SDL ho zatím nepotřebuje
(GTK a `dlopen` záplata pro MagnumOpus vypíná).

## Široké znaky a locale

**[ověřeno v QEMU]** x86_64 i aarch64 (krok `wchar` testu `ctest`, sada
`cstd`; host testy `cargo test -p cstd`, `-p cstd-core`, `-p uniprop`).
Podklad pro libc++ (krok 7a-2, běhové prostředí C++): `wchar.h`
a `wctype.h` úplně, `locale.h` s objekty `locale_t` a varianty `*_l`.

### Kódování

| Locale | Kódování | `MB_CUR_MAX` | `CODESET` |
|---|---|---|---|
| `C`, `POSIX` (výchozí při startu) | jednobajtové: 0x00-0x7F ASCII, bajty 0x80-0xFF se v širokých znacích vratně mapují na U+DF80-U+DFFF (jako musl) | 1 | `ANSI_X3.4-1968` |
| `C.UTF-8`, `""` a každé jiné jméno bez cizího kódování (`cs_CZ.UTF-8`, `en_US`, `de_DE@euro`) | UTF-8 (jen platné: bez příliš dlouhých zápisů a náhradních polovin) | 4 | `UTF-8` |
| jméno s jiným kódováním (`cs_CZ.ISO-8859-2`) | odmítne se (`setlocale` vrátí `NULL`, `newlocale` `ENOENT`) | - | - |

- Proč jednobajtové `C`: program v locale C (výchozí) může libovolná data
  projít přes široké znaky bez ztráty a bez chyby; to POSIX vyžaduje
  a dělá to i musl. Programy, které chtějí UTF-8, volají
  `setlocale(LC_ALL, "")` jako všude jinde.
- `""` čte `LC_ALL`, `LC_<kategorie>`, `LANG`; nic nenastaveno = `C.UTF-8`
  (systém je UTF-8).
- Kategorie se **zatím nelokalizují**: čísla, datum a řazení jsou všude
  jako v locale C (desetinná tečka, anglické názvy měsíců, `strcoll` =
  `strcmp`). Kódování určuje jen `LC_CTYPE`; ostatní kategorie si jméno
  pamatují (`setlocale(LC_ALL, NULL)` vrátí složené jméno
  `C.UTF-8;C;C;C;C;C`, které jde předat zpět). České řazení přijde
  později.
- Locale vlákna: `uselocale` (jinak platí globální ze `setlocale`),
  `newlocale`/`duplocale`/`freelocale`. Jednotná locale (`C`, `C.UTF-8`)
  jsou sdílená bez alokace. `LC_GLOBAL_LOCALE` = `(locale_t)-1`.

### Funkce

| Hlavička | Funkce | Poznámka |
|---|---|---|
| `wchar.h` převody | `mbrtowc`, `mbrlen`, `mbsinit`, `wcrtomb`, `btowc`, `wctob`, `mbsrtowcs`, `mbsnrtowcs`, `wcsrtombs`, `wcsnrtombs` | stav `mbstate_t` (8 B, nuly = počátek); nulový ukazatel na stav = stav vlákna |
| `stdlib.h` | `mblen`, `mbtowc`, `wctomb`, `mbstowcs`, `wcstombs`, `MB_CUR_MAX` (`__ctype_get_mb_cur_max`) | `MB_LEN_MAX` 4 (`limits.h`) |
| `wchar.h` řetězce | `wcslen`, `wcsnlen`, `wcscpy`, `wcpcpy`, `wcsncpy`, `wcpncpy`, `wcscat`, `wcsncat`, `wcslcpy`, `wcslcat`, `wcscmp`, `wcsncmp`, `wcscasecmp`, `wcsncasecmp`, `wcscoll`, `wcsxfrm`, `wcschr`, `wcsrchr`, `wcsspn`, `wcscspn`, `wcspbrk`, `wcsstr`, `wcstok`, `wcsdup`, `wmem*`, `wcwidth`, `wcswidth` | porovnání podle hodnot `wchar_t`; `wcscasecmp` přes jednoduchý převod Unicode |
| `wchar.h` čísla a čas | `wcstol`, `wcstoll`, `wcstoul`, `wcstoull`, `wcstoimax`, `wcstoumax`, `wcstod`, `wcstof`, `wcstold`, `wcsftime` | úvodní bílé znaky podle `iswspace`, číslice jen ASCII; `ERANGE` |
| `wchar.h` proudy | `fgetwc`, `getwc`, `getwchar`, `fputwc`, `putwc`, `putwchar`, `ungetwc`, `fgetws`, `fputws`, `fwide` | v proudu kódování locale vlákna; rozpracovaný znak si pamatuje `FILE` |
| `wchar.h` formát | `wprintf`, `fwprintf`, `swprintf`, `vwprintf`, `vfwprintf`, `vswprintf`, `wscanf`, `fwscanf`, `swscanf` a `v`-varianty | stejný stroj jako `printf`/`scanf`; `swprintf` vrátí -1, když se výstup nevejde |
| `wctype.h` | `isw*`, `towupper`, `towlower`, `wctype`, `iswctype`, `wctrans`, `towctrans` | podle Unicode, nezávisí na locale; `iswdigit`/`iswxdigit` jen ASCII |
| `locale.h` | `setlocale`, `localeconv`, `newlocale`, `uselocale`, `duplocale`, `freelocale`, `LC_*_MASK` | viz výše |
| `*_l` | `is*_l`, `toupper_l`, `tolower_l`, `isw*_l`, `towupper_l`, `towlower_l`, `iswctype_l`, `wctype_l`, `wctrans_l`, `towctrans_l`, `strcoll_l`, `strxfrm_l`, `wcscoll_l`, `wcsxfrm_l`, `strftime_l`, `wcsftime_l`, `strto{d,f,ld}_l`, `strto{l,ll,ul,ull}_l`, `nl_langinfo_l` | libc++ je volá místo `uselocale` |
| `printf` | `%lc`, `%ls`, `%C`, `%S`; `asprintf`, `vasprintf` (buffer z `malloc`) | převod do kódování locale; znak mimo kódování = -1 a `EILSEQ`; přesnost v bajtech, nikdy půl znaku |
| `scanf` | `%lc`, `%ls`, `%l[` | v `scanf` převod z kódování locale, v `wscanf` naopak `%c %s %[` do vícebajtového |

### Tabulky Unicode

Třídy znaků (`iswalpha` = vlastnost Alphabetic, `iswupper`/`iswlower`
= Uppercase/Lowercase, `iswspace` = White_Space bez mezer bez zalomení
U+00A0, U+2007, U+202F), jednoduché převody velikosti (pole 12 a 13
`UnicodeData.txt`) a šířka znaku v terminálu (`wcwidth`: 2 pro
East_Asian_Width W/F, 0 pro kombinující znaménka a formátovací znaky,
-1 pro netisknutelné) jsou v crate `libs/uniprop` (`no_std`, použije ho
i terminál). Tabulky (`src/tables.rs`, seřazené rozsahy, binární hledání,
ASCII rychlou cestou) generuje `cargo xtask unicode-tables` z UCD ve verzi
`core::char::UNICODE_VERSION` překladače (dnes 18.0.0); soubory se
stahují přes HTTPS s kontrolou SHA-256. Host test hlídá, že verze sedí
a že `iswalpha`, `iswupper`, `iswlower` a převody velikosti dávají totéž
co `char` v Rustu pro všech 1,1 milionu kódových bodů. Data UCD jsou pod
licencí Unicode-3.0 (soubor `THIRD-PARTY.md` v kořeni repozitáře).

### Vnitřní stavba

- `cstd-core/src/mb.rs`: automat UTF-8 a jednobajtové kódování (`Enc`),
  `mbstate_t`. Formátovací stroj (`format.rs`) je společný pro bajty
  i široké znaky (`Out<u8>`/`Out<u32>`); kódování locale zjistí přes
  háček, který `cstd` zaregistruje při první změně locale (do té doby
  platí C, i ve službách `no_std`).
- `cstd/src/locale/`: stav locale (globální maska + locale vlákna v TLS),
  `nl_langinfo`, varianty `*_l` (`xl.rs`).
- `cstd/src/wchar/`: `conv.rs`, `wcs.rs`, `num.rs`, `wctype.rs`,
  `wio.rs`, `wprintf.rs`, `wscanf.rs`. Motor `scanf` pracuje se znaky
  `u32` v bajtovém i širokém režimu.
- `long double` (`wcstold`, `strtold_l`) je v C (`c/math_ld.c`) s přesností
  `double` jako `strtold`.

## Doplněk POSIX pro porty a Windows vrstvu

**[ověřeno v QEMU]** x86_64 i aarch64 (krok `posix2` testu `ctest`, sada
`cstd`; host testy `posix_tests.rs` a `spawn_tests.rs` v `cargo test -p
cstd`). Funkce, které chtějí porty a Wine
nad tím, co už knihovna měla (Windows vrstva = krok 8), a `termios.h`
(viz [termios](#termios)).

### Procesy: `posix_spawn` a `waitpid`

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="posix_spawn bez fork">
<defs><marker id="cs9" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="90" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">rodič: tabulka fd</text><text class="m" x="125" y="54" text-anchor="middle">0 stdin, 1 stdout, 2 stderr</text><text class="m" x="125" y="72" text-anchor="middle">5, 6 roura, 7 soubor ...</text><text class="m" x="125" y="90" text-anchor="middle">akce souborů (adddup2 ...)</text>
<path class="ln" d="M240 55 L288 55" marker-end="url(#cs9)"/>
<rect class="acc" x="290" y="10" width="200" height="90" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">plán (cstd)</text><text class="m" x="390" y="54" text-anchor="middle">jen fd 0-2: kanál proudu,</text><text class="m" x="390" y="72" text-anchor="middle">/dev/null nebo nic;</text><text class="m" x="390" y="90" text-anchor="middle">jinak ENOTSUP</text>
<path class="ln" d="M490 55 L538 55" marker-end="url(#cs9)"/>
<rect class="box" x="540" y="10" width="230" height="90" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">launchd</text><text class="m" x="655" y="54" text-anchor="middle">program, manifest, job,</text><text class="m" x="655" y="72" text-anchor="middle">relace FS a sítě</text><text class="m" x="655" y="90" text-anchor="middle">(jako std::process)</text>
<path class="ln" d="M655 100 L655 148" marker-end="url(#cs9)"/>
<rect class="ok" x="540" y="150" width="230" height="90" rx="6"/><text class="t b" x="655" y="174" text-anchor="middle">potomek</text><text class="m" x="655" y="194" text-anchor="middle">nový proces v podřízeném jobu,</text><text class="m" x="655" y="212" text-anchor="middle">fd 0-2 z bloku parametrů,</text><text class="m" x="655" y="230" text-anchor="middle">výchozí signály</text>
<path class="ln" d="M538 195 L242 195" marker-end="url(#cs9)"/>
<rect class="box" x="10" y="150" width="230" height="90" rx="6"/><text class="t b" x="125" y="174" text-anchor="middle">rodič: tabulka potomků</text><text class="m" x="125" y="194" text-anchor="middle">pid_t z cstd -> handle procesu</text><text class="m" x="125" y="212" text-anchor="middle">waitpid, wait4, waitid,</text><text class="m" x="125" y="230" text-anchor="middle">kill (ukončení)</text>
</svg>
<figcaption>posix_spawn bez fork: akce souborů se vyhodnotí v rodiči, potomka spustí launchd.</figcaption>
</figure>

`fork` v systému není (rozhodnutí uživatele, viz [Procesy, locale,
wchar](#procesy-locale-wchar)). `posix_spawn` a `posix_spawnp` spouští
potomka přes launchd stejně jako `std::process::Command`
([Rust std](rust-std.md)): podřízený job, nová relace FS se stejnou
identitou, relace sítě a tři standardní proudy. Ostatní fd potomek nemá (rozhodnutí 4A), proto se
akce souborů vyhodnotí už v rodiči:

| Akce | Výsledek |
|---|---|
| `addclose(0-2)` | proud se nepředá (vstup prázdný, výstup zahozený) |
| `addclose(> 2)` | nic (potomek vyšší čísla nemá) |
| `adddup2(x, 0-2)` | proud potomka = popis za `x` v rodiči (stdio, i jiný proud po `dup2`, roura nebo soubor) nebo 0-2 po předchozích akcích; socket `ENOTSUP` |
| `addopen(0-2, "/dev/null")` | prázdný vstup / zahozený výstup |
| `addopen(0-2, soubor)` | rodič soubor otevře (cesta vůči adresáři potomka v tom kroku, příznaky `open`) a předá ho jako proud |
| cokoli s cílem > 2 | `ENOTSUP` (blok parametrů předává jen tři proudy) |

**Soubor jako proud** (zpráva fsd `OPEN_STREAM`, [souborový systém](souborovy-system.md#protokol-fsproto)):
blok parametrů nese jen kanály proudu bajtů, proto fsd soubor
„přehrává“ do kanálu (stdin: od aktuální pozice do konce, pak konec
proudu) nebo z kanálu zapisuje (stdout/stderr: od aktuální pozice, u
`O_APPEND` na konec). fsd si otevře vlastní handle téhož souboru, takže
proud přežije zavření fd i konec rodiče. Rozdíl proti POSIX: potomek má
**vlastní pozici** v souboru (sdílená pozice popisu otevření mezi
procesy není); zápis potomka je v souboru, jakmile ho fsd zpracuje (po
skončení potomka chvíli, ne nutně hned). Restart fsd proud ukončí.
| `addchdir(_np)`, `addfchdir(_np)` | pracovní adresář potomka (relativní vůči předchozímu); neexistující = chyba `posix_spawn` |
| `addclosefrom_np(n)` | zavře 0-2 od `n` |

- **Síť potomka:** jako POSIX se dědí, má-li ji rodič (jinak než
  `std::process::Command`, kde jen s `network(true)`). Aplikace s vlastním
  manifestem ale dostane síť jen podle svého manifestu: rozhoduje launchd,
  ne rodič (průnik relace rodiče a `[network]` manifestu, manifest bez
  `[network]` = žádná síť). Pomocný program bez manifestu dědí relaci
  sítě rodiče beze změny.
- **Program:** cesta s `/` (relativní vůči adresáři potomka). U
  `posix_spawn` holý název = soubor v adresáři potomka, existuje-li;
  `posix_spawnp` hledá v `PATH` (bez proměnné `/system/bin`). Nenalezený
  holý název dostane launchd jako program systému (registr příkazů,
  `/system/bin`, initfs). Chyba launchd se vrátí jako kód chyby
  (`ENOENT`, `EACCES`, bez kanálu ke launchd `ENOSYS`).
- **Atributy:** `SETSID`, `SETPGROUP` se skupinou 0, `SETSIGDEF`,
  `SETSIGMASK`, `RESETIDS` se přijmou (potomek je vždy nový proces
  v novém jobu s výchozími obsluhami a prázdnou maskou signálů; uložená
  maska se nepředává). Skupina jiná než 0 = `ENOTSUP`, plánování jiné než
  `SCHED_OTHER` nebo priorita jiná než 0 = `EPERM`.
- **`pid_t` potomka** přiděluje `cstd` (tabulka v procesu, čísla od
  `getpid() + 1`); jiné procesy je neznají.
- **`waitpid`, `wait`, `wait3`, `wait4`, `waitid`:** `WNOHANG`, `WNOWAIT`;
  skupiny procesů nejsou, `waitpid(0)` a `waitpid(-pgid)` čekají jako
  `-1` na kteréhokoli potomka. Stav jako Linux: `WEXITSTATUS` = spodních
  8 bitů kódu konce; konec, který způsobil systém, se hlásí jako signál
  (`PROCESS_KILL` = `SIGKILL`, nebo signál z `kill`, výjimka CPU =
  `SIGSEGV`/`SIGILL`/`SIGFPE`/`SIGTRAP`/`SIGBUS`, panika = `SIGABRT`).
  `struct rusage` je vynulovaná (čas CPU jádro zatím nedává). Čekání na
  více potomků jde přes `OBJECT_WAIT_MANY` (nejvýš 64 najednou) v kolech
  po 50 ms, aby se do čekání dostal i potomek spuštěný jiným vláknem.
- **`kill` na potomka:** `SIGKILL`, `SIGTERM`, `SIGINT`, `SIGHUP`,
  `SIGQUIT` potomka ukončí hned (`PROCESS_KILL`, jeho obsluha signálu
  neproběhne), signál 0 jen ověří existenci. Doručení ostatních signálů
  do obsluhy potomka zatím nejde (`ENOTSUP`); `SIGCHLD` rodič nedostává.

### Semafory

`semaphore.h`: nepojmenované semafory (`sem_init`, `sem_wait`,
`sem_trywait`, `sem_timedwait`, `sem_clockwait`, `sem_post`,
`sem_getvalue`, `sem_destroy`) nad futexem, `sem_t` 32 B jako Linux.
Čekání je bod zrušení vlákna. `pshared` = 1 se přijme, semafor ale
funguje jen mezi vlákny procesu: systém nemá sdílenou paměť mezi procesy
(`MAP_SHARED` se zápisem je `ENOTSUP`) a futex v jádře je v adresovém
prostoru procesu. Pojmenované semafory (`sem_open`, `sem_close`,
`sem_unlink`) proto vrací `SEM_FAILED` / -1 s `ENOSYS`; skutečné přijdou
se sdílenou pamětí mezi procesy (samostatný úkol: `shm_open` a futex mezi
procesy).

### Vlákna, soubory, čas

- **`pthread_attr_setstack`:** vlákno opravdu běží v paměti volajícího
  (`libos::thread::spawn_on_stack`); paměť musí žít do konce vlákna
  a ochrannou oblast pod ní si zajistí volající (Wine si zásobníky vláken
  přiděluje sám). Dále `getstack`, `set/getguardsize` (bez vlastního
  zásobníku má vlákno vždy ochrannou oblast `libos`), `set/getscope` (jen
  `PTHREAD_SCOPE_SYSTEM`, rozsah procesu `ENOTSUP`), `set/getinheritsched`,
  `set/getschedpolicy` a `set/getschedparam` (jen `SCHED_OTHER`, priorita 0).
- **`fchdir`:** adresář otevřený jako fd; cesta je z doby otevření
  (`osrt::fs::File::path`), po přejmenování adresáře zastará.
- **`statfs`, `fstatfs`** (`sys/statfs.h`, `sys/vfs.h`, rozložení jako
  Linux na 64 bitech; `statvfs` přináší větev C++ 7a se stejným zdrojem):
  velikost bloku, bloky celkem a volné bloky svazku relace FS (zpráva fsd
  `INFO`, bloky čekající na uvolnění se nepočítají); všechny cesty
  procesu jsou na jednom svazku, cesta nebo fd jen ověří platnost.
  `f_type` = `MOFS_SUPER_MAGIC` (0x4D4F4653), počty souborů 0 (FS je nemá
  pevně dané), `ST_RDONLY` se nenastavuje (práva jsou podle ACL souboru).
- **`times`** (`sys/times.h`): uplynulý čas v tiku `sysconf(_SC_CLK_TCK)`
  = 100; čas CPU jádro zatím nedává, pole `tms` jsou 0 (jako `clock()`
  = -1).
- **`asctime`, `asctime_r`, `ctime`, `ctime_r`:** formát normy C
  (`"Sun Sep 16 01:03:52 1973\n"`), rok nad 9999 = `EOVERFLOW`; buffer
  `asctime`/`ctime` je per vlákno.
- **`ffs`, `ffsl`, `ffsll`** (`strings.h`).
- **Relace a skupiny procesů nejsou:** proces vede vlastní relaci
  i skupinu (číslo = `getpid()`); `setsid`, `getsid`, `getpgid`, `getpgrp`
  ho vrací, `setpgid` jen „zůstat ve vlastní skupině“, jinak `EPERM`.
- **`putenv`** (kopie řetězce: pozdější změna se do prostředí
  neprojeví, jinak než glibc; bez `=` proměnnou smaže), **`gethostname`**
  (= `uname().nodename`, `sethostname` = `EPERM`), typy BSD `caddr_t`,
  `u_char` ... (`sys/types.h`).
- **`_exit`** (`unistd.h`, = `_Exit`), `select` i přes `sys/time.h` (jako glibc), čísla signálů
  `SIGIO`, `SIGXCPU`, `SIGXFSZ`, `SIGPWR`, `SIGSYS`, `SIGSTKFLT` (cstd je
  samo nevytváří) a bity `S_ISUID`, `S_ISGID`, `S_ISVTX`.
- **`errno`:** úplná tabulka Linuxu (`asm-generic`, čísla 1-133 bez děr
  41 a 58, stejná na obou architekturách) s texty `strerror`, aliasy
  `EWOULDBLOCK`, `EDEADLOCK`, `EOPNOTSUPP`.

### Síť

- **Konstanty** jako Linux: `IPPROTO_*` (`netinet/in.h`), volby `IP_*`
  a `IPV6_*`, `struct ip_mreq`, `ip_mreqn`, `ipv6_mreq`, `in_pktinfo`,
  `in6_pktinfo`, makra `IN6_IS_ADDR_*`, `INADDR_*`; `SO_*`, `MSG_*`
  (`sys/socket.h`), `TCP_*` (`netinet/tcp.h`). Konstanta neznamená
  podporu: `setsockopt` přijme jen časové limity a volby bez účinku
  (`SO_REUSEADDR`, `SO_KEEPALIVE`, velikosti bufferů, `SO_LINGER`,
  `TCP_NODELAY`), ostatní `ENOPROTOOPT`. `getsockopt` navíc `SO_DOMAIN`,
  `SO_PROTOCOL` a `IP_TTL` (64).
- **`SOCK_RAW` a `AF_PACKET` = `EPERM`** (jako Linux bez `CAP_NET_RAW`):
  surové pakety a rámce má jen netd; `AF_NETLINK` = `EAFNOSUPPORT`.
- **Řídicí zprávy:** makra `CMSG_FIRSTHDR`, `CMSG_NXTHDR`, `CMSG_DATA`,
  `CMSG_SPACE`, `CMSG_LEN`, `CMSG_ALIGN` a `struct cmsghdr` jako Linux.
  `sendmsg` s řídicí zprávou vrátí `EOPNOTSUPP`: `SCM_RIGHTS` (předání
  fd) jde v systému jinak (handly v kanálech, Wine má vlastní transport),
  ostatní (`IP_PKTINFO` ...) netd nepřenáší. `recvmsg` žádné nevrací
  (`msg_controllen` = 0).
- **`getifaddrs`, `freeifaddrs`** (`ifaddrs.h`) a **`if_nameindex`,
  `if_freenameindex`, `if_nametoindex`, `if_indextoname`** (`net/if.h`):
  smyčka `lo` (index 1; 127.0.0.1/8 a ::1/128) a rozhraní sítě `eth0`
  (index 2) ze stavu netd (`INFO`: adresa, maska z prefixu, broadcast
  IPv4, `IFF_UP`/`IFF_RUNNING` podle linky) - jen s relací sítě
  s právem `INFO`. Rozhraní bez adresy má `ifa_addr` = NULL; položky
  `AF_PACKET` (MAC) nejsou.
- **`ioctl` `SIOCGIF*` na socketu** (`struct ifreq`, `struct ifconf`,
  `net/if.h`): `SIOCGIFFLAGS`, `SIOCGIFMTU` (smyčka 65536), `SIOCGIFINDEX`,
  `SIOCGIFNAME`, `SIOCGIFADDR`, `SIOCGIFNETMASK`, `SIOCGIFBRDADDR` (jen
  IPv4), `SIOCGIFHWADDR` (MAC, smyčka `ARPHRD_LOOPBACK`), `SIOCGIFCONF`
  nad týmiž rozhraními; nastavování (`SIOCSIF*`) = `ENOTTY`.
- **Hlavičky pro překlad portů** (bez nové funkčnosti): `sys/un.h`
  (`struct sockaddr_un`; sockety `AF_UNIX` zatím nejsou), `struct ucred`
  (`SO_PEERCRED`), `struct tcp_info` a stavy `TCP_*` (`getsockopt(TCP_INFO)`
  = `ENOPROTOOPT`), `net/if_arp.h` (`ARPHRD_*`, `struct arphdr`),
  `net/ethernet.h` (`ETH_ALEN`, `struct ether_header`); `ioctl(SIOCATMARK)`
  na socketu vrací 0 (data mimo pořadí netd nepřenáší).
- **`gethostbyname`, `gethostbyaddr`** (`struct hostent`, `h_errno`,
  `hstrerror`): staré rozhraní vrací NULL s `h_errno` = `NO_RECOVERY`;
  programy mají `getaddrinfo`.

### termios

**[ověřeno v QEMU]** strana programu (krok `termios` testu `ctest` proti
testovacímu terminálu v initu, host testy `termios_tests.rs`
a `cargo test -p ttyproto`); strana okna terminálu (`shsess` spouští
externí programy) **[navrženo]**.

Proces spuštěný v terminálu dostane ve startovní zprávě vedle proudů
stdin/stdout/stderr i **řídicí kanál terminálu** (štítek
`abi::params::tag::TTY`, protokol `libs/ttyproto`). Data jdou dál proudy
bajtů; kanál nese jen řízení:

| op | Požadavek | Odpověď |
|---|---|---|
| `GET_MODE` 1 | - | režim (pole `struct termios`) |
| `SET_MODE` 2 | kdy (`TCSANOW`/`TCSADRAIN`/`TCSAFLUSH`), režim | režim, který terminál skutečně nastavil |
| `GET_SIZE` 3 | - | velikost okna (`struct winsize`) |
| `SET_SIZE` 4 | velikost | - (okno terminálu ji klientovi měnit nedovolí: `ACCESS_DENIED`) |
| `FLUSH` 5 | fronta | - |
| `DRAIN` 6 | - | - |
| `WATCH_SIZE` 7 | - | handle události, kterou terminál signalizuje při změně velikosti |

- **Řádková disciplína je na straně terminálu** (rozhodnutí A): editor
  řádku, echo a Ctrl+C (`SIGINT`) zůstávají v terminálu, který zná
  nativní protokol okna; VT sekvence jsou jen adaptér pro převzaté
  programy. V surovém režimu (`cfmakeraw`) posílá terminál bajty kláves
  (šipky jako VT sekvence). Nepodporované příznaky terminál v odpovědi
  `SET_MODE` vynuluje (zjistí je následné `tcgetattr`); linka je
  virtuální (vždy 8 bitů bez parity, rychlost se jen uchová).
- **cstd:** `tcgetattr`, `tcsetattr`, `cfmakeraw`, `cfget/setispeed`,
  `cfget/setospeed`, `cfsetspeed`, `tcflush`, `tcdrain`, `tcflow`
  a `tcsendbreak` (jen ověří terminál), `tcgetpgrp`/`tcsetpgrp`/`tcgetsid`
  (vlastní skupina procesu), `ioctl` `TCGETS`, `TCSETS(W/F)`, `TCFLSH`,
  `TIOCGWINSZ`, `TIOCSWINSZ`, `TIOCGPGRP`. Hodnoty a `struct termios`
  (60 B) jako glibc.
- **`isatty`** = fd 0-2 se stdio **a** proces má řídicí kanál terminálu;
  kanál proudu od initu nebo roura terminál není (stdout je pak plně
  bufferovaný jako na Linuxu). Jiný fd = `ENOTTY`.
- **`SIGWINCH`:** první `tcgetattr` nebo `TIOCGWINSZ` požádá o událost
  (`WATCH_SIZE`) a spustí vlákno, které po její signalizaci zavolá
  `raise(SIGWINCH)`; obsluha běží v tom vlákně jako u časovače
  (rozhodnutí 6A).

### Vnitřní stavba (doplněk)

- `cstd/src/spawn/mod.rs` akce souborů (`plan`), atributy, hledání
  programu; `spawn/wait.rs` tabulka potomků, `waitpid` a spol., `kill`
  na potomka; systémová část `sys/os_proc.rs` nad `osrt::process`
  (`wait_any`). Host testy mají náhradní potomky (`exit:<kód>`,
  `sleep:<ms>:<kód>`) v `sys/host.rs`.
- `cstd/src/termios.rs` (klient `ttyproto`, `ioctl` terminálu, vlákno
  `SIGWINCH`), `osrt::tty` (řídicí kanál ze startovní zprávy); host testy
  mají terminál v paměti (`sys/host.rs`, `ttyproto::Tty`).
- `cstd/src/semaphore.rs`, `pthread/attr.rs`, `statfs.rs`, `ifaddrs.rs`, `ifreq.rs`;
  `time.rs` (`asctime`, `times`), `unistd.rs` (`setsid`), `misc.rs` (`ffs`), `socket.rs`
  (`SOCK_RAW`, řídicí zprávy).
- `osrt`: `fs::volume_space`, `fs::File::path`, `net::interface`,
  `process::wait_any`, `thread::spawn_on`; `libos::thread::spawn_on_stack`.

## FFmpeg a licence

FFmpeg poběží jako **samostatný izolovaný proces** (služba přehrávání
a dekódování, viz [Architektura](architektura.md)); aplikace ho
nelinkují, mluví se službou. Licence:

- Překládá se jen pod **LGPL-2.1-or-later** - nikdy `--enable-gpl` ani
  `--enable-nonfree` (GPL a nesvobodné části nesmí do systému).
- Povinnost LGPL (možnost znovu slinkovat upravenou knihovnu) se týká jen
  binárky procesu FFmpeg, ne zbytku systému: se systémem dodáme zdroje
  použité verze FFmpegu, naše úpravy a skript sestavení, kterým si
  uživatel proces sestaví znovu s vlastní verzí knihovny.
- Později dynamické linkování (sdílená knihovna FFmpeg), až bude zavaděč
  sdílených knihoven; pak stačí vyměnit knihovnu.
- Záznam v `THIRD-PARTY.md` vznikne s prvním převzatým kódem (sekce
  FFmpeg je tam už teď s pravidly).
