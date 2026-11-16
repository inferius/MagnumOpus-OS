# Kód a data třetích stran / Third-party code and data

Jednotný formát pro všechna vlákna: jeden řádek na zdroj. Sloupce:
**Název**, **Verze** (tag, commit nebo datum stažení), **Zdroj** (URL),
**Licence** (SPDX, u souborů bez SPDX text hlavičky), **Cesta** (kde je to
v repozitáři), **Jak použito** (převzato / přeloženo do Rustu / jen fakta).
Kód pod GPL smí být převzat jen do oddělené složky `gpl/` (viz níže); mimo
ni z GPL zdrojů jen fakta (hodnoty registrů, pořadí kroků z dokumentace).

One line per source, same format for all threads: name, version, source,
licence, path in the repository, how it is used (copied / translated to
Rust / facts only). GPL code may be adopted only into the separate `gpl/` folder (see below); outside it, facts only.

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

## FFmpeg 9.0.2 a dav1d 1.5.4 (porty v C / C ports)

Nejsou v repozitáři: `cargo xtask` je stáhne z oficiálních míst (HTTPS),
ověří SHA-256 (`xtask/src/cports/mod.rs`, tabulka `PORTS`) a přeloží proti
sysrootu C bez úprav zdrojů. / Not in the repository: `cargo xtask` downloads
them from the official sites (HTTPS), checks SHA-256 and builds them against
the C sysroot without source changes.

- **FFmpeg:** <https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz>, SHA-256
  `8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e`
  (archiv podepsaný klíčem FFmpeg FCF9 86EA 15E6 E293 A564 4F10 B432 2F04
  D676 58D8, podpis ověřen při připnutí / archive signed with the FFmpeg
  release key, signature checked when pinned).
- **dav1d:** <https://downloads.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz>,
  SHA-256 `686616b7c69eb88d44459391ab25cac13b6647a3b288835c5784e71c1514a5c5`
  (shodný s oficiálním `.sha256` / matches the official `.sha256`).
- **Licence / Licence:** FFmpeg jen LGPL-2.1-or-later: `--disable-gpl
  --disable-version3 --disable-nonfree`; xtask po sestavení ověří
  v `config.h`, že `CONFIG_GPL`, `CONFIG_NONFREE` a `CONFIG_VERSION3` jsou 0,
  jinak sestavení selže. dav1d BSD-2-Clause. / FFmpeg only
  LGPL-2.1-or-later; xtask checks `config.h` after the build and fails
  otherwise. dav1d BSD-2-Clause.
- **Použití / Use:** jen proces dekodéru `ffdec` (`user/ffdec`), který
  spouští služba `mediad`; aplikace FFmpeg nelinkují. FFmpeg je dynamicky:
  `lib/libavutil.so.61`, `libswresample.so.7`, `libswscale.so.10`,
  `libavcodec.so.63` (s dav1d staticky uvnitř), `libavformat.so.63` nad
  `libc.so`. / Only the `ffdec` decoder process started by the `mediad`
  service; applications do not link FFmpeg. FFmpeg is dynamic (the libraries
  above over `libc.so`).
- **Povinnosti LGPL / LGPL obligations:** knihovny jdou nahradit vlastní
  verzí (výměna souborů v `/system/lib`); zdroje = připnutý archiv,
  přepínače configure jsou v `xtask/src/cports/mod.rs` (`FFMPEG_ARGS`),
  záplaty žádné; `cargo xtask build` je skript pro opětovné sestavení.
  / The libraries can be replaced by the user's own version (replace the
  files in `/system/lib`); sources = the pinned archive, configure switches
  in `xtask/src/cports/mod.rs` (`FFMPEG_ARGS`), no patches; `cargo xtask
  build` is the rebuild script.
- **Patentové kodeky / Patent-encumbered codecs:** výchozí obraz obsahuje
  nativní dekodéry FFmpeg pro H.264, HEVC a AAC (LGPL, žádné x264/x265,
  fdk-aac). **Riziko:** při šíření systému (hlavně v USA) mohou patentové
  pooly (MPEG LA / Via LA, Access Advance) uplatnit licenční nároky; licence
  FFmpeg patenty nepokrývá. `XTASK_BEZ_PATENTOVYCH_KODEKU=1` sestaví obraz
  bez těchto dekodérů (ověřeno v `config_components.h`). / The default image
  contains FFmpeg's native H.264, HEVC and AAC decoders (LGPL, no x264/x265,
  fdk-aac). **Risk:** when distributing the system (mainly in the USA) patent
  pools (MPEG LA / Via LA, Access Advance) may raise licence claims; the
  FFmpeg licence does not cover patents. `XTASK_BEZ_PATENTOVYCH_KODEKU=1`
  builds an image without these decoders.
- **Související knihovny / Related libraries (plánováno / planned):**
  libass (ISC), HarfBuzz (MIT), FriBidi (LGPL-2.1-or-later, dynamicky /
  dynamically), FreeType (zvolena licence FTL, ne GPL; vyžaduje zmínku
  "Portions of this software are copyright The FreeType Project" / FTL
  chosen, not the GPL; requires the credit above). Všechny jen v procesu
  dekodéru `ffdec`. / All only inside the `ffdec` decoder process.
- **Popis / Description:** `docs/cs/multimedia.md`, `docs/en/multimedia.md`.

## Souborové systémy vyměnitelných médií / Removable media file systems

Proces média `user/volfs` (docs/cs/media.md); kód cizích FS běží jen v něm,
nikdy ve `fsd`. / The media process `user/volfs` (docs/en/media.md);
foreign FS code runs only there, never in `fsd`.

| Název / Name | Verze / Version | Zdroj / Source | Licence / License | Cesta / Path | Jak použito / How used |
|---|---|---|---|---|---|
| crate `fatfs` | 0.3.6 | https://github.com/rafalh/rust-fatfs | MIT | závislost `user/volfs` (feature `std`) | FAT12/16/32 s dlouhými jmény, čtení i zápis / read and write |
| crate `ntfs` | 0.4.0 | https://github.com/ColinFinck/ntfs | MIT OR Apache-2.0 | závislost `user/volfs` | NTFS jen pro čtení / read only |
| obraz `testfs1` z crate `ntfs` | 0.4.0 | https://github.com/ColinFinck/ntfs/tree/master/testdata | MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE` vedle) | `third_party/test-images/ntfs/` | testovací obraz NTFS 2 MiB beze změny; vytvořen nástrojem `mkntfs` (ntfs-3g, GPL - jen nástroj, nelinkuje se) skriptem `create-testfs1.sh` / unchanged 2 MiB NTFS test image made with `mkntfs` by the script |

exFAT zatím ne: patenty Microsoftu (licence OIN jen pro „Linux System“),
rozhodnutí uživatele čeká. / exFAT not yet: Microsoft patents (the OIN
licence covers only the "Linux System"), the user's decision is pending.

## Testovací vektory AV1 a VP9 / AV1 and VP9 test vectors

Jen v obrazu testů a vývoje (`media/` v initfs), stažené s SHA-256
(`xtask/src/cports/mod.rs`, `TEST_MEDIA`). / Only in the image for tests
and development (`media/` in initfs), downloaded with SHA-256.

- `av1-1-b8-01-size-64x64.ivf` (+ `.md5`): Alliance for Open Media,
  <https://storage.googleapis.com/aom-test-data/>, BSD-2-Clause.
- `vp90-2-02-size-64x64.webm` (+ `.md5`): WebM Project (libvpx),
  <https://storage.googleapis.com/downloads.webmproject.org/test_data/libvpx/>,
  BSD-3-Clause.

## zlib 1.3.1 a SQLite 3.46.1 (porty v C / C ports)

Nejsou v repozitáři: `cargo xtask` je stáhne z oficiálních míst (HTTPS),
ověří SHA-256 (`xtask/src/cports/`) a přeloží proti sysrootu C bez úprav
zdrojů; programy jsou v initfs (testovací obraz). / Not in the repository:
`cargo xtask` downloads them from the official places (HTTPS), verifies the
SHA-256 (`xtask/src/cports/`) and builds them against the C sysroot
without source changes; the programs are in the initfs (test image).

- **zlib** - licence zlib (Jean-loup Gailly, Mark Adler); zdroj / source:
  <https://github.com/madler/zlib/releases/tag/v1.3.1>. Programy / programs:
  `zlib-example`, `minigzip`.
- **SQLite** - public domain; zdroj / source: amalgamace / amalgamation
  <https://www.sqlite.org/2024/sqlite-amalgamation-3460100.zip>. Programy /
  programs: `sqlite3` (shell), `sqltest` (náš test / our test).
- **Popis / Description:** `docs/cs/cstd.md` (kapitola Sysroot C a porty),
  `docs/en/cstd.md`.

## SDL 3.4.20 (port v C / C port)

Není v repozitáři: `cargo xtask` stáhne vydání z GitHubu (HTTPS), ověří
SHA-256, použije naši záplatu a vlastní ovladač videa
(`third_party/c-ports/sdl3/`) a sestaví ho CMake + Ninja proti sysrootu C
(`xtask/src/cports/`). / Not in the repository: `cargo xtask` downloads the
release from GitHub (HTTPS), verifies the SHA-256, applies our patch and own
video driver (`third_party/c-ports/sdl3/`) and builds it with CMake + Ninja
against the C sysroot (`xtask/src/cports/`).

- **Licence / Licence:** zlib (Sam Lantinga a přispěvatelé / and
  contributors); zdroj / source:
  <https://github.com/libsdl-org/SDL/releases/tag/release-3.4.20>.
- **Naše úpravy / Our changes:** `0001-magnumopus.patch` (registrace
  ovladačů videa a zvuku, `SDL_PLATFORM_MAGNUMOPUS`, bez GTK a `misc/unix`,
  test `testmagnumopus`) a ovladače `overlay/src/video/magnumopus/`
  a `overlay/src/audio/magnumopus/` (náš kód, licence zlib jako SDL). Úpravy se neposílají do projektu SDL: ten nepřijímá kód
  generovaný AI. / Our changes stay here: the SDL project does not accept
  AI-generated contributions.
- **Programy / programs:** ukázkové hry / example games `snake`, `woodeneye`
  (examples/demo); testy SDL / SDL tests `testsprite`, `testaudio`,
  `testkeys`, `checkkeys`; náš test / our test `testmagnumopus`. Data: `sdl-icon.png`,
  `sdl-sample.wav` (z / from `test/`).
- **Popis / Description:** `docs/cs/sdl.md`, `docs/en/sdl.md`.

## LLVM 23.1.5: libc++, libc++abi, libunwind (běh C++ / C++ runtime)

Není v repozitáři: `cargo xtask` stáhne `llvm-project-23.1.5.src.tar.xz`
z GitHubu (HTTPS, 179 MB), ověří SHA-256, rozbalí jen `runtimes/`,
`libcxx/`, `libcxxabi/`, `libunwind/`, `libc/` (sdílené hlavičky pro
`std::from_chars`) a `cmake/`, použije naši záplatu a sestaví runtime CMake
+ Ninja (`xtask/src/cports/cxx.rs`). Do programů se linkují staticky
(`libc++.a`, `libc++abi.a`, `libunwind.a`) a do initfs jde sdílená
`/system/lib/libc++.so`. / Not in the repository: `cargo xtask` downloads
`llvm-project-23.1.5.src.tar.xz` from GitHub (HTTPS, 179 MB), verifies the
SHA-256, extracts only `runtimes/`, `libcxx/`, `libcxxabi/`, `libunwind/`,
`libc/` (shared headers for `std::from_chars`) and `cmake/`, applies our
patch and builds the runtime with CMake + Ninja (`xtask/src/cports/cxx.rs`).
Programs link it statically and the shared `/system/lib/libc++.so` goes to
the initfs.

- **Licence / Licence:** Apache-2.0 WITH LLVM-exception (výjimka LLVM:
  binární podoba nevyžaduje uvedení licence / the LLVM exception waives the
  attribution requirement for binary forms); plný text / full text:
  `third_party/c-ports/llvm-runtimes/LICENSE-libcxx.TXT`,
  `LICENSE-libcxxabi.TXT`, `LICENSE-libunwind.TXT` (z archivu / from the
  archive). Zdroj / source:
  <https://github.com/llvm/llvm-project/releases/tag/llvmorg-23.1.5>.
- **Naše úpravy / Our changes:** `0001-magnumopus.patch` pro / for
  `__magnumopus__`: `random_device` nad / over `getentropy`, `chrono` nad /
  over `clock_gettime`, `__cxa_thread_atexit`, backend locale `linux.h`,
  výchozí tabulka znaků / default character table.
- **Programy / programs:** `cpptest`, `cpptest-dyn` (náš test / our test,
  `user/cpptest/`).
- **Popis / Description:** `docs/cs/cpp.md`, `docs/en/cpp.md`.

## FreeType 2.15.0 (port v C / C port)

Není v repozitáři: `cargo xtask` stáhne vydání z download.savannah.gnu.org
(HTTPS), ověří SHA-256 (archiv ověřený podpisem Wernera Lemberga, klíč
`E306 7470 7856 409F F194 8010 BE6C 3AAC 63AD 8E3F`) a sestaví ho přes
CMake jako statickou `libfreetype.a` i sdílenou `libfreetype.so`
(`/system/lib`) pro x86_64 i aarch64 (`xtask/src/cports/`). Bez úprav
zdrojů; vypnuté PNG, bzip2, Brotli a HarfBuzz, komprimovaná písma přes zlib
vložený ve FreeType. / Not in the repository: `cargo xtask` downloads the
release from download.savannah.gnu.org (HTTPS), verifies the SHA-256 (the
archive checked against Werner Lemberg's signature, key
`E306 7470 7856 409F F194 8010 BE6C 3AAC 63AD 8E3F`) and builds it with
CMake as a static `libfreetype.a` and a shared `libfreetype.so`
(`/system/lib`) for x86_64 and aarch64. No source changes; PNG, bzip2,
Brotli and HarfBuzz off, compressed fonts through the zlib bundled in
FreeType.

- **Licence / Licence:** FreeType License (FTL); FreeType je pod dvojí
  licencí FTL / GPLv2, používáme jen FTL (BSD-like s uvedením autorů:
  „Portions of this software are copyright © The FreeType Project
  (www.freetype.org). All rights reserved.“) / FreeType is dual licensed
  FTL / GPLv2, we use FTL only (BSD-like with credit). Text:
  `third_party/c-ports/freetype/FTL.TXT` (z archivu / from the archive).
  Zdroj / source: <https://download.savannah.gnu.org/releases/freetype/>.
- **Programy / programs:** `fttest` (náš test / our test,
  `user/ctest/c/fttest.c`), data `ft-test.ttf` (písmo Inter / Inter font).
- **Uživatelé / users:** Wine (`win32u` ji načte `dlopen` z `/system/lib`
  / loads it with `dlopen` from `/system/lib`), jiné programy v C / other C
  programs.
- **Popis / Description:** `docs/cs/porty-c.md`, `docs/en/c-ports.md`.

## HarfBuzz 14.2.0 (port v C++ / C++ port)

Není v repozitáři: `cargo xtask` stáhne vydání z GitHubu (HTTPS), ověří
SHA-256 a přeloží amalgamaci `src/harfbuzz.cc` bez výjimek a RTTI proti
libc++ (`xtask/src/cports/`). Bez úprav zdrojů. / Not in the repository:
`cargo xtask` downloads the release from GitHub (HTTPS), verifies the
SHA-256 and builds the `src/harfbuzz.cc` amalgamation without exceptions
and RTTI against libc++. No source changes.

- **Licence / Licence:** „Old MIT“ (MIT-like, Behdad Esfahbod, Google,
  Red Hat a další / and others); text: `third_party/c-ports/harfbuzz/COPYING`
  (z archivu / from the archive). Zdroj / source:
  <https://github.com/harfbuzz/harfbuzz/releases/tag/14.2.0>. Otisk SHA-256
  zaznamenán při prvním stažení (projekt ho nezveřejňuje) / SHA-256
  recorded at the first download (the project does not publish one).
- **Programy / programs:** `hbtest` (náš test / our test,
  `user/cpptest/hbtest.c`), data `hb-test.ttf` (písmo Inter, viz níže /
  Inter font, see below).
- **Popis / Description:** `docs/cs/cpp.md`, `docs/en/cpp.md`.

## Unicode Character Database

- **Název / Name:** Unicode Character Database (`UnicodeData.txt`, `DerivedCoreProperties.txt`, `PropList.txt`, `EastAsianWidth.txt`)
- **Verze / Version:** 18.0.0 (= `core::char::UNICODE_VERSION` překladače / of the compiler)
- **Zdroj / Source:** https://www.unicode.org/Public/18.0.0/ucd/ (stahuje / downloaded by `cargo xtask unicode-tables`, SHA-256 v / in `xtask/src/unicode.rs`)
- **Licence / Licence:** Unicode License v3 (Unicode-3.0), plný text / full text: `libs/uniprop/UNICODE-LICENSE.txt`
- **Cesta / Path:** `libs/uniprop/src/tables.rs` (odvozené tabulky, ne soubory UCD / derived tables, not the UCD files)
- **Popis / Description:** `docs/cs/cstd.md`, `docs/en/cstd.md` (kapitola Široké znaky a locale / chapter Wide characters and locale).

## Wine 11.0 (Windows vrstva / Windows layer)

Není v repozitáři: `cargo xtask wine` stáhne `wine-11.0.tar.xz`
z dl.winehq.org (HTTPS; SHA-512 ověřený proti podepsanému seznamu
`sha512sums.asc`, v xtasku připnutý SHA-256), použije naše záplaty
z `third_party/wine/` a sestaví ho ve WSL (DLL jako PE a unixovou část nad
`libc.so`). / Not in the repository: `cargo xtask wine` downloads
`wine-11.0.tar.xz` from dl.winehq.org (HTTPS; SHA-512 checked against the
signed list `sha512sums.asc`, SHA-256 pinned in xtask), applies our patches
from `third_party/wine/` and builds it in WSL (DLLs as PE and the Unix side
over `libc.so`).

- **Licence / Licence:** LGPL-2.1-or-later (Wine), zdroj / source:
  <https://dl.winehq.org/wine/source/11.0/>. Běží v procesech aplikací
  Windows a ve wineserveru, ne v jádře ani ve službách systému / runs in
  Windows application processes and in wineserver, not in the kernel or
  system services.
- **Naše úpravy / Our changes:** záplaty v `third_party/wine/` (LGPL jako
  Wine): `0001-config-sub-magnumopus.patch` (systém `magnumopus`
  v `config.sub`), `0002-ntdll-signal-x86_64-magnumopus.patch` (kontext
  signálu jako Linux, TEB v GS přes `magnumopus/thread.h`, bez LDT),
  `0003-server-no-fork-magnumopus.patch` (wineserver se místo `fork`
  spustí znovu v popředí přes `posix_spawn`, synchronizace rourou jako
  dřív / wineserver restarts itself in the foreground via `posix_spawn`
  instead of `fork`, the same pipe handshake),
  `0004-server-memfd-sections-magnumopus.patch` (paměť anonymních sekcí
  jako `memfd` místo dočasného souboru / anonymous section memory as a
  `memfd` instead of a temporary file),
  `0005-ntdll-spawn-process-magnumopus.patch` (nový proces Windows přes
  `posix_spawn` zavaděče místo dvojího `fork` / a new Windows process via
  `posix_spawn` of the loader instead of a double `fork`),
  `0006-process-memory-magnumopus.patch` (klient svěří wineserveru paměť
  procesovým fd, `read/write_process_memory` přes `PROCESS_MEMORY` / the
  client entrusts its memory to wineserver with a process fd,
  `read/write_process_memory` via `PROCESS_MEMORY`),
  `0007-server-signals-magnumopus.patch` (signály serveru přes řídicí fd
  procesu a fd vláken / server signals via the process control fd and
  thread fds), `0008-winemo-drv-magnumopus.patch` (ovladač `winemo.drv`
  v `configure`, výchozí ovladač grafiky v `explorer` / the `winemo.drv`
  driver in `configure`, the default graphics driver in `explorer`).
- **Ovladač grafiky / Graphics driver:** `third_party/wine/winemo.drv/`
  (LGPL jako Wine, část `window.c` a tabulka kláves z `winewayland.drv` /
  LGPL like Wine, part of `window.c` and the key table from
  `winewayland.drv`): okna Wine nad kompozitorem přes naši knihovnu
  `mowin` (náš kód, `libs/mowin`) / Wine windows on the compositor through
  our `mowin` library (our code, `libs/mowin`). Sestavení ho zkopíruje do
  `dlls/winemo.drv` / the build copies it into `dlls/winemo.drv`.
- **Popis / Description:** `docs/cs/windows-vrstva.md`,
  `docs/en/windows-layer.md`.

## Písmo Tuffy / Tuffy font

- **Název / Name:** Tuffy Regular
- **Verze / Version:** z testovacích písem crate `fontdb` 0.24 / from the test fonts of the `fontdb` 0.24 crate
- **Zdroj / Source:** http://tulrich.com/fonts/ (Thatcher Ulrich)
- **Licence / Licence:** public domain
- **Cesta / Path:** `libs/font/fonts/Tuffy.ttf` (popis: `libs/font/fonts/README.md`)

## Písmo Inter / Inter font

- **Název / Name:** Inter (statické řezy / static faces Light, Regular, Medium, SemiBold, Bold)
- **Verze / Version:** 4.1
- **Zdroj / Source:** https://github.com/rsms/inter/releases/tag/v4.1 (`Inter-4.1.zip`, `extras/ttf/`)
- **Licence / Licence:** SIL Open Font License 1.1, plný text / full text: `libs/font/fonts/Inter-OFL.txt`
- **Cesta / Path:** `libs/font/fonts/Inter-*.ttf`

## Písmo JetBrains Mono / JetBrains Mono font

- **Název / Name:** JetBrains Mono (Regular, SemiBold)
- **Verze / Version:** 2.304
- **Zdroj / Source:** https://github.com/JetBrains/JetBrainsMono/releases/tag/v2.304 (`fonts/ttf/`)
- **Licence / Licence:** SIL Open Font License 1.1, plný text / full text: `libs/font/fonts/JetBrainsMono-OFL.txt`
- **Cesta / Path:** `libs/font/fonts/JetBrainsMono-*.ttf`

## Kód pod GPL (`gpl/`) / Code under the GPL (`gpl/`)

Pravidlo (rozhodnuto 2026-11-14): kód pod GPL jen ve složce `gpl/`
(GPL-2.0-only, `gpl/LICENSE`), jen jako samostatný proces ovladače
komunikující přes veřejný protokol; podrobně `gpl/README.md` a
`docs/cs/ovladace.md`. Zatím tam žádný kód není.

Rule (decided 2026-11-14): GPL code only in `gpl/` (GPL-2.0-only), only as
a standalone driver process talking over the public protocol; details in
`gpl/README.md` and `docs/en/drivers.md`. No code there yet.

Šablona hlavičky souboru v `gpl/` / header template for files in `gpl/`:

```text
// SPDX-License-Identifier: GPL-2.0-only
// Puvod / Origin: <projekt, verze nebo commit, cesta> (<URL>)
// Copyright (C) <puvodni autori>
// Zmeny / Changes: <co se zmenilo oproti puvodnimu kodu>
// Tento ovladac je samostatny proces; se systemem mluvi jen pres
// protokol <nazev protokolu>. Zadne sdilene vnitrni struktury.
```

Tabulka / table (stejné sloupce jako výše / same columns as above):

| Název | Verze | Zdroj | Licence | Cesta | Jak použito |
|---|---|---|---|---|---|
