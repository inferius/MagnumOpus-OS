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
> Části 2-8 (vlákna, fd, čas, sockety, `math.h`, `mmap`, signály, `libc.so`,
> porty) **[navrženo]**, viz [Pokračování](#pokračování-úplná-libc-a-posix-části-2-8).

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
| `stdlib.h` | `exit`, `_Exit`, `atexit`, `getenv`, `abort` | `EXIT_SUCCESS`, `EXIT_FAILURE` |
| `string.h` | `strerror` | zbytek v `cstd-core` |
| `assert.h` | `assert` | selhání vypíše soubor a řádek na stderr a ukončí proces |

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

Chyba `std::io::Error` se převádí na `errno` podle druhu chyby
(`NotFound` -> `ENOENT`, `PermissionDenied` -> `EACCES`, ...; neznámý druh
`EIO`). Neplatný režim `fopen` nebo jméno, které není UTF-8, = `EINVAL`.

## Omezení části 1

- **Plovoucí čárka v `printf`** chybí (`%f`, `%g`, `%e` vypíší `?`).
- Jen proudy `FILE`; čísla fd, vlákna, čas, sockety a `math.h` přináší
  pokračování níže.

## Pokračování: úplná libc a POSIX (části 2-8)

**[navrženo]**, k odsouhlasení (krok 2 v pořadí převzetí cizího softwaru,
[architektura.md](architektura.md#bezpečnost)). Cíl: přeložit beze změn
zdrojů zlib, sqlite, pak závislosti SDL a FFmpeg (LGPL, samostatný proces).
Volby označené **(doporučeno)** jsou návrh; konečné rozhodnutí je
v kapitole [Otevřené otázky](#otevřené-otázky).

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
   `chmod`/`chown` = `EPERM` (práva jsou granty FS, ne bity), `link`/
   `symlink` = `ENOTSUP` do FS v3, zámky `fcntl`/`flock` otázka 8.
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

- **`fork` nebude** (procesy jen přes `spawn`, rozhodnutí
  [rust-std.md](rust-std.md)). `fork` vrací `ENOSYS`. `posix_spawn`
  a `posix_spawnp` ano: aplikace s manifestem přes `launchd`, pomocné
  programy přes `spawn` (jako `std::process`), akce souborů pro fd 0-2
  (otázka 4), `waitpid` nad handlem procesu, `system`/`popen` přes shell
  systému.
- **Locale:** jen `C` (výchozí při startu, jak chce POSIX) a `C.UTF-8`.
  `setlocale(LC_ALL, "")` vybere `C.UTF-8`; jakékoli jméno `*.UTF-8`
  se přijme jako `C.UTF-8`. Desetinná tečka vždy `.`, texty a měsíce
  anglicky. Lokalizace aplikací není úkol libc.
- **wchar:** `wchar_t` 32 bitů (UTF-32) na obou architekturách,
  `MB_CUR_MAX` 1 v `C` a 4 v `C.UTF-8`. Převody (`mbrtowc`, `wcrtomb`,
  `mbstowcs`, ...), řetězce `wcs*`, `wctype.h` (třídy Unicode z tabulek),
  `swprintf`. Široké proudy (`fwprintf`, `fgetwc`) až když je bude
  potřebovat port.

### Otevřené otázky

1. **Obsah libc.so.** A **(doporučeno)** jeden `cdylib` `cstd + std +
   osrt` (jeden runtime, žádný nový kód; ~1,5 MB, kód sdílený přes libd).
   B cstd přepsat nad `osrt` bez `std` (menší `libc.so`, ale druhá
   implementace stdio a souborů vedle std). C jen statická `libc.a`,
   dynamické až s FFmpegem.
2. **Zavaděč a libc.** A **(doporučeno)** `ld.so` zůstane samostatný
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

### Jádro: AVX a XSAVE

**[ověřeno v QEMU]** Jádro ukládá stav FPU/SIMD přes XSAVE (x87, SSE, AVX
a AVX-512 podle CPUID, bez XSAVE FXSAVE), viz [Jádro](jadro.md). Kód v C
se překládá pro základní úroveň CPU (x86-64-v2, NEON,
[pozadavky-hw.md](pozadavky-hw.md)); AVX/AVX2 a výš volí knihovny
(FFmpeg, kodeky) za běhu podle CPUID.

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
