# Běh C++

Programy a knihovny v C++ (HarfBuzz, později libass, Wine, Ladybird, LLVM)
běží nad **libc++**, **libc++abi** a **libunwind** z projektu LLVM ve
stejné verzi jako clang hostitele (23.1.5). Pod nimi je knihovna C
[cstd](cstd.md) a Rust std (jeden runtime procesu). Sestavení je port
v xtasku ([Porty v C](porty-c.md), `xtask/src/cports/cxx.rs`).

## Stav

| Část | Co | Stav |
|---|---|---|
| statický běh | `libc++.a`, `libc++abi.a`, `libunwind.a` v předponě portů, `clang++.cfg` | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `cpp`, test `cpptest`) |
| sdílený běh | `/system/lib/libc++.so` (libc++ + abi) a samostatná `/system/lib/libunwind.so`, program PIE přes zavaděč | **[ověřeno v QEMU]** x86_64 i aarch64 (test `cpptest-dyn`) |
| výjimky | Itanium ABI, tabulky DWARF `.eh_frame`, i přes rámce C (`-funwind-tables`) | **[ověřeno v QEMU]** x86_64 i aarch64 (`cpptest`: přes ramec C, `exception_ptr`, ve vláknech) |
| RTTI | `dynamic_cast`, `typeid`, `bad_cast` | **[ověřeno v QEMU]** x86_64 i aarch64 |
| vlákna | `std::thread`, `mutex`, `condition_variable` nad pthread z cstd; `thread_local` s destruktory (`__cxa_thread_atexit_impl`) | **[ověřeno v QEMU]** x86_64 i aarch64 |
| konec procesu | `__cxa_atexit` a `atexit` ve společném obráceném pořadí, `__cxa_finalize`, `__dso_handle` v každém modulu | **[ověřeno v QEMU]** x86_64 i aarch64 (destruktor globálního objektu po `main`) |
| `std::filesystem` | nad fsd: `*at` relativně k adresáři, `realpath`, `statvfs`, `sendfile` | **[ověřeno v QEMU]** x86_64 i aarch64 |
| `chrono`, `random_device` | `clock_gettime`, `getentropy` (generátor jádra) | **[ověřeno v QEMU]** x86_64 i aarch64 |
| HarfBuzz 14.2.0 | amalgamace bez výjimek a RTTI, volaná z C | **[ověřeno v QEMU]** x86_64 i aarch64 (test `hbtest`) |
| proudy, locale, široké znaky (7a-2) | `<iostream>`, `<fstream>`, `<sstream>`, `<locale>` (`C`, `C.UTF-8`), `std::wstring`, `codecvt`, `<format>`, `<regex>` nad funkcemi wchar a locale v cstd | **[ověřeno v QEMU]** x86_64 i aarch64 (`cpptest`: `iostream`, `proudy`, `locale-siroke-znaky`, `format-regex`) |
| vybrané testy `libcxx/test` | dávky testů libc++ v QEMU | **[navrženo]** |

## Vrstvy

<figure class="diagram">
<svg viewBox="0 0 780 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Vrstvy běhu C++">
<defs><marker id="cp1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="760" height="40" rx="6"/><text class="t b" x="390" y="35" text-anchor="middle">program v C++ (STL, výjimky, vlákna, filesystem)</text>
<rect class="acc" x="10" y="70" width="370" height="50" rx="6"/><text class="t b" x="195" y="92" text-anchor="middle">libc++</text><text class="m" x="195" y="110" text-anchor="middle">kontejnery, vlákna, chrono, filesystem</text>
<rect class="acc" x="400" y="70" width="370" height="50" rx="6"/><text class="t b" x="585" y="92" text-anchor="middle">libc++abi</text><text class="m" x="585" y="110" text-anchor="middle">new/delete, RTTI, __cxa_throw, guard</text>
<rect class="acc" x="400" y="140" width="370" height="40" rx="6"/><text class="t b" x="585" y="165" text-anchor="middle">libunwind (.eh_frame přes dl_iterate_phdr)</text>
<rect class="ok" x="10" y="200" width="760" height="40" rx="6"/><text class="t b" x="390" y="225" text-anchor="middle">cstd (libc.a / libc.so) + osrt + Rust std</text>
<path class="ln" d="M195 50 L195 66" marker-end="url(#cp1)"/>
<path class="ln" d="M585 50 L585 66" marker-end="url(#cp1)"/>
<path class="ln" d="M585 120 L585 136" marker-end="url(#cp1)"/>
<path class="ln" d="M195 120 L195 196" marker-end="url(#cp1)"/>
<path class="ln" d="M585 180 L585 196" marker-end="url(#cp1)"/>
</svg>
<figcaption>libc++ stojí na libc++abi a ta na libunwind; všechno pod nimi je knihovna C cstd nad Rust std, takže proces má jeden alokátor, jedna vlákna a jedno TLS.</figcaption>
</figure>

## Sestavení

Port `llvm-runtimes` (`xtask/src/cports/cxx.rs`) se staví před ostatními
porty, protože C++ porty (HarfBuzz) linkují libc++:

- **Zdroj:** `llvm-project-23.1.5.src.tar.xz` (HTTPS, SHA-256), z archivu
  se rozbalí jen potřebné podstromy (`fetch::prepare_subset`, asi 50 MB
  z 179 MB; testy a dokumentace ne). Záplata
  `third_party/c-ports/llvm-runtimes/0001-magnumopus.patch`:
  `random_device` přes `getentropy`, `chrono` přes `clock_gettime`,
  `__cxa_thread_atexit` v libc++abi, výběr backendu locale a výchozí
  tabulka znaků pro `__magnumopus__`.
- **Dvě konfigurace CMake** nad `runtimes/`:
  - statická (`toolchain.cmake`, bez PIC) instaluje `libc++.a`,
    `libc++abi.a`, `libunwind.a` a hlavičky `include/c++/v1` do
    `target/c-ports/<arch>/prefix`,
  - dynamická (`toolchain-dyn.cmake`, PIC nad `libc.so`) staví
    `libc++.so` s libc++abi uvnitř a samostatnou `libunwind.so` do
    `prefix-dyn`; v initfs jsou jako `/system/lib/libc++.so`
    a `/system/lib/libunwind.so` (`libc++.so` na ni závisí, `DT_NEEDED`).
    **Jeden unwinder na proces** jako `libgcc_s` na Linuxu: dva unwindery
    by měly každý vlastní registraci FDE a mezipaměť a výjimka C++ přes
    rámce jiného modulu (zpětné volání z Windows vrstvy) by šla špatně.
    Sdílené knihovny v C ji dostanou přes `-lunwind` (sysroot C ji má
    v `lib/`; obal `mos-cc` pro Wine ji linkuje s `--as-needed`, takže
    `DT_NEEDED` vznikne jen u modulu, který volá `_Unwind_*`, např.
    `ntdll.so` kvůli `_Unwind_Find_FDE`). Statické programy beze změny
    (`libunwind.a`).
- **Locale a široké znaky** (od 7a-2, `LIBCXX_ENABLE_LOCALIZATION`,
  `LIBCXX_ENABLE_WIDE_CHARACTERS`): backend locale libc++ je
  `support/linux.h` (obecné POSIX 2008: `newlocale`, `uselocale`,
  `*_l`, `mbrtowc`...; funkce dává cstd), třídy znaků pro `ctype<char>`
  bere libc++ z vlastní výchozí tabulky (`_LIBCPP_PROVIDES_DEFAULT_RUNE_TABLE`,
  jako musl). `wchar_t` je 32bitový (UTF-32), locale `C` jednobajtové,
  `C.UTF-8` a ostatní jména UTF-8 ([cstd](cstd.md)).
- **Vlákna** nad pthread z cstd (`LIBCXX_HAS_PTHREAD_API`), bez libgcc
  a compiler-rt: pomocné funkce překladače jsou v `libc.a` (staticky)
  a `libbuiltins.a` (dynamicky).

První sestavení runtime trvá jednotky minut (obě konfigurace, asi 1900
kroků Ninja na architekturu); další jsou přírůstková.

### Program v C++ proti sysrootu

V předponě portů jsou soubory pro `clang++ --config=...`:

| Soubor | Program |
|---|---|
| `target/c-ports/<arch>/prefix/clang++.cfg` | statický ET_EXEC s `libc++.a`, `libc++abi.a`, `libunwind.a`, `libc.a` |
| `target/c-ports/<arch>/prefix/clang++-dyn.cfg` | PIE nad `libc++.so` a `libc.so` (zavaděč `ld.so`) |

Hlavičky libc++ jsou před hlavičkami cstd (`-nostdinc++ -isystem
.../c++/v1`): obalové hlavičky (`<cstdio>`, `<cstdlib>`) vkládají ty
z cstd přes `#include_next`. Způsob `Sources` portů překládá zdroje `.cc`
a `.cpp` přes clang++ a takový program linkuje s libc++
([Porty v C](porty-c.md)).

## Výjimky

Standard Itanium s tabulkami DWARF (`.eh_frame`, `.eh_frame_hdr`), jako
na Linuxu:

- **Hledání tabulek:** libunwind volá `dl_iterate_phdr`. Pod zavaděčem
  odpovídá `ld.so` (každý modul svým `PT_GNU_EH_FRAME`). Statický program
  hlavičky ELF v paměti nemá: `dl_iterate_phdr` v `libc.a` popíše jediný
  modul (`PT_LOAD` přes celý obraz, `PT_GNU_EH_FRAME` nad `.eh_frame_hdr`)
  ze symbolů linker skriptu `user/user-std.ld`
  (`__mos_image_start/end`, `__eh_frame_hdr_start/end`). Tabulku
  `.eh_frame_hdr` vytvoří lld s `--eh-frame-hdr` (sysroot C ho předává).
- **Rámce C:** sysroot C překládá s `-funwind-tables`, takže výjimka projde
  i zpětným voláním z kódu v C (test `vyjimka-pres-ramec-c`).
- **Rámce Rustu:** cíl má `panic=abort` a Rust nemá tabulky rozbalování.
  Výjimka, která by měla projít funkcí v Rustu (například zpětné volání
  z `qsort` v cstd), skončí `std::terminate`. Knihovny v Rustu, které
  volají zpět do C++, musí výjimky chytit na své straně.
- **Výjimky procesoru** (SIGSEGV, dělení nulou) se na výjimky C++
  nepřevádějí (`-fnon-call-exceptions` ne). Výjimka z obsluhy signálu ven =
  `terminate`.

## Konec procesu a `thread_local`

| Funkce | Kde | Chování |
|---|---|---|
| `__cxa_atexit(f, arg, dso)` | cstd (`cxxsup.rs`) | destruktory statických objektů; jeden seznam s `atexit`, při konci v obráceném pořadí registrace |
| `__cxa_finalize(dso)` | cstd | spustí a odebere funkce modulu (`NULL` = všechny) |
| `__dso_handle` | `libc.a` (statický program), `crt1.o` (PIE), `crtdso.o` (každá `.so`) | skrytý symbol modulu, podle něj `__cxa_finalize` pozná, co patří knihovně |
| `__cxa_thread_atexit_impl` | cstd | destruktory `thread_local`; při konci vlákna z `pthread_create` před destruktory klíčů, při `exit` pro volající vlákno před seznamem `atexit` |

Seznam `atexit`/`__cxa_atexit` se spustí z jedné funkce v `osrt::at_exit`
zaregistrované po vyprázdnění proudů stdio, takže proudy se vyprázdní až po
všech destruktorech.

**`dlclose`:** zavaděč moduly neodstraňuje (`dlclose` jen sníží počítadlo,
[dynamické linkování](dynamicke-linkovani.md), jako musl). Kód knihovny tedy
zůstává namapovaný a její destruktory statických objektů a `thread_local`
proběhnou bezpečně při konci procesu, ne při `dlclose`. `__cxa_finalize`
zavaděč nevolá: zničit objekty knihovny, která zůstává nahraná a při dalším
`dlopen` se znovu neinicializuje, by bylo horší než počkat do konce.

## Co doplnil cstd

| Funkce | Pro |
|---|---|
| `getentropy`, `sys/random.h` | `std::random_device`; čeká, dokud generátor jádra není nasazený |
| `aligned_alloc`, `posix_memalign` (cstd-core) | `operator new` se zarovnáním; uvolní obyčejný `free` (zarovnání je v hlavičce bloku, [cstd](cstd.md)) |
| `timespec_get`, `lldiv` | `<ctime>`, `<cstdlib>` |
| `dl_iterate_phdr` (statický), `link.h`, `elf.h` | libunwind |
| `openat`, `unlinkat`, `mkdirat`, `fstatat`, `fchmodat` relativně k adresáři, `realpath`, `pathconf`, `statvfs`, `sendfile`, `PATH_MAX` | `std::filesystem` |

⚠️ FS pracuje s cestami: `openat(dir, "x")` složí cestu adresáře
zapamatovanou při jeho otevření se jménem. Na rozdíl od Linuxu tedy funkce
`*at` nechrání před přejmenováním adresáře mezi otevřením a voláním
(`remove_all` v libc++ na tom staví ochranu před záměnou symlinku; FS
symlinky zatím nevytváří).

## HarfBuzz

Port `harfbuzz` (způsob `Sources`): amalgamace `src/harfbuzz.cc` přeložená
clang++ s `-fno-exceptions -fno-rtti`, bez FreeType, ICU a glib (vlastní
parser OpenType, Unicode tabulky uvnitř). Test `hbtest` (v C) načte písmo
Inter z initfs přes fsd, vytvaruje český text a ověří glyfy (žádný
`.notdef`), posuny a kerning dvojice „AV“. HarfBuzz je první spotřebitel
běhu C++ (titulky libass a vykreslování textu ve FFmpegu, krok 7).

## Testy

| Test | Kde | Co ověřuje |
|---|---|---|
| `cpptest: libc++ staticky` | QEMU, sada `cpp` | `user/cpptest/cpptest.cpp`: proudy (`iostream`, `fstream`, `stringstream`), locale `C.UTF-8` (`ctype<wchar_t>`, `codecvt`), `wstring`, `format`, `regex`, statické konstruktory, kontejnery, výjimky (i přes rámec C), RTTI, zarovnaný `new`, `bad_alloc`, vlákna a `thread_local` s destruktory, `chrono`, `random_device`, `std::filesystem` nad fsd; destruktor globálního objektu po `main` |
| `cpptest-dyn: libc++.so` | QEMU, sada `cpp` | totéž jako PIE nad `/system/lib/libc++.so` |
| `hbtest: HarfBuzz` | QEMU, sada `cpp` | tvarování textu s diakritikou, kerning |
| `cxxsup_tests`, `fsat_tests` | host, `cargo test -p cstd` | pořadí `__cxa_atexit` a `__cxa_finalize` podle modulu, destruktory `thread_local`, `getentropy`, `timespec_get`, `statvfs`, cesty `*at`, `sendfile`, shoda konstant s hlavičkami |

Spuštění: `cargo xtask test --only cpp` (`--arch aarch64` pro druhou
architekturu).
