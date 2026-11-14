# Dynamické linkování

Sdílené knihovny ELF (`ET_DYN`) a programy PIE: zavaděč `ld.so`,
relokace, TLS sdílených knihoven, `dlopen`, sdílení kódu knihoven mezi
procesy a pečeť RELRO. Je to krok 4 v pořadí převzetí cizího softwaru
([architektura.md](architektura.md#bezpečnost)): potřebuje ho unixová část
Wine a FFmpeg (LGPL, dynamicky). Programy v Rustu zůstávají statické
(`ET_EXEC`, [rust-std.md](rust-std.md)).

## Stav

| Část | Co | Stav |
|---|---|---|
| DL-1 | parser `ET_DYN` v `libs/elf`: programové hlavičky (`PT_INTERP`, `PT_TLS`, `PT_GNU_RELRO`, `PT_DYNAMIC`), dynamická sekce, symboly s GNU a SysV hashem, relokace RELA a RELR obou architektur, odmítnutí TEXTREL | **[implementováno]** (host testy `cargo test -p elf` proti souborům z clang a ld.lld) |
| DL-2 | zavaděč `user/ld` + `libs/dynload`: `PT_INTERP` ve `spawn`, ASLR, `DT_NEEDED`, `BIND_NOW`, relokace, statické TLS; PIC v `cbuild`, `/system/lib` v obrazu | **[navrženo]** |
| DL-3 | `dlopen`/`dlsym`/`dlclose`, dynamické TLS (`__tls_get_addr`, TLSDESC), `dl_iterate_phdr`, `r_debug` | **[navrženo]** |
| DL-4 | služba šablon `libd` (sdílení kódu knihoven), pečeť RELRO | **[navrženo]** |

## Návrh (schválené volby)

<figure>
<svg viewBox="0 0 760 200" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Spuštění programu PIE">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.t{font:13px sans-serif;fill:#1d2333}
.m{font:11px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="dl1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="5" y="30" width="130" height="60" rx="6"/><text class="t b" x="70" y="55" text-anchor="middle">spawn (rodič)</text><text class="m" x="70" y="73" text-anchor="middle">program + ld.so</text>
<rect class="acc" x="165" y="30" width="130" height="60" rx="6"/><text class="t b" x="230" y="55" text-anchor="middle">ld.so (dítě)</text><text class="m" x="230" y="73" text-anchor="middle">PT_INTERP, ASLR</text>
<rect class="acc" x="325" y="30" width="130" height="60" rx="6"/><text class="t b" x="390" y="55" text-anchor="middle">DT_NEEDED</text><text class="m" x="390" y="73" text-anchor="middle">/system/lib, $ORIGIN</text>
<rect class="acc" x="485" y="30" width="130" height="60" rx="6"/><text class="t b" x="550" y="55" text-anchor="middle">relokace</text><text class="m" x="550" y="73" text-anchor="middle">BIND_NOW, TLS</text>
<rect class="box" x="645" y="30" width="110" height="60" rx="6"/><text class="t b" x="700" y="55" text-anchor="middle">RELRO R--</text><text class="m" x="700" y="73" text-anchor="middle">pak vstup programu</text>
<line class="ln" x1="135" y1="60" x2="161" y2="60" marker-end="url(#dl1)"/>
<line class="ln" x1="295" y1="60" x2="321" y2="60" marker-end="url(#dl1)"/>
<line class="ln" x1="455" y1="60" x2="481" y2="60" marker-end="url(#dl1)"/>
<line class="ln" x1="615" y1="60" x2="641" y2="60" marker-end="url(#dl1)"/>
<text class="m" x="5" y="130">Kód knihoven ze zapečetěných šablon (libd, sdílené stránky), data a GOT jako kopie při zápisu.</text>
<text class="m" x="5" y="150">DL-1 (tato část): parser libs/elf, ze kterého ld.so čte hlavičky, symboly a relokace.</text>
</svg>
<figcaption>Spuštění programu PIE se sdílenými knihovnami (DL-2 až DL-4 navrženo).</figcaption>
</figure>

- **Zavaděč:** samostatný `ld.so` (zároveň `libc.so` ve stylu musl),
  program má `PT_INTERP`; zavaděč psaný v Rustu (`libs/dynload`,
  `user/ld`), sám staticky relokovaný.
- **Formát:** ELF `ET_DYN` podle SysV gABI: `DT_GNU_HASH` i `DT_HASH`,
  RELA a RELR, `DT_RUNPATH`, `SONAME`. Jen `BIND_NOW` a plné RELRO (žádné
  líné PLT). **`DT_TEXTREL` se odmítá** (zápis do spustitelné paměti
  odporuje W^X). TLS: local-exec, initial-exec, general/local-dynamic přes
  `__tls_get_addr`, TLSDESC (na aarch64 výchozí). Verze symbolů
  (`DT_VERSYM`) se čtou, ale nevyhodnocují; skrytá verze jen vyřadí symbol
  z vyhledávání.
- **Kde leží knihovny:** `/system/lib` (neměnná součást obrazu) a soukromé
  knihovny aplikace přes `$ORIGIN`; `LD_LIBRARY_PATH` jen pro vývoj.
- **Verze:** soubor se jmenuje přesně jako `SONAME` s hlavní verzí
  (`libavcodec.so.61`), bez řetězů symlinků.
- **ASLR:** náhodná báze každého `ET_DYN` v každém procesu, zarovnání
  64 KiB.
- **Rust zůstává statický**; dynamické jsou C/C++ (`cstd` jako `libc.so`)
  a pluginy `cdylib`.
- **RELRO:** po relokaci `VM_PROTECT` na R-- a snížení `max_flags`
  (jako `mseal`).

## Parser `libs/elf` (DL-1)

**[implementováno]** Soubor je nedůvěryhodný: každé čtení má kontrolu mezí,
vadný vstup vrátí `ElfError`, nikdy nepanikaří (test
`damaged_input_does_not_panic`: zkrácení a tisíce náhodně změněných bajtů).

| API | Co |
|---|---|
| `ElfFile::parse(data)` | statický program (`ET_EXEC`) pro tuto architekturu - loader, jádro, `spawn` (beze změny) |
| `ElfFile::parse_object(data, Machine)` | `ET_EXEC` i `ET_DYN` pro zadanou architekturu (host testy umí obě) |
| `program_headers`, `load_segments` (s `align`), `interp`, `tls` + `tls_data`, `relro` | programové hlavičky |
| `dynamic()` -> `Dynamic` | `DT_NEEDED` (nejvýš 32), `SONAME`, `RUNPATH` (jinak `RPATH`), tabulky, `init`/`fini` a pole, příznaky (`bind_now`, `pie`, `static_tls`), verze; **odmítne `DT_TEXTREL`/`DF_TEXTREL` (`ElfError::TextRelocations`) a `DT_REL`** |
| `symbols(&Dynamic)` -> `SymbolTable` | `get(i)`, `lookup(jméno)` přes GNU hash (Bloomův filtr), jinak SysV; vrací jen exportované symboly (globální/slabé, viditelnost default/protected, verze ne skrytá) |
| `relas(&Dynamic)` | RELA a PLT jako `Rela { offset, kind, sym, addend }`; `Kind` společný pro obě architektury (`Relative`, `Absolute`, `GlobDat`, `JumpSlot`, `TlsModule`, `TlsOffset`, `TlsTpOffset`, `TlsDesc`, `IRelative`, `Copy`, `Unknown`) |
| `relrs(&Dynamic)` | adresy relativních relokací z RELR (dekodér `Relr`) |
| `at_vaddr(addr, len)` | data souboru na virtuální adrese (musí ležet v datech jednoho `PT_LOAD`) |

Testovací soubory (`libs/elf/tests/fixtures/`, zdroj v C a `gen.sh`):
knihovna s RELA a oběma hashi, táž s RELR a jen GNU hashem, program PIE
s `PT_INTERP`, knihovna s TEXTREL (jen x86_64 - na aarch64 ji lld
nevytvoří). Překládají se clangem a ld.lld s `max-page-size=65536`
a commitují se, testy clang nepotřebují.

## Testy

| Test (`cargo test -p elf`) | Ověřuje |
|---|---|
| `shared_library_headers` | `PT_TLS` a jeho data, RELRO, `DT_NEEDED`, `SONAME`, `RUNPATH $ORIGIN`, `BIND_NOW`, zarovnání 64 KiB (x86_64 i aarch64) |
| `symbol_lookup_gnu_and_sysv` | vyhledání přes GNU i jen SysV hash, druh a hodnota symbolu, skrytý a nedefinovaný symbol se nenajde |
| `relocations_x86_64`, `relocations_aarch64` | druhy relokací a jejich symboly (TLS GD s `__tls_get_addr`, TLSDESC na aarch64), `Rela::value` |
| `relr_matches_relative_relocations`, `relr_decoder` | RELR ze souboru a syntetické bitmapy (i prázdná bitmapa posune o 63 slov) |
| `pie_program` | `PT_INTERP`, `DF_1_PIE`, `ElfFile::parse` PIE odmítne |
| `rejects_textrel_and_wrong_machine` | TEXTREL, jiná architektura, krátký soubor |
| `hash_functions_match_reference` | GNU a SysV hash proti známým hodnotám |
| `damaged_input_does_not_panic` | zkrácení a náhodné změny bez paniky |
