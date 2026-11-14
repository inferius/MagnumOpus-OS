# Knihovna C (cstd)

Vlastní knihovna C systému MagnumOpus, napsaná v Rustu s ABI C. Má dvě
části: **`cstd-core`** (základ bez systémových služeb pro služby `no_std`,
např. ACPICA v `acpid`) a **`cstd`** (vrstva POSIX nad [Rust std](rust-std.md)
pro aplikace). Program v C tak běží na stejném runtime jako program
v Rustu: jeden alokátor, jedno TLS, jedno stdio. Překlad C (clang,
příznaky, ochrany) popisuje [Překlad C](preklad-c.md).

> **Stav:** část 1 **[ověřeno v QEMU]** na x86_64 i aarch64 (test
> `ctest: stdio, soubory, errno, getenv, atexit`, sada `cstd`): `errno`,
> `FILE` (stdin/stdout/stderr, soubory), `exit`/`atexit`, `getenv`. Další
> části (vlákna, čísla fd, čas, `qsort`, plovoucí čárka v `printf`)
> **[navrženo]**.

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

## Omezení a plán

- **Plovoucí čárka v `printf`** chybí (`%f`, `%g`, `%e` vypíší `?`); doplní
  se do formátovacího stroje v `cstd-core` (jen pro hard-float cíle).
- **Část 2 [navrženo]:** `pthread` nad vlákny `osrt` a futexem (potřebuje
  rozšíření `std::os::magnumopus` o futex), tabulka čísel fd v `osrt`
  (`open`, `read`, `write`, `close`, sdílená s `std`), čas (`time`,
  `clock_gettime`), `qsort`, `bsearch`, `strtod`, `mmap` anonymní (po
  alokaci stránek na požádání), sockety.
- **Porty:** nejdřív zlib (malý, testy v balíku), později sqlite.

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
