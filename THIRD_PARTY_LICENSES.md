# Licence převzatého kódu / Third-party licences

Kód převzatý do repozitáře (vendored) a jeho licence. Závislosti z crates.io
mají licence ve svých balíčcích (`cargo metadata`); tady je jen kód, který
leží přímo v repozitáři.

Vendored code kept in this repository and its licence. Dependencies from
crates.io carry their licences in their packages (`cargo metadata`); this
file lists only code stored in the repository itself.

## ACPICA

- **Název / Name:** ACPICA (ACPI Component Architecture) - interpret AML, tabulky, události / AML interpreter, tables, events
- **Verze / Version:** 20250404 (tag `R2025_04_04`, commit `8a54b2a8c4bc2a60d1ddaba4a1ffebe38c2a0062`)
- **Zdroj / Source:** https://github.com/acpica/acpica
- **Licence / Licence:** BSD-3-Clause (zvolená z trojí licence Intel / BSD / GPLv2; GPL se nepoužívá / chosen from the triple licence, GPL not used)
- **Cesta / Path:** `third_party/acpica` (popis verze a zásahů: `third_party/acpica/README.md`); používá / used by `libs/acpica`, `user/acpid`

Hlavičky zdrojových souborů ACPICA nabízejí vedle licence Intel výslovně
i licenci BSD ("Alternatively, you may choose to be licensed under the terms
of the following license") a GPLv2. Systém používá **licenci BSD** (je
slučitelná s licencí MIT/Apache-2.0 zbytku systému); plné znění všech tří
je v `third_party/acpica/LICENSE`. Podmínka licence BSD pro binární
distribuci: přiložit upozornění "NO WARRANTY" níže.

The ACPICA source headers explicitly offer, besides the Intel licence, the
BSD licence ("Alternatively, you may choose to be licensed under the terms
of the following license") and GPLv2. The system uses the **BSD licence**
(compatible with the MIT/Apache-2.0 licence of the rest of the system); the
full text of all three is in `third_party/acpica/LICENSE`. The BSD
condition for binary redistribution: include the "NO WARRANTY" disclaimer
below.

```text
Copyright (c) 1999 - 2025, Intel Corp.
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
1. Redistributions of source code must retain the above copyright
   notice, this list of conditions, and the following disclaimer,
   without modification.
2. Redistributions in binary form must reproduce at minimum a disclaimer
   substantially similar to the "NO WARRANTY" disclaimer below
   ("Disclaimer") and any redistribution must be conditioned upon
   including a substantially similar Disclaimer requirement for further
   binary redistribution.
3. Neither the names of the above-listed copyright holders nor the names
   of any contributors may be used to endorse or promote products derived
   from this software without specific prior written permission.

NO WARRANTY
THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Písmo Tuffy / Tuffy font

- **Název / Name:** Tuffy Regular
- **Verze / Version:** z testovacích písem crate `fontdb` 0.24 / from the test fonts of the `fontdb` 0.24 crate
- **Zdroj / Source:** http://tulrich.com/fonts/ (Thatcher Ulrich)
- **Licence / Licence:** public domain
- **Cesta / Path:** `libs/font/fonts/Tuffy.ttf` (popis: `libs/font/fonts/README.md`)
