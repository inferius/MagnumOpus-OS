# Kód a data třetích stran / Third-party code and data

Jednotný formát pro všechna vlákna: jeden řádek na zdroj. Sloupce:
**Název**, **Verze** (tag, commit nebo datum stažení), **Zdroj** (URL),
**Licence** (SPDX, u souborů bez SPDX text hlavičky), **Cesta** (kde je to
v repozitáři), **Jak použito** (převzato / přeloženo do Rustu / jen fakta).
Kód pod GPL se nepřekládá; z GPL zdrojů jen fakta (hodnoty registrů,
pořadí kroků z dokumentace).

One line per source, same format for all threads: name, version, source,
licence, path in the repository, how it is used (copied / translated to
Rust / facts only). GPL code is never translated.

Kód převzatý celý do repozitáře (vendored, např. ACPICA) má vlastní sekci
s plným textem licence, pokud ho licence vyžaduje v binární distribuci.
Závislosti z crates.io mají licence ve svých balíčcích (`cargo metadata`).

Code vendored whole into the repository (e.g. ACPICA) has its own section
with the full licence text when the licence requires it in binary
distributions. Dependencies from crates.io carry their licences in their
packages (`cargo metadata`).

## Nastavení obrazu / Display configuration

| Název / Name | Verze / Version | Zdroj / Source | Licence / License | Cesta / Path | Jak použito / How used |
|---|---|---|---|---|---|
| Linux `drivers/gpu/drm/drm_edid.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (text "Permission is hereby granted, free of charge") | `libs/display/src/edid.rs`, `cta.rs`, `modes.rs` | postupy (pořadí zdrojů režimů, HF-SCDB), napsáno znovu |
| Linux `drivers/gpu/drm/drm_displayid.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX) | `libs/display/src/displayid.rs` | fakta (jednotky clock typu I/VII), napsáno znovu |
| Linux `drivers/gpu/drm/drm_modes.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (text "Permission is hereby granted, free of charge") | `libs/display/src/cvt.rs` | ověření vzorců CVT, napsáno znovu z VESA CVT 1.2 |
| Linux `drivers/gpu/drm/display/drm_dp_helper.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT-like (Keith Packard: "Permission to use, copy, modify, distribute, and sell ... without fee") | `libs/display/src/dp/auxch.rs`, `train.rs` | postupy AUX (DEFER, I2C přes AUX), napsáno znovu |
| Linux `drivers/gpu/drm/display/drm_dp_mst_topology.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT-like (Red Hat: "Permission to use, copy, modify, distribute, and sell ... without fee") | `libs/display/src/dp/mst.rs` | algoritmus CRC4/CRC8 sideband zpráv, přeloženo do Rustu |
| Linux `include/drm/display/drm_dp.h` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT-like (Keith Packard) | `libs/display/src/dp/mod.rs` | fakta (adresy DPCD) |
| Linux `drivers/gpu/drm/display/drm_hdmi_helper.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX) | `libs/display/src/hdmi.rs` | fakta (SCDC, scrambling), napsáno znovu |
| Linux `drivers/gpu/drm/i915/display/intel_dp_link_training.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (text "Permission is hereby granted, free of charge") | `libs/display/src/dp/train.rs` | postup (počty kol, společné buzení), napsáno znovu |
| Linux `drivers/gpu/drm/i915/display/intel_dp_aux.c`, `intel_dp_aux_regs.h` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX) | `libs/intelgfx/src/dpaux.rs`, `regs.rs` | fakta (registry, bity, postup), napsáno znovu |
| Linux `drivers/gpu/drm/i915/display/intel_gmbus.c`, `intel_gmbus.h`, `intel_gmbus_regs.h` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (text "Permission is hereby granted"; `intel_gmbus.h` SPDX MIT) | `libs/intelgfx/src/gmbus.rs`, `regs.rs` | fakta (registry, piny), napsáno znovu |
| Linux `drivers/gpu/drm/i915/display/intel_display_regs.h`, `intel_display_reg_defs.h`, `skl_universal_plane_regs.h` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX) | `libs/intelgfx/src/regs.rs` | fakta (offsety a bity registrů) |
| Linux `drivers/gpu/drm/i915/display/intel_display_device.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX) | `libs/intelgfx/src/regs.rs` | fakta (offsety pipe a transcoderů) |
| Linux `drivers/gpu/drm/i915/display/intel_ddi.c`, `intel_display.c`, `intel_dpll_mgr.c`, `intel_hdmi.c`, `skl_universal_plane.c`, `intel_vblank.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX nebo text "Permission is hereby granted") | `libs/intelgfx/src/phy.rs`, `backend.rs` | postupy (tabulka úrovní buzení, flip přes PLANE_SURF), napsáno znovu |
| Linux `drivers/gpu/drm/i915/gt/intel_ggtt.c`, `intel_gtt.h` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (SPDX) | `libs/intelgfx/src/gtt.rs` | fakta (umístění GGTT, bity položky) |
| Linux `include/drm/intel/pciids.h` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (text "Permission is hereby granted, free of charge") | `libs/intelgfx/src/platform.rs` | fakta (PCI ID) |
| Linux `drivers/gpu/drm/i915/i915_reg.h`, `gem/i915_gem_stolen.c` | master, 2026-11-08 | https://github.com/torvalds/linux | MIT (text "Permission is hereby granted"; `i915_gem_stolen.c` SPDX MIT) | `user/devmgr/src/native.rs`, `user/intel-display/src/main.rs` | fakta (kodovani GMS) |
| Linux `arch/x86/kernel/early-quirks.c` | master, 2026-11-08 | https://github.com/torvalds/linux | GPL-2.0 | `user/devmgr/src/native.rs` | **jen fakta** (velikost stolen paměti Gen9 z GGC), kód nepřevzat |
| linuxhw/EDID (AOC 2243W `07E32DADDF83`, LG Display LGD0217 `925C880E8A08`) | master, 2026-11-08 | https://github.com/linuxhw/EDID | CC-BY-4.0 | `libs/display/src/tests/data.rs`, `libs/intelgfx/src/tests/data.rs` | data (dumpy EDID pro testy), beze změny |
| Intel Graphics PRM (Skylake až Lunar Lake) | veřejné svazky (Intel, "Graphics for Linux" developer reference) | intel.com | dokumentace | `libs/intelgfx` | fakta (popis registrů); hodnoty v kódu ověřeny proti hlavičkám i915 výše |
| VESA CVT 1.2, DMT 1.0 r13, E-EDID 1.4, DisplayID 1.3/2.0, DisplayPort 1.4a; CTA-861-G; HDMI 1.4b/2.0 | specifikace | VESA, CTA, HDMI Forum | specifikace | `libs/display` | fakta (formáty, vzorce, tabulky) |

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
