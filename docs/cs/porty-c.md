# Porty v C

Převzaté knihovny a programy v C (zlib, sqlite, SDL, později FFmpeg) se
staví jedním mechanismem v xtasku (`xtask/src/cports/`) proti
[sysrootu C](cstd.md#sysroot-c-a-porty-část-2i). Každý port je připnutá
verze z oficiálního místa, ověřená otiskem, s našimi záplatami
v repozitáři.

## Stav

| Část | Co | Stav |
|---|---|---|
| verze nástrojů | CMake 3.24+, Ninja 1.10+ (`--version`), rada při staré verzi | **[implementováno]** host test; [Sestavení](sestaveni.md#nástroje) |
| zdroj portu | HTTPS (`curl`), SHA-256, `tar` (i zip), záplaty `git apply`, vlastní soubory (overlay) | **[ověřeno v QEMU]** zlib a sqlite (sada `cstd porty`), záplaty a overlay se SDL (sada `sdl`) |
| `Sources` | seznam zdrojů přeložený clangem přímo | **[ověřeno v QEMU]** zlib 1.3.1, sqlite 3.46.1 |
| `CMake` | CMake + Ninja, soubor nástrojů a modul platformy `MagnumOpus` ze sysrootu | **[ověřeno v QEMU]** SDL 3.4.20 ([SDL](sdl.md), sada `sdl`) |
| `Configure` | skript `configure` (autotools, FFmpeg) + `make` | **[implementováno]**, ověří se s FFmpegem |
| chybějící nástroj | chyba se jménem nástroje a radou k instalaci podle hostitele | **[implementováno]** host test `cargo test -p xtask cports` |

## Port

<figure class="diagram">
<svg viewBox="0 0 780 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Sestavení portu v C">
<defs><marker id="pc1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="10" width="230" height="70" rx="6"/><text class="t b" x="125" y="34" text-anchor="middle">archiv</text><text class="m" x="125" y="52" text-anchor="middle">HTTPS, SHA-256</text><text class="m" x="125" y="68" text-anchor="middle">target/c-ports/dl</text>
<rect class="acc" x="275" y="10" width="230" height="70" rx="6"/><text class="t b" x="390" y="34" text-anchor="middle">zdroj portu</text><text class="m" x="390" y="52" text-anchor="middle">tar + záplaty + overlay</text><text class="m" x="390" y="68" text-anchor="middle">third_party/c-ports/&lt;port&gt;</text>
<rect class="acc" x="540" y="10" width="230" height="70" rx="6"/><text class="t b" x="655" y="34" text-anchor="middle">sestavení</text><text class="m" x="655" y="52" text-anchor="middle">Sources / CMake / Configure</text><text class="m" x="655" y="68" text-anchor="middle">proti sysrootu C</text>
<path class="ln" d="M240 45 L271 45" marker-end="url(#pc1)"/>
<path class="ln" d="M505 45 L536 45" marker-end="url(#pc1)"/>
<rect class="ok" x="275" y="120" width="230" height="64" rx="6"/><text class="t b" x="390" y="146" text-anchor="middle">předpona portů</text><text class="m" x="390" y="164" text-anchor="middle">include/, lib/ (další porty)</text>
<rect class="ok" x="540" y="120" width="230" height="64" rx="6"/><text class="t b" x="655" y="146" text-anchor="middle">initfs</text><text class="m" x="655" y="164" text-anchor="middle">programy (statické ET_EXEC)</text>
<path class="ln" d="M600 80 L440 116" marker-end="url(#pc1)"/>
<path class="ln" d="M655 80 L655 116" marker-end="url(#pc1)"/>
</svg>
<figcaption>Archiv se ověří a připraví se zdroj se záplatami; port se sestaví jedním ze tří způsobů. Knihovny jdou do předpony, programy do initfs.</figcaption>
</figure>

Port je záznam v tabulce `PORTS` (`xtask/src/cports/mod.rs`):

| Pole | Co |
|---|---|
| `url`, `file`, `sha256` | připnutý archiv; jen HTTPS, otisk se kontroluje při každém použití |
| `top` | adresář v archivu |
| `patches` | záplaty v `third_party/c-ports/<port>/` (pořadí použití, `-p1`) |
| `build` | způsob sestavení (níže) |

Vlastní soubory portu (např. ovladač SDL pro náš kompozitor) jsou
v `third_party/c-ports/<port>/overlay/` a kopírují se přes zdroj po
záplatách. Připravený zdroj (`target/c-ports/src/<port>-<otisk>`) se dělá
znovu jen při změně archivu, záplaty nebo souboru overlay.

## Způsoby sestavení

| Způsob | Kdy | Potřebuje |
|---|---|---|
| `Sources` | malé knihovny, kde je seznam zdrojů jednoduchý (zlib, sqlite) | jen clang |
| `CMake` | projekty s CMake (SDL) | CMake, Ninja |
| `Configure` | projekty se skriptem `configure` (autotools, FFmpeg) | `sh`, `make` |

- **`CMake`:** xtask vygeneruje `target/c-ports/<arch>/cmake/toolchain.cmake`
  a modul platformy `Platform/MagnumOpus.cmake` (`UNIX`, `MAGNUMOPUS`, jen
  statické knihovny). Příznaky jsou stejné jako v `clang.cfg` sysrootu;
  `-lc` je v `CMAKE_C_STANDARD_LIBRARIES` (za objekty). Hledání knihoven
  a hlaviček jen v sysrootu a předponě portů, `pkg-config` hostitele je
  vypnutý (nesmí najít knihovny Windows ani Linuxu). Konfigurace se
  opakuje jen při změně voleb, souboru nástrojů nebo zdroje.
- **`Configure`:** argumenty mají zástupné hodnoty `{prefix}`, `{host}`,
  `{arch}`, `{cc}`, `{ar}`, `{cflags}`, `{ldflags}`, protože každý skript
  chce cíl jinak (autotools `--host=`, FFmpeg `--target-os= --arch= --cc=`).
  Sestavuje se mimo strom zdrojů.
- **Výstup:** knihovny a hlavičky do předpony `target/c-ports/<arch>/prefix`
  (další port je najde), programy do initfs.

## Nástroje hostitele

Když nástroj chybí, sestavení skončí jednou chybou se všemi chybějícími
nástroji a radou podle hostitele:

| Nástroj | Windows | Linux / macOS |
|---|---|---|
| CMake (3.24+) | `winget install Kitware.CMake` | `apt install cmake` / `brew install cmake` |
| Ninja | `winget install Ninja-build.Ninja` | `apt install ninja-build` / `brew install ninja` |
| `sh` | Git for Windows | součást systému |
| `make` | MSYS2 (`pacman -S make`) nebo WSL | `apt install make` |

Cestu jde zadat i proměnnou (`CMAKE`, `NINJA`, `SH`, `MAKE`). Na Windows se
hledají i běžná místa instalace mimo `PATH` (instalátor nemusel `PATH`
v otevřeném terminálu obnovit).

> **Past:** `target/` leží uvnitř repozitáře. `git apply` by v něm našel
> náš repozitář a záplatu mimo jeho index tiše přeskočil; xtask proto
> nastavuje `GIT_CEILING_DIRECTORIES` na adresář zdroje portu.

## Porty

| Port | Verze | Licence | Způsob | Programy |
|---|---|---|---|---|
| zlib | 1.3.1 | zlib | `Sources` | `zlib-example`, `minigzip` |
| sqlite | 3.46.1 | public domain | `Sources` | `sqlite3`, `sqltest` |
| SDL | 3.4.20 | zlib | `CMake` | `snake`, `woodeneye`, `testsprite`, `testaudio`, `testkeys`, `checkkeys`, `testmagnumopus` ([SDL](sdl.md)) |

Testy portů a omezení (sqlite bez WAL a mmap I/O) popisuje
[cstd](cstd.md#sysroot-c-a-porty-část-2i).
