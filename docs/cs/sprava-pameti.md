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
| 3 | `VMO_CLONE` (copy-on-write) a sdílený kód programů z initfs | **[ověřeno v QEMU]** x86_64 i aarch64 (self-testy `mm_cow`, sada `pamet`) |
| 4 | `VMO_SEAL`, W^X s oprávněním „smí JIT“ (politika jobu, manifest), audit | **[ověřeno v QEMU]** x86_64 i aarch64 (self-testy `mm_wx`, sada `pamet`) |
| 5 | API v libos (podklad `mmap` v `cstd`), ochranná oblast pod zásobníky | **[ověřeno v QEMU]** x86_64 i aarch64 (sada `pamet`) |
| 6 | zvětšitelný objekt paměti (`VMO_RESIZABLE`, `VMO_SET_SIZE`, `VMO_SIZE`), výjimka za koncem (`BEYOND_END`), paměťový fd v `cstd` | **[ověřeno v QEMU]** x86_64 i aarch64 (self-testy `mm_resize`, sada `cstd`: krok `memfd`, test `ctest memfd` dvou procesů) |

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

## Kopie při zápisu (CoW) a sdílený kód

`VMO_CLONE` vytvoří klon, který **sdílí rámce** zdroje. Každý rámec má
v `phys` počítadlo dalších vlastníků (`share`, `shares`, `try_unshare`);
položka stránky sdíleného rámce má příznak `SHARED` a mapuje se jen pro
čtení.

<figure>
<svg viewBox="0 0 760 250" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Kopie při zápisu">
<style>
.box{fill:#f4f6fb;stroke:#4a5a80;stroke-width:1.2}
.acc{fill:#e8f1e4;stroke:#3f7a3a;stroke-width:1.2}
.t{font:14px sans-serif;fill:#1d2333}
.m{font:12px sans-serif;fill:#3a4256}
.b{font-weight:bold}
.ln{stroke:#4a5a80;stroke-width:1.2;fill:none}
@media (prefers-color-scheme: dark){
.box{fill:#1f2533;stroke:#8fa2d0}.acc{fill:#1f2e22;stroke:#7fbf78}
.t{fill:#e6e9f2}.m{fill:#c3c9d9}.ln{stroke:#8fa2d0}}
</style>
<defs><marker id="sp2" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto"><path d="M0,0 L8,4 L0,8 z" fill="#4a5a80"/></marker></defs>
<rect class="box" x="20" y="20" width="160" height="50" rx="6"/><text class="t b" x="100" y="44" text-anchor="middle">zdroj</text><text class="m" x="100" y="61" text-anchor="middle">položka SHARED</text>
<rect class="box" x="20" y="150" width="160" height="50" rx="6"/><text class="t b" x="100" y="174" text-anchor="middle">klon</text><text class="m" x="100" y="191" text-anchor="middle">položka SHARED</text>
<rect class="acc" x="300" y="85" width="170" height="50" rx="6"/><text class="t b" x="385" y="108" text-anchor="middle">rámec F</text><text class="m" x="385" y="125" text-anchor="middle">shares = 1</text>
<rect class="box" x="560" y="20" width="190" height="64" rx="6"/><text class="t b" x="655" y="42" text-anchor="middle">zápis klonu</text><text class="m" x="655" y="60" text-anchor="middle">kopie F -> F', shares 0</text><text class="m" x="655" y="76" text-anchor="middle">(slib klonu)</text>
<rect class="box" x="560" y="140" width="190" height="64" rx="6"/><text class="t b" x="655" y="162" text-anchor="middle">pak zápis zdroje</text><text class="m" x="655" y="180" text-anchor="middle">jediný vlastník: F bez kopie</text><text class="m" x="655" y="196" text-anchor="middle">(slib zdroje zpět)</text>
<line class="ln" x1="180" y1="45" x2="296" y2="100" marker-end="url(#sp2)"/>
<line class="ln" x1="180" y1="175" x2="296" y2="120" marker-end="url(#sp2)"/>
<line class="ln" x1="470" y1="100" x2="556" y2="60" marker-end="url(#sp2)"/>
<line class="ln" x1="470" y1="120" x2="556" y2="165" marker-end="url(#sp2)"/>
</svg>
<figcaption>Kopie při zápisu: kopíruje jen ten, kdo zapisuje, dokud má rámec ještě dalšího vlastníka.</figcaption>
</figure>

- **Zápis do sdíleného rámce** (výpadek, `VMO_WRITE`, kopírování jádra,
  `VMO_PHYS_ADDR`) jde přes `MemoryObject::writable_page`, vždy bez zámku
  adresního prostoru:
  1. odmapuje rámec ve všech mapováních objektu (žádný starý zápis přes
     TLB už do sdíleného rámce nedopadne);
  2. pod `cow_lock` rámec zkopíruje a vzdá se svého dílu, nebo - když už
     nemá další vlastníky - si ho nechá bez kopie;
  3. odmapuje starý rámec mapováním, která ho mezitím namapovala pro čtení.
- **Do rámce s dalšími vlastníky nikdo nezapisuje**, takže kopie je
  konzistentní. Výpadek zápisu na sdíleném rámci pustí zámek prostoru,
  zkopíruje a instrukci zopakuje.
- **Slib:** klon slíbí celou svou velikost, zdroj jednu stránku navíc za
  každý poprvé sdílený rámec (sám může zapisovat jako první). Kdo si
  rámec nakonec nechá (poslední vlastník), slib za kopii vrátí. Kopie tak
  nikdy nenarazí na nedostatek paměti. Slib navíc za zdroj se strhne
  z rozpočtu jobu klonu.
- **Připnuté rámce** (DMA, `VMO_PHYS_ADDR`) se nesdílejí: klon dostane
  kopii hned. `VMO_PHYS_ADDR` na sdíleném rámci nejdřív udělá vlastní
  kopii a teprve tu připne.
- **Souběh s klonováním:** zápisy zdroje během volání `VMO_CLONE` klon
  může vidět; zdroj o žádný zápis nepřijde.

**Sdílený kód programů** (`libs/libos/src/spawn.rs`): program z initfs se
při prvním spuštění nahraje do šablony (objekt paměti na segment, na účet
spouštějícího). Kód a konstanty se mapují do každého procesu **přímo ze
šablony** (všechny instance sdílejí tytéž stránky), zapisovatelná data
dostane každý proces jako **klon s kopií při zápisu** na účet svého jobu.
Sdílí se jen obraz uvnitř namapovaného initfs (neměnný obsah, klíč je
offset v initfs); program načtený odjinud (např. ze souboru do haldy) se
nahrává jako dřív, aby záměna obrazu nemohla podstrčit cizí kód.
Zapečetění šablon (`VMO_SEAL`) přijde v části 4.

## W^X, pečeť a oprávnění JIT

- **Nikdy zápis a spuštění zároveň**, ani s JIT (`VM_MAP` i `VM_PROTECT`).
- **Pečeť** (`VMO_SEAL`): obsah objektu se už nikdy nezmění (zápis
  jádrem, mapování pro zápis i připnutí pro zařízení selžou). Objekt
  s mapováním, které smí zapisovat, zapečetit nejde. Programy se nahrávají
  zápisem, pak pečeť, pak mapování pro spuštění: spawn v libos zapečetí
  šablony kódu, jádro zapečetí kód initu.
- **Bez oprávnění JIT** jde spouštět jen zapečetěná paměť: kód se po
  spuštění už nemůže změnit.
- **S oprávněním JIT** (`jit::RUN` jobu cílového procesu) smí `VM_MAP`
  i `VM_PROTECT` spouštět nezapečetěnou paměť: přepínat RW <-> RX nebo
  mapovat tentýž objekt dvakrát (RW pro zápis, RX pro spuštění - bez
  shootdownu při každém přepnutí).
- **Kdo JIT dává:** politika jobu (`JOB_SET_LIMIT` s `policy::JIT`). Nový
  job nemá nic, kořenový (init) jen udělování; zapnout ho smí jen ten,
  jehož rodič smí udělovat
  (`jit::GRANT`), odebrání platí hned i pro potomky. Init dá právo
  udělovat jobům relací a `jit::RUN` aplikaci, která má v manifestu
  `[app] jit = true` (prohlížeč, .NET).
- **Audit:** první spustitelná nezapečetěná paměť procesu se zaloguje
  (varování s adresou a délkou) a `JOB_PROCESSES` ukáže
  `proc_flags::JIT_USED`.
- **Nejvyšší práva mapování** určí `VM_MAP` podle práv handlu a politiky:
  zápis jen s `WRITE` a ne u zapečetěného objektu, spuštění jen s
  `EXECUTE` a jen u zapečetěného objektu nebo s JIT.

**Omezení (řečeno výslovně):** W^X tady chrání proti tomu, aby se
spustitelný kód za běhu změnil nebo aby vznikla paměť zapisovatelná
a spustitelná zároveň (typický exploit přes poškozenou paměť). **Nechrání
proti procesu, který může volat syscally:** ten si může nový kód zapsat do
objektu, zapečetit ho a spustit - stejně jako to dělá zavaděč programů.
Silnější ochrana (spouštět jen kód zapečetěný důvěryhodným zavaděčem nebo
podepsaný) je **[navrženo]** a patří k plánovaným podpisům programů
(Secure Boot, brána spuštění - viz [architektura.md](architektura.md#bezpečnost));
potřebuje k pečeti capability „smí vytvářet spustitelný kód“.

## Zvětšitelný objekt paměti

Běžný objekt paměti velikost nemění. Zvětšitelný (`VMO_CREATE` s
`vm::VMO_RESIZABLE`, `libos::Vmo::create_resizable`; C `memfd_create`) má
**kapacitu** pevnou od vytvoření a **velikost** mezi 0 a kapacitou:

- Seznam stránek se alokuje hned na celou kapacitu (jádro platí 8 B na
  stránku kapacity, rozpočet jádra jobu), takže čtenáři bez zámku nikdy
  nevidí jeho přesun. Slib (`phys::commit`) a rozpočet uživatelské paměti
  platí jen za aktuální velikost - model „slib při vytvoření, žádný OOM
  kill“ zůstává: zvětšení nové stránky hned slíbí, nebo selže
  (`NO_MEMORY`).
- `VMO_SET_SIZE` (právo `RESIZE`) mění velikost. **Zmenšení** jde
  v pořadí: zveřejnit novou velikost (výpadek za koncem pak selže; výpadek,
  který mezitím stránku vytvořil, ji po kontrole vrátí sám), odmapovat
  rozsah u všech mapperů (`drop_frame_ptes`, mapování zůstane), vyprázdnit
  TLB a teprve pak uvolnit rámce a slib. Připnutá stránka za novým koncem
  = `BAD_STATE`.
- Mapování smí pokrýt celou kapacitu (jako `mmap` za konec souboru).
  Výpadek za aktuální velikostí se nevyřeší: výjimka `PAGE_FAULT`
  s důvodem `abi::exception::reason::BEYOND_END` v poli `reason` zprávy
  (`Report`), knihovna C z ní udělá `SIGBUS`. Kopírování jádra
  (`copy_to_user`) za koncem = `BAD_ADDRESS`, `VMO_READ`/`VMO_WRITE`
  `OUT_OF_RANGE`.
- `VMO_SIZE` vrátí aktuální velikost (C `fstat` v jiném procesu).
  Sdílení mezi procesy je obyčejné `VM_MAP` téhož objektu (stejné rámce, ne
  kopie); příjemce jen pro čtení dostane handle bez `WRITE` a `RESIZE`.

## API v libos

Funkce nad vlastním adresním prostorem (`libs/libos/src/vm.rs`), podklad
pro `mmap` v C knihovně (`cstd`) a pro Wine:

| Funkce | Co dělá | Odpovídá |
|---|---|---|
| `vm::reserve(len, flags)`, `vm::reserve_at(addr, len, flags)` | rezervace bez paměti (`RESERVE_GUARD` = ochranná) | `VirtualAlloc(MEM_RESERVE)` |
| `vm::alloc(len, prot)` | anonymní paměť na volném místě (stránky na požádání) | `mmap(MAP_ANONYMOUS)` |
| `vm::commit(addr, len, prot)` | anonymní paměť do rezervace | `MEM_COMMIT` |
| `vm::protect(addr, len, prot)` | práva (0 = žádný přístup) | `mprotect`, `VirtualProtect` |
| `vm::decommit(addr, len)` | paměť zpět, adresy zůstanou rezervované | `MEM_DECOMMIT` |
| `vm::release(addr, len)` | uvolní i část oblasti | `munmap`, `MEM_RELEASE` |
| `vm::query(addr)` | oblast nebo volné místo | `VirtualQuery` |
| `vm::map_stack(process, vmo, len)` | zásobník s ochrannou oblastí pod sebou | - |

Pro jiné procesy (zavaděč, ladění) jsou stejné operace na `Process`
(`map`, `reserve`, `protect`, `query`, `decommit`, `unmap`).

**Ochranná oblast pod zásobníky:** hlavní zásobník programu (`spawn`)
i zásobník každého vlákna (`thread::spawn`) má pod sebou ochrannou
rezervaci `vm::STACK_GUARD` (64 KiB, nestojí paměť). Přetečení zásobníku
tak skončí výpadkem v ochranné oblasti s hláškou „přístup do ochranné
oblasti (přetečení zásobníku?)“ místo tichého přepsání cizí paměti.
`join` vlákna uvolní zásobník i ochrannou oblast najednou.

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
