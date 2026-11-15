# libs/cstd-core

Základ knihovny C v Rustu (`no_std`, ABI C): řetězce, `ctype`, `malloc` nad alokátorem programu, `strtol` a spol., formátovací stroj (`snprintf`), ochrana zásobníku. Používají ho služby no_std s kódem v C (ACPICA v `acpid`, závislost `cstd = { package = "cstd-core", ... }`) i `libs/cstd` pro aplikace. Popis: `docs/cs/preklad-c.md`, `docs/cs/cstd.md`.

## Klíčové soubory

- `src/format.rs` - formátovací stroj `printf` (`Out`, `Args`, `VaArgs`); `src/float.rs` - `%f %e %g %a` přes `core::fmt` do bufferu na zásobníku (~4,4 KiB); `src/string.rs`, `src/ctype.rs`, `src/stdlib.rs`, `src/stdio.rs`, `src/ssp.rs`.
- `include/` - základní hlavičky (`_cstd.h` s makry, `string.h`, `stdlib.h`, `stdio.h`, `ctype.h`, `assert.h`).

## Invarianty

- Feature `service-io` (výchozí) exportuje `printf`, `puts`, `putchar`, `abort`, `__cstd_assert_fail` s výstupem do logu služby (`set_output`). Aplikace se std ji vypínají - tyto symboly dodává `cstd` nad std.
- `malloc` má 16B hlavičku těsně před ukazatelem: velikost a zarovnání (16 B, nebo víc u `alloc_aligned`, tedy `aligned_alloc` a `posix_memalign` z `cstd`); `free` z ní spočítá `Layout`. `realloc` zarovnaného bloku vrátí obyčejný blok (C větší zarovnání po `realloc` nezaručuje).
- Žádné nebezpečné funkce (`sprintf`, `gets`, `strtok`); `%n` nic nezapíše.

## Testy

- `cargo test -p cstd-core`; ACPICA v QEMU: `cargo xtask test --only acpi`.

## Pasti

- Plovoucí čárka předpokládá hard-float (všechny programy v userspace): `VaArgs::double` čte `f64` z `VaList` podle ABI. `%Lf` = `?`.
- `parse_float` (`stdlib.rs`) je čistá logika `strtod`; symbol `strtod` s `errno` exportuje až `cstd`.
