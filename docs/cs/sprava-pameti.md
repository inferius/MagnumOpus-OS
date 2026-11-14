# Správa paměti procesů

Jak jádro dává procesům paměť: objekty paměti, stránky na požádání,
nulová stránka, rozpočty a piny. Je to krok 3 v pořadí převzetí cizího
softwaru ([architektura.md](architektura.md), kapitola Převzetí cizího software: pořadí):
podklad pro `mmap` v C knihovně, pro Wine a pro pozdější mapování souborů
z fsd. Rozložení adresního prostoru a ochrany (SMAP, NX) popisuje
[jadro.md](jadro.md#paměť), syscally [syscally.md](syscally.md).

## Stav po částech

| Část | Co | Stav |
|---|---|---|
| 1 | stránky na požádání, nulová stránka, výpadek v jádře, commit a strop, piny | **[ověřeno v QEMU]** x86_64 i aarch64 (self-testy jádra `pamet`, sada `pamet` v initu) |
| 2 | rezervace (`VM_RESERVE`), částečné `VM_UNMAP` a decommit, `VM_PROTECT`, `VM_QUERY`, ochranné oblasti | **[ověřeno v QEMU]** x86_64 i aarch64 (self-testy `mm_regions`, sada `pamet`) |
| 3 | `VMO_CLONE` (copy-on-write) a sdílený kód programů | **[navrženo]** |
| 4 | `VMO_SEAL`, W^X s oprávněním „smí JIT“, audit | **[navrženo]** |
| 5 | API v libos (podklad `mmap` v `cstd`), guard pod zásobníky | **[navrženo]** |

## Stránky na požádání

Objekt paměti (`VMO_CREATE`) při vytvoření **slíbí** celou velikost, ale
fyzické rámce si vezme až při prvním zápisu:

- **Slib (commit)** se strhne hned dvakrát: z rozpočtu uživatelské paměti
  jobu (jako dřív) a z globálního stropu paměti pro programy
  (`phys::commit`). Strop = volná RAM bez rezervy jádra a bez už slíbených
  stránek. Co se slíbit nedá, selže hned při vytvoření (`NO_MEMORY`).
- **Proto nikdy OOM killer:** výpadek stránky si bere rámec ze slibu, takže
  na nedostatek paměti nenarazí. Výjimka je jen jádro samo: kdyby jeho
  heap a tabulky stránek vyčerpaly víc než svou rezervu, výpadek skončí
  ukončením procesu s hláškou v logu.
- **Čtení** dosud nevzniklé stránky dává nuly a rámec nevznikne.
- **Mapování** (`VM_MAP`) namapuje jen stránky, které už existují. Ostatní
  doplní výpadek.

<figure>
<svg viewBox="0 0 760 300" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Výpadek stránky">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.warn{fill:#fbefe6;stroke:#a0602a;stroke-width:1.2}
.t{font:14px sans-serif;fill:#1d2333}
.m{font:12px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}.warn{fill:#33261c;stroke:#d39a63}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="sp1" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="10" y="20" width="200" height="56" rx="6"/><text class="t b" x="110" y="44" text-anchor="middle">výpadek v userspace</text><text class="m" x="110" y="63" text-anchor="middle">adresa + čtení/zápis/spuštění</text>
<rect class="box" x="280" y="20" width="200" height="56" rx="6"/><text class="t b" x="380" y="44" text-anchor="middle">mapování a práva</text><text class="m" x="380" y="63" text-anchor="middle">pod zámkem prostoru</text>
<rect class="warn" x="550" y="20" width="200" height="56" rx="6"/><text class="t b" x="650" y="44" text-anchor="middle">konec procesu</text><text class="m" x="650" y="63" text-anchor="middle">mimo mapování / bez práva</text>
<rect class="acc" x="130" y="150" width="240" height="70" rx="6"/><text class="t b" x="250" y="174" text-anchor="middle">čtení / spuštění</text><text class="m" x="250" y="193" text-anchor="middle">stránka existuje -> její rámec</text><text class="m" x="250" y="210" text-anchor="middle">jinak nulová stránka jen pro čtení</text>
<rect class="acc" x="400" y="150" width="240" height="70" rx="6"/><text class="t b" x="520" y="174" text-anchor="middle">zápis</text><text class="m" x="520" y="193" text-anchor="middle">rámec ze slibu (commit_page)</text><text class="m" x="520" y="210" text-anchor="middle">nulová stránka pryč (BBM, shootdown)</text>
<rect class="box" x="400" y="245" width="240" height="44" rx="6"/><text class="m" x="520" y="263" text-anchor="middle">ostatním mapováním nulovou stránku</text><text class="m" x="520" y="279" text-anchor="middle">odmapovat (forget_zero)</text>
<line class="ln" x1="210" y1="48" x2="276" y2="48" marker-end="url(#sp1)"/>
<line class="ln" x1="480" y1="48" x2="546" y2="48" marker-end="url(#sp1)"/>
<line class="ln" x1="350" y1="76" x2="260" y2="146" marker-end="url(#sp1)"/>
<line class="ln" x1="410" y1="76" x2="510" y2="146" marker-end="url(#sp1)"/>
<line class="ln" x1="520" y1="220" x2="520" y2="241" marker-end="url(#sp1)"/>
<text class="m" x="10" y="120">Po vyřešení se instrukce provede znovu.</text>
</svg>
<figcaption>Výpadek stránky: čtení dostane nulovou stránku, zápis vlastní rámec.</figcaption>
</figure>

## Oblasti: rezervace, části, práva

Adresní prostor se skládá z **oblastí**: mapování objektu paměti nebo
**rezervace** (adresy zabrané bez paměti). Operace na rozsahu uvnitř
oblasti ji nejdřív rozdělí na hranicích; každý kus má vlastní záznam
v rozpočtu jádra jobu.

| Operace | Co dělá | Pro koho |
|---|---|---|
| `VM_RESERVE` | zabere adresy bez paměti; stojí jen záznam (ne tabulky stránek) | Wine (`VirtualAlloc(MEM_RESERVE)`), velké arény, zásobníky |
| `VM_MAP` + `MAP_IN_RESERVATION` | namapuje objekt jen dovnitř rezervace (s `MAP_FIXED`) | `MEM_COMMIT` |
| `VM_UNMAP` | zruší i část oblasti; `UNMAP_DECOMMIT` = mapování zpět na rezervaci | `munmap`, `MEM_DECOMMIT`, `MEM_RELEASE` |
| `VM_PROTECT` | práva v rozsahu, 0 = žádný přístup; nikdy W+X, nikdy nad práva při `VM_MAP` | `mprotect`, `VirtualProtect` |
| `VM_QUERY` | oblast nebo volné místo na adrese | `VirtualQuery`, výpis paměti |

- **Ochranná oblast** (`RESERVE_GUARD`): přístup do ní ukončí proces
  s hláškou „přístup do ochranné oblasti (přetečení zásobníku?)“.
  Doručení výjimky programu (Windows `PAGE_GUARD`, SEH) přijde s krokem 5
  pořadí převzetí. Ochrana pod zásobníky v libos je část 5.
- **Přidat spuštění** (`VM_PROTECT` s X nad původní práva) dnes nejde;
  přijde s oprávněním „smí JIT“ (část 4).
- **Rezervace bez tabulek stránek:** tabulky se strhávají jen
  za mapování (odhad podle délky při `VM_MAP`), takže rezervace desítek GiB
  stojí stovky bajtů.

## Nulová stránka

Jedna sdílená vynulovaná stránka (`mm::zero_frame`), mapovaná vždy **jen
pro čtení** za stránky, které ještě nevznikly. Program, který si vezme
velký buffer a jen z něj čte, tak nespotřebuje žádnou paměť.

Když stránka později vznikne, nesmí nikdo dál číst starou nulovou
stránku místo zapsaných dat:

- **zápis výpadkem** v témže mapování nulovou stránku nahradí vlastním
  rámcem (break-before-make: na aarch64 `unmap_4k` zneplatní TLB všech CPU
  hned, na x86_64 doběhne shootdown před návratem vlákna);
- **každý jiný vznik stránky** (zápis jiným mapováním nebo jiným procesem,
  `VMO_WRITE`, kopírování jádra do paměti procesu, `VMO_PHYS_ADDR`)
  odmapuje nulovou stránku všem mapovacím procesům objektu
  (`MemoryObject::forget_zero` -> `AddressSpace::drop_zero_ptes`).

**Souběh:** výpadek čtení nastaví příznak objektu „někde je nulová
stránka“ dřív, než ověří, že stránka neexistuje; vznik stránky (CAS
položky) přečte příznak až po sobě. Kdo nulovou stránku namapoval před
vznikem stránky, toho `forget_zero` najde, protože bere zámek jeho
adresního prostoru. Kdo přišel po vzniku, uviděl stránku už existující.

## Kopírování jádra

Jádro do paměti procesu nikdy nesahá přes jeho adresy (SMAP zůstává
zapnutý), takže v jádře výpadek nevznikne:

- **čtení** (`copy_from_user`) dosud nevzniklé stránky čte z nulové
  stránky;
- **zápis** (`copy_to_user`) nejdřív stránky rozsahu vytvoří a odmapuje
  za ně nulovou stránku, teprve potom kopíruje.

## Piny a DMA

Pole počítadel pinů má jeden záznam na každý rámec RAM (`phys::pin`,
`unpin`, `is_pinned`). Připnutý rámec používá zařízení: nesmí se
přesunout, nahradit kopií (budoucí CoW) ani vystěhovat (budoucí pager).

- **Paměť pro DMA** (`VMO_CREATE_CONTIGUOUS`) je souvislá, celá přítomná
  hned a připnutá po celou dobu života objektu. Líné stránky ani nulová
  stránka se jí netýkají.
- **`VMO_PHYS_ADDR` na běžném objektu** stránku vytvoří a připne. Adresa,
  kterou ovladač předá zařízení, tak zůstane platná, dokud objekt žije.

## Rozpočty

| Co | Kdy se strhne | Z čeho |
|---|---|---|
| stránky objektu | při vytvoření (celá velikost) | uživatelská paměť jobu + globální strop |
| rámec stránky | při prvním zápisu | ze slibu (nic navíc) |
| seznam stránek, objekt | při vytvoření | paměť jádra jobu |
| tabulky stránek mapování | při `VM_MAP` (odhad podle délky) | paměť jádra jobu |

Slib se vrátí při zániku objektu (stránky, které nikdy nevznikly) nebo
s rámcem. **Později** (zapsáno jako možnost): příznak `SPARSE` pro řídké
objekty (arény `malloc` v `cstd`), u kterých by se slib strhával až při
výpadku.

## Klíčové soubory

| Soubor | Co |
|---|---|
| `kernel/src/mm/phys.rs` | commit a strop (`commit`, `uncommit`, `alloc_committed_zeroed`), pole pinů |
| `kernel/src/object/vmo.rs` | položky stránek (0 = nevznikla), `commit_page`, `forget_zero`, `expose_phys` |
| `kernel/src/mm/address_space.rs` | mapování jen existujících stránek, kopírování jádra, `drop_zero_ptes` |
| `kernel/src/mm/address_space/fault.rs` | řešení výpadku (`resolve_fault`) |
| `kernel/src/arch/*/` | rozpoznání výpadku (x86_64 chybový kód, aarch64 ESR) -> `trap::user_page_fault` |

## Testy

- Self-testy jádra (sada `pamet`, `kernel/src/tests/mm_lazy.rs`): commit
  bez rámců, strop commitu, čtení a zápis výpadkem, práva, koherence nulové
  stránky mezi mapováními a procesy, kopírování jádra, piny.
- Testy userspace (`cargo xtask test --only pamet`,
  `user/init/src/tests/memory_lazy.rs`): objekt 256 MiB, nulová stránka
  proti zápisu jiným mapováním a `VMO_WRITE`, 4 vlákna na stejných
  stránkách, slib nad RAM.
