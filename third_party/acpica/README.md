# ACPICA (vendored)

Referenční implementace ACPI od Intelu, převzatá beze změn kódu.
Intel's reference ACPI implementation, vendored with no code changes.

| | |
|---|---|
| Verze / Version | 20250404 |
| Tag | `R2025_04_04` |
| Commit | `8a54b2a8c4bc2a60d1ddaba4a1ffebe38c2a0062` |
| Odkud / From | https://github.com/acpica/acpica |
| Licence | BSD-3-Clause (zvolená z trojí licence, viz `../../THIRD-PARTY.md` a `LICENSE`) |
| Překlad / Build | `libs/acpica/build.rs` (přes `libs/cbuild`), konfigurace `libs/acpica/include/acmagnum.h` |

## Co je převzato / What is included

- `source/include` - všechny hlavičky / all headers,
- `source/components/{dispatcher,events,executer,hardware,namespace,parser,resources,tables,utilities}`,
- `LICENSE`.

Vynecháno / Left out: `source/components/{debugger,disassembler}`,
`source/compiler` (iASL), `source/tools`, `source/os_specific`, testy a
dokumentace. Nepřekládá se / Not compiled: `resources/rsdump.c` (ladicí
výpis), `utilities/utprint.c` (vlastní `printf` ACPICA - máme ho v
`libs/cstd-core`).

## Zásahy / Edits

Jediná změna: výběr platformy v `source/include/platform/acenv.h` (stejně
jako to dělají ostatní systémy) - za větev `__ZEPHYR__`:

```c
#elif defined(__MAGNUMOPUS__)
#include "acmagnum.h"
```

`acmagnum.h` leží mimo převzatý strom (`libs/acpica/include`). Při
aktualizaci ACPICA stačí tyto dva řádky přidat znovu.
