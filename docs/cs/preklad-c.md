# Překlad kódu v C

Systém je v Rustu, ale převzatý kód v C se překládá a linkuje do programů
v userspace: první je ACPICA ([ACPI](acpi.md)), další budou ovladače DRM
z Linuxu (přes vrstvu kompatibility) a vlastní libc. Tahle stránka popisuje,
jak se C zapojuje do sestavení, na čem stojí (knihovna `cstd`) a jak se
hlídá jeho bezpečnost.

> **Stav:** hotový překlad (clang, `libs/cbuild`), základ libc (`libs/cstd`:
> řetězce, paměť, ctype, `printf` do logu, ochrana zásobníku) a první
> uživatel ACPICA. Soubory (`fopen`), vlákna a čas v libc zatím nejsou.

## Jak se C zapojí do sestavení

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Preklad kodu v C">
<defs><marker id="pc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="80" rx="6"/><text class="t b" x="110" y="45" text-anchor="middle">zdroje C</text><text class="m" x="110" y="64" text-anchor="middle">third_party/acpica</text><text class="m" x="110" y="80" text-anchor="middle">libs/acpica/c/glue.c</text>
<path class="ln" d="M210 60 L268 60" marker-end="url(#pc1)"/>
<rect class="acc" x="270" y="20" width="220" height="80" rx="6"/><text class="t b" x="380" y="45" text-anchor="middle">build.rs + cbuild</text><text class="m" x="380" y="64" text-anchor="middle">clang --target=..-none-elf</text><text class="m" x="380" y="80" text-anchor="middle">společné příznaky a ochrany</text>
<path class="ln" d="M490 60 L548 60" marker-end="url(#pc1)"/>
<rect class="box" x="550" y="20" width="220" height="80" rx="6"/><text class="t b" x="660" y="45" text-anchor="middle">libacpica.a</text><text class="m" x="660" y="64" text-anchor="middle">llvm-ar, v OUT_DIR</text><text class="m" x="660" y="80" text-anchor="middle">cargo:rustc-link-lib</text>
<path class="ln" d="M660 100 L660 150" marker-end="url(#pc1)"/>
<rect class="ok" x="430" y="152" width="340" height="80" rx="6"/><text class="t b" x="600" y="177" text-anchor="middle">program (rust-lld)</text><text class="m" x="600" y="196" text-anchor="middle">kód v Rustu + knihovna C + cstd</text><text class="m" x="600" y="212" text-anchor="middle">+ compiler_builtins (memcpy, memset)</text>
<rect class="box" x="10" y="152" width="380" height="80" rx="6"/><text class="t b" x="200" y="177" text-anchor="middle">libs/cstd (Rust, ABI C)</text><text class="m" x="200" y="196" text-anchor="middle">include/*.h: string, stdlib, ctype, stdio, assert</text><text class="m" x="200" y="212" text-anchor="middle">strlen, malloc, printf, __stack_chk_*</text>
<path class="ln" d="M390 192 L428 192" marker-end="url(#pc1)"/>
</svg>
<figcaption>C se překládá v build.rs crate, který ho obsahuje; výsledná statická knihovna se slinkuje s programem spolu s cstd.</figcaption>
</figure>

1. Crate s kódem v C (např. `libs/acpica`) má `build.rs`, který zavolá
   `cbuild::Build` se seznamem souborů a hlaviček.
2. `cbuild` najde clang (`CLANG`, `PATH`, `C:\Program Files\LLVM\bin`,
   `/usr/bin`), přeloží soubory paralelně s příznaky pro cíl Rustu
   (`TARGET`) a sbalí je `llvm-ar` do `lib<jméno>.a` v `OUT_DIR`.
3. Cargo knihovnu přilinkuje (`cargo:rustc-link-lib=static=...`) k programu,
   který crate používá. Linkuje rust-lld spolu s kódem v Rustu.
4. Funkce, které kód v C potřebuje od systému, dodá Rust s ABI C
   (`#[unsafe(no_mangle)] extern "C"`): knihovna C je `libs/cstd`, služby
   OS konkrétní program (ACPICA: `user/acpid/src/osl`).

Znovu se překládá jen to, co se změnilo (čas souboru proti `.o`); změna
příznaků, prekladače nebo kterékoli hlavičky = vše znovu. ACPICA (164
souborů) se na běžném PC přeloží za několik sekund.

### Nástroje

| Nástroj | Odkud | Poznámka |
|---|---|---|
| clang 17+ | LLVM (Windows: `winget install LLVM.LLVM`, Linux: balíček `clang`) | jeden clang umí oba cíle (`x86_64`, `aarch64`) |
| llvm-ar | stejná instalace LLVM | hledá se vedle clangu |

Chybějící clang = chyba buildu s návodem, jak ho nainstalovat.

### Přidání kódu v C (příklad)

```rust
// build.rs crate libs/foo
fn main() {
    cbuild::Build::new("foo")
        .trust(cbuild::Trust::Own)          // vlastní kód: varování = chyba
        .include("include")
        .include("../cstd/include")         // libc systému
        .file("c/foo.c")
        .compile();
}
```

```toml
[dependencies]
cstd = { path = "../cstd" }                # knihovna C se přilinkuje

[build-dependencies]
cbuild = { path = "../cbuild" }
```

Převzatý kód: `Trust::Vendored` (varování se nevypisují, kód se nemění)
a hlavičky převzatého kódu ve vlastním kódu přes `include_system`
(`-isystem`), aby se varování z nich nepočítala jako chyby.

## Příznaky překladu

| Příznak | Proč |
|---|---|
| `--target=x86_64-unknown-none-elf` / `aarch64-unknown-none-elf` | stejné ABI jako cíle Rustu (ELF, bez OS) |
| `-mgeneral-regs-only` | programy jsou soft-float (bez FPU/SIMD), stejně jako kód v Rustu |
| `-mno-red-zone`, `-mcmodel=small` (x86) | stejný model jako cíl Rustu |
| `-mstrict-align` (aarch64) | jako cíl Rustu; nezarovnaný přístup do registrů zařízení = výjimka |
| `-ffreestanding -nostdlibinc` | žádné hlavičky hostitele (Windows SDK, glibc), jen clangu a cstd |
| `-fno-pic -fno-common -ffunction-sections -fdata-sections` | statické programy, nepoužité funkce vyhodí linker |
| `-O2` | AML interpret a podobný kód je bez optimalizace mnohonásobně pomalejší |
| `-std=c11` | jazyk ACPICA i Linuxu |

## Jak se hlídá bezpečnost

Kód v C nemá kontroly paměti Rustu. Proto:

| Opatření | Co chrání |
|---|---|
| `-fstack-protector-strong`, `-mstack-protector-guard=global` | kanárek v každé funkci s polem na zásobníku; přetečení pole = `__stack_chk_fail` (cstd) ukončí proces dřív, než se skočí na podvrženou návratovou adresu. Kanárek je náhodný (`cstd::init_stack_guard`), nejnižší bajt nula (řetězcové funkce ho nepřepíšou "správně") |
| `-ftrivial-auto-var-init=zero` | neinicializované lokální proměnné jsou nuly (žádný únik starých dat, předvídatelné chování) |
| `-fno-strict-aliasing -fwrapv -fno-delete-null-pointer-checks` | překladač nevyhodí kontroly ukazatelů a přetečení podle "nedefinovaného chování" |
| `-Werror=implicit-function-declaration`, `incompatible-pointer-types`, `int-conversion`, `return-type` | volání funkce bez prototypu (useknutý ukazatel) a podobné chyby nejdou přeložit ani v převzatém kódu |
| vlastní kód: `-Wall -Wextra -Werror` | varování = chyba buildu |
| cstd bez nebezpečných funkcí | `sprintf`, `gets`, `strtok` nejsou; `%n` v printf nic nezapíše; obří šířka formátu je omezená |
| `_Static_assert` na rozložení struktur | změna verze převzatého kódu nemůže tiše rozbít rozhraní pro Rust |
| proces, ne jádro | C běží v userspace s omezeným prostředkem; pád = pád služby, jádro a RAM jiných procesů chrání |

Na zvážení dál: `-fsanitize=bounds -fsanitize-trap` (meze polí bez
běhové knihovny; ACPICA má pole proměnné délky deklarovaná jako `[1]`,
muselo by se ověřit), CFI (`-fsanitize=cfi` potřebuje LTO).

## Knihovna `cstd`

Základ vlastní libc, napsaný v Rustu (testovatelný na hostiteli:
`cargo test -p cstd`). Hlavičky jsou v `libs/cstd/include`; typy
`stddef.h`, `stdint.h`, `stdarg.h`, `stdbool.h` a `limits.h` dodává clang.

| Hlavička | Funkce |
|---|---|
| `string.h` | `strlen`, `strnlen`, `strcmp`, `strncmp`, `strcpy`, `strncpy`, `strcat`, `strncat`, `strchr`, `strrchr`, `strstr`, `memchr`; `memcpy`, `memmove`, `memset`, `memcmp` dodává `compiler_builtins` |
| `ctype.h` | `isdigit`, `isxdigit`, `isalpha`, `isalnum`, `isupper`, `islower`, `isspace`, `isprint`, `isgraph`, `iscntrl`, `ispunct`, `toupper`, `tolower` (ASCII) |
| `stdlib.h` | `malloc`, `calloc` (kontrola přetečení), `realloc`, `free`, `strtoul`, `strtoull`, `strtol`, `strtoll`, `atoi`, `abs`, `abort` |
| `stdio.h` | `printf`, `vprintf`, `snprintf`, `vsnprintf`, `puts`, `putchar`; výstup = log programu po řádcích |
| `assert.h` | `assert` (selhání ukončí proces se jménem souboru a řádkem) |

Program nastaví na začátku `main`, kam jde výstup, a náhodný kanárek –
**před** prvním voláním kódu v C:

```rust
cstd::set_output(|line| libos::println!("acpid: {}", core::str::from_utf8(line).unwrap_or("?")));
cstd::init_stack_guard(libos::random::u64().0);
```

`printf` podporuje příznaky `-+ #0`, šířku a přesnost (i `*`), délky
`hh h l ll z j t` a převody `d i u o x X c s p %`. Plovoucí čárka zatím
není (programy jsou bez FPU, vypíše se `?`).

### Plán: vlastní libc a přenos dalšího C

- **libc nad libos** (milník jazyků): soubory (`FILE`, `fopen` nad fsd),
  `errno`, čas, vlákna (`pthread` nad `libos::thread`), `qsort`, `bsearch`,
  plovoucí čárka. Staví se na stejných hlavičkách a stejném postupu
  (Rust s ABI C, hlavičky v `include`).
- **Ovladače DRM z Linuxu**: vrstva kompatibility (`linux/*.h` na
  primitiva systému) bude další crate s `build.rs` přes `cbuild`; ovladač
  poběží jako proces v userspace jako ostatní ovladače.

## Licence převzatého kódu

Převzatý kód má vlastní adresář v `third_party/` s README (verze, odkud,
co se vynechalo, zásahy) a licencí; přehled je v `THIRD-PARTY.md`
v kořeni repozitáře.
