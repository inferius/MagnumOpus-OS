# Jádro: základní funkce

Jak jádro funguje dnes (milník 4). Proč je navržené takto, popisuje
[architektura.md](architektura.md).

## Start

1. **UEFI loader** (`loader/`) načte z ESP `\os\kernel.elf` a `\os\initfs.img`,
   postaví tabulky stránek (identita, přímá mapa fyzické paměti, jádro
   v horních 2 GiB), zjistí framebuffer a ACPI RSDP, ukončí boot services
   a skočí do jádra s ukazatelem na `BootInfo` (`libs/boot-info`).
2. **Jádro na prvním CPU** (`kernel/src/main.rs`): konzole, GDT/TSS/IDT,
   správa paměti (vlastní tabulky stránek), ACPI (MADT, HPET), kalibrace
   hodin (TSC) a APIC timeru, uvolnění paměti firmwaru, přepnutí na zásobník
   s ochrannou stránkou.
3. **Plánovač a ostatní CPU**: registrace CPU, start ostatních CPU
   (INIT-SIPI-SIPI, kontrola a korekce posunu TSC), vlákno `boot`.
4. **Vlákno `boot`**: self-testy jádra, převzetí initfs, spuštění procesu
   `init`. V testovacím režimu (`cargo xtask test`) počká na konec `init`
   a ukončí QEMU s výsledkem.

## Paměť

**Virtuální adresní prostor** (každý proces má vlastní spodní polovinu,
horní polovinu sdílí s jádrem):

| Od | Co |
|---|---|
| `0x0000_0000_0001_0000` | paměť procesu (pod ní nic - nulový ukazatel vždy selže) |
| `0x0000_0000_0040_0000` | programy (statické ELF) |
| `0x0000_1000_0000_0000` | mapování vybraná jádrem (heap, zásobníky, ...) |
| `0xFFFF_8000_0000_0000` | přímá mapa veškeré fyzické paměti (RW, NX) |
| `0xFFFF_FD00_0000_0000` | registry zařízení (MMIO, bez cache) |
| `0xFFFF_FE00_0000_0000` | zásobníky jádra (každý s ochrannou stránkou) |
| `0xFFFF_FFFF_8000_0000` | obraz jádra (kód R-X, konstanty R--, data RW-) |

- **Fyzická paměť**: buddy alokátor (`mm/phys.rs`), bloky 4 KiB až 4 MiB.
  Paměť pod 1 MiB je vyhrazená pro start CPU.
- **Heap jádra** (`mm/heap.rs`): třídy velikostí 16 B - 2 KiB, větší
  alokace přímo z buddy alokátoru (max. 4 MiB najednou).
- **Ochrana**: NX pro data, W^X pro programy, SMEP (jádro nespustí kód
  procesu) a SMAP (jádro nesahá na paměť procesu přes jeho adresy - data
  syscallů kopíruje přes přímou mapu podle seznamu mapování, takže v jádře
  nemůže vzniknout page fault na adrese procesu).
- **Objekty paměti** (`object/vmo.rs`): stránky fyzické paměti, které jde
  číst, zapisovat a mapovat. Stránky vznikají na požádání (slib při
  vytvoření, rámec při prvním zápisu, čtení přes sdílenou nulovou stránku)
  **[ověřeno v QEMU]**; podrobně [sprava-pameti.md](sprava-pameti.md).
- **Adresní prostor procesu** (`mm/address_space.rs`): seznam mapování
  (objekt + offset + práva). Odmapování vyprázdní TLB na všech CPU
  (`mm/tlb.rs`) dřív, než se uvolní stránky.

## Plánovač

Podrobnosti, srovnání návrhů a měření: [Plánovač](planovac.md).

- Každé CPU má jednu frontu (`libs/schedcore`): nejdřív vlákna `realtime`
  se zbývajícím rozpočtem (pevný rozpočet v periodě, EDF), pak spravedlivé
  třídy `interactive`, `normal` a `background` s vahami 15 : 4 : 1 (vážený
  virtuální čas tříd), uvnitř třídy EEVDF s kvantem podle třídy
  (3 / 6 / 12 ms). Nic nehladoví: `background` má na vytíženém CPU aspoň
  1/20 času mimo `realtime`.
- **Přidělování tříd**: syscall `SCHED_CONTROL`, strop z jobu (aplikace si
  sama `realtime` nedá); zpráva propůjčí třídu odesílatele vláknu, které
  ji obslouží (dědění přes IPC).
- **Tickless**: časovač se nastaví jen na konec kvanta, obnovu rozpočtu
  `realtime` nebo nejbližší probuzení; nečinné CPU spí v `hlt`.
- **Rozkládání** podle třídy a kapacity jádra: `interactive` a `realtime`
  na výkonná jádra, `background` na úsporná (Intel hybrid CPUID 0x1A, ARM
  MIDR, ACPI PPTT); nečinné CPU si bere práci z cizích front.
- **Preempce** vlákno nikdy neuspí ani neukončí, jen ho vrátí do fronty -
  vlákno může být přerušeno kdekoli, i uprostřed přípravy na čekání.
- **Probuzení**: každé čekání má pořadové číslo; probuzení se starým číslem
  se ignoruje. Plané probuzení je dovolené, čekající kód podmínku ověří znovu.
- **Přepnutí na vlákno procesu** nastaví jeho adresní prostor (CR3)
  a zásobník jádra pro vstup z userspace (TSS.RSP0); vlákna jádra a končící
  vlákna procesu běží v adresním prostoru jádra.

## Objekty, handly, práva

- Objekty: `Event`, `Channel` (konec kanálu), `MemoryObject`, `Process`,
  `Thread`. Každý má **signály** (`SIGNALED`, `TERMINATED`, `READABLE`,
  `PEER_CLOSED`), na které lze čekat (jeden i více objektů, s termínem).
- **Tabulka handlů** procesu: handle = 20 bitů index + 12 bitů generace.
  Slot, který vyčerpá generace, se vyřadí - starý handle tak nikdy nezačne
  platit pro jiný objekt.
- **Práva** (`READ`, `WRITE`, `EXECUTE`, `MAP`, `DUPLICATE`, `TRANSFER`,
  `WAIT`, `SIGNAL`, `MANAGE`) lze při duplikaci jen ubírat.
- **Kanály**: zprávy s daty a handly; předání handlu ho přesune (odesílatel
  o něj přijde). Zavření konce nastaví druhému `PEER_CLOSED`.
- **Porty** (`kernel/src/object/port.rs`): jednorázové registrace „až
  signály objektu protnou masku, vlož paket s klíčem“. Čekání na libovolně
  mnoho objektů bez limitu `OBJECT_WAIT_MANY`. Objekt drží registraci
  v seznamu čekajících stejně jako vlákno (`ObjectCore`), na port jen slabý
  odkaz. Místo pro paket se rezervuje předem, doručení z přerušení tedy
  nealokuje. Pořadí zámků: objekt, fronta portu, signály portu. Viz
  [Porty](syscally.md#porty).
- **Náhodná čísla** (`kernel/src/random.rs`, crate `libs/csprng`):
  zásobník entropie (BLAKE2s) a generátor ChaCha20 s mazáním klíče po
  každém požadavku. Zdroje: `arch::hw_random` (x86 RDSEED/RDRAND),
  jitter čítače, časování přerušení (bez zámku v `trap::device_irq`)
  a ovladač virtio-rng (`RANDOM_ADD`). Viz
  [Náhodná čísla](syscally.md#náhodná-čísla).

## Procesy a vlákna

- Proces vzniká prázdný (`PROCESS_CREATE`), rodič do něj namapuje paměť
  a spustí první vlákno (`PROCESS_START`) s bootstrap kanálem. Formát
  spustitelných souborů řeší libos, jádro nahrává jen `init`.
- **Konec procesu**: `PROCESS_EXIT`, `PROCESS_KILL`, pád (výjimka CPU) nebo
  konec posledního vlákna. Všechna vlákna dostanou požadavek na ukončení
  (čekající skončí s `CANCELED`, běžící jinde se přeruší IPI), skončí při
  návratu do userspace. Poslední vlákno proces dokončí.
- **Paměť skončeného procesu se vrací hned** **[ověřeno v QEMU]** (test
  `job: pamet skonceneho procesu se vrati hned`, x86_64 i aarch64): při
  dokončení se zruší adresní prostor (mapování, objekty paměti, které
  drželo jen mapování, tabulky stránek včetně kořenové), zavřou se handly
  (kanály a další objekty) a vyprázdní fronty futexů; rozpočty jobu se
  vrátí. Teprve potom proces dostane signál `TERMINATED`, takže kdo na něj
  čeká, už paměť vidí vrácenou. Kdo drží handle skončeného procesu (rodič,
  supervisor), drží jen malý **náhrobek**: číslo, jméno, kód ukončení, čas
  CPU a signály pro `PROCESS_EXIT_CODE`, `JOB_PROCESSES` a čekání. Ten
  zanikne s posledním handlem.
- **Pořadí kvůli souběhům:** vlákno při konci nejdřív přepne CPU na
  adresní prostor jádra a plánovač mu prostor procesu už nenahraje; teprve
  pak sníží počet živých vláken. Když poslední vlákno skončí, žádné CPU
  prostor procesu aktivní nemá a jádro ho smí zrušit (po vyprázdnění TLB
  na všech CPU). Syscall jiného vlákna konec nepředběhne: proces se
  dokončí, až všechna jeho vlákna syscall opustila. Operace jiných procesů
  přes handle (`VM_MAP` -> `BAD_STATE`, `VM_UNMAP` -> `NOT_FOUND`,
  `THREAD_CREATE` -> `BAD_STATE`) se serializují zámkem prostoru.
- **Paměť pro DMA bez IOMMU** (`VMO_CREATE_CONTIGUOUS` bez domény zařízení)
  se při konci procesu nevrací: zařízení do ní může dál zapisovat podle
  deskriptorů mrtvého ovladače. Drží ji náhrobek a vrátí se až s posledním
  handlem procesu (v logu varování). S IOMMU se vrací hned, protože se
  stránky nejdřív odmapují z domény zařízení. Úplné řešení (zastavit
  zařízení, vypnout bus master, a pak paměť vrátit) je **[navrženo]**.
- **Výjimka v programu** (page fault, neplatná instrukce, ...) ukončí jen
  proces, ne jádro: kód -1 000 100 - vektor, v logu adresa a typ chyby.
- **Vstup do userspace:** vstupní bod a zásobník vlákna musí ležet v paměti
  procesu (`USER_MIN`..`USER_MAX`); poslední stránka spodní poloviny
  adresního prostoru je vždy nenamapovaná, takže návrat ze syscallu nikdy
  nemíří na nekanonickou adresu (ta by vyvolala výjimku ještě v jádře).
- **Registr ukazatele vlákna** (aarch64 `TPIDR_EL0`, který si program
  smí zapsat sám): plánovač ho při přepnutí uloží do odcházejícího vlákna
  a nahraje hodnotu nového; nové vlákno začíná s 0. Hodnota jednoho
  procesu tak nikdy není vidět v jiném **[ověřeno v QEMU]** (test
  `procesy: TPIDR_EL0 neprosakuje mezi procesy`, aarch64). Na x86_64 si
  program `FS_BASE` sám nastavit nemůže.
- **Stav FPU/SSE:** každé vlákno má vlastní 512B oblast FXSAVE; přepnutí
  kontextu ji vždy uloží a obnoví. Nové vlákno začíná s výchozím stavem
  (FCW 0x037F, MXCSR 0x1F80), takže data jiného vlákna nemohou uniknout.
  Jádro samo FPU/SSE nepoužívá (soft-float). AVX není zapnuté (instrukce
  skončí výjimkou #UD).
- **Rozpočty a meze:** paměť jádra a uživatelská paměť se strhává
  z rozpočtu jobu procesu (strom jobů, limity v bajtech), rezerva paměti
  jádra a strukturální meze - viz
  [syscally.md](syscally.md#joby-rozpočty-prostředků).

## IOMMU (ochrana DMA)

Zařízení zapisuje do paměti samo (DMA), na adresy, které mu ovladač napíše
do deskriptorů. Bez IOMMU tak chybný ovladač může nechat zařízení přepsat
jádro. Jádro proto zapíná IOMMU, pokud ji stroj má: **Intel VT-d** (tabulka
ACPI DMAR) nebo **AMD-Vi** (tabulka ACPI IVRS), na aarch64 **SMMUv3**
(tabulka ACPI IORT). Všechny fungují stejně a zbytek jádra je nerozlišuje
(`arch::iommu::Domain`):

<figure class="diagram">
<svg viewBox="0 0 760 230" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="IOMMU">
<defs><marker id="io1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker></defs>
<rect class="box" x="10" y="30" width="150" height="60" rx="6"/><text class="t b" x="85" y="55" text-anchor="middle">zařízení 00:03.0</text><text class="m" x="85" y="74" text-anchor="middle">DMA na adresu X</text>
<path class="ln" d="M160 60 L218 60" marker-end="url(#io1)"/>
<rect class="acc" x="220" y="20" width="200" height="80" rx="6"/><text class="t b" x="320" y="45" text-anchor="middle">IOMMU (VT-d / AMD-Vi)</text><text class="m" x="320" y="64" text-anchor="middle">kořen → kontext 00:03.0</text><text class="m" x="320" y="81" text-anchor="middle">→ doména → tabulky stránek</text>
<path class="ln" d="M420 45 L498 45" marker-end="url(#io1)"/>
<rect class="ok" x="500" y="20" width="250" height="50" rx="6"/><text class="t" x="625" y="42" text-anchor="middle">X je v doméně (DMA paměť ovladače)</text><text class="m" x="625" y="60" text-anchor="middle">→ přístup do RAM povolen</text>
<path class="ln" d="M420 80 L498 120" marker-end="url(#io1)"/>
<rect class="err" x="500" y="95" width="250" height="60" rx="6"/><text class="t" x="625" y="118" text-anchor="middle">X mimo doménu</text><text class="m" x="625" y="136" text-anchor="middle">→ zablokováno + přerušení → log jádra</text>
<text class="m" x="10" y="190">Zařízení bez domény (žádný ovladač) nesmí DMA vůbec. Doména vznikne z omezeného Resource</text>
<text class="m" x="10" y="208">s identitou zařízení; mapuje se jen paměť z `dma_memory` téhož Resource (adresa pro zařízení = fyzická).</text>
</svg>
<figcaption>Překlad DMA: každé zařízení smí jen do své domény.</figcaption>
</figure>

- **Výchozí stav: zakázáno.** Po zapnutí nemá žádné zařízení doménu, takže
  DMA neprojde (zařízení bez ovladače nic nezapíše).
- **Doména zařízení** vznikne, když správce zařízení vytvoří omezený
  Resource s identitou zařízení (`RESOURCE_CREATE` s argumentem `device`,
  v libos `Resource::restrict_device`). Více prostředků téhož zařízení
  sdílí jednu doménu.
- **Mapuje se jen DMA paměť** z tohoto prostředku (`VMO_CREATE_CONTIGUOUS`),
  identicky: adresa pro zařízení je fyzická adresa, ovladač se nemění. Při
  zrušení objektu paměti se stránky z domény odmapují dřív, než se vrátí
  alokátoru.
- **Zablokovaný přístup** IOMMU nahlásí přerušením (vektor 36) a jádro ho
  zaloguje jako varování se zařízením, adresou a kódem důvodu (viz
  [hlasky.md](hlasky.md#jádro-iommu)).
- **Bez IOMMU** (stroj ji nemá, má ji vypnutou ve firmwaru, nebo
  `cargo xtask run --iommu none`) jádro zaloguje varování a DMA není
  omezené.
- **Známé riziko bez IOMMU: zařízení po smrti ovladače.** Jádro dnes
  zařízení nezastaví, když jeho ovladač skončí; zařízení může dál
  zapisovat podle deskriptorů mrtvého ovladače. Paměť pro DMA proto drží
  náhrobek procesu ovladače (viz [Procesy a vlákna](#procesy-a-vlákna)).
  Jakmile ale zanikne poslední handle procesu (např. devmgr ho zahodí při
  restartu), paměť se vrátí alokátoru a zápis zařízení může poškodit data
  jiného procesu nebo jádra. Oprava **[navrženo]**: při zániku posledního
  prostředku zařízení (nebo konci ovladače) vypnout bus master v konfiguraci
  PCI, počkat na dokončení rozběhnutých přenosů, a teprve pak paměť vrátit.
- Ovladač virtio musí přijmout vlastnost `VIRTIO_F_ACCESS_PLATFORM`
  (zařízení pak používá překlad); virtio-blk to dělá.

Postup ovladače (nic navíc proti systému bez IOMMU):

```rust
// devmgr: prostředek jen pro zařízení 00:03.0 (BARy, konfigurace, kvóty)
let res = root.restrict_device(&ranges, 8, 4 << 20, (0, 0x00, 0x03, 0))?;
// ovladač: DMA paměť se namapuje do domény zařízení
let dma = res.dma_memory(64 * 1024)?;
let phys = res.phys_addr(&dma, 0)?; // = adresa pro zařízení
```

Implementace v `kernel/src/arch/x86_64/iommu/`:

| Soubor | Co dělá |
|---|---|
| `mod.rs` | společné rozhraní: `init`, `domain(segment, sběrnice, funkce)`, `Domain::map/unmap`, obsluha přerušení chyb |
| `vtd.rs` | Intel VT-d: kořenová tabulka → tabulky kontextu sběrnic → stránkové tabulky domény (3 nebo 4 úrovně podle jednotky); po každé změně globální zneplatnění cache kontextu a IOTLB přes registry; chyby z registrů záznamů chyb |
| `amdvi.rs` | AMD-Vi: tabulka zařízení (65 536 záznamů, výchozí „vše zablokováno“) → stránkové tabulky domény (4 úrovně); zneplatnění přes frontu příkazů s čekáním na dokončení; chyby ze záznamu událostí (přerušení MSI nastavené v konfiguraci PCI samotné IOMMU) |

Na aarch64 je to `kernel/src/arch/aarch64/smmu.rs` (SMMUv3): dvouúrovňová
tabulka streamů (StreamID = RID podle IORT, neplatná položka = DMA
zablokované) → STE s konfigurací stage 1 → kontextový deskriptor s vlastním
ASID a tabulkami stránek ve formátu CPU; zneplatnění přes frontu příkazů
(CFGI_STE, TLBI_NH_ASID, SYNC); chyby z fronty událostí se vypisují při
operacích s doménami. Zprávy MSI zařízení procházejí SMMU, proto se do
domény zařízení mapuje i stránka ITS (GITS_TRANSLATER).

Výpis chyb je omezený: jeden zablokovaný přenos může vyvolat desítky
událostí, podrobně se vypíšou první 4 a zbytek souhrnem.

Zatím chybí: překlad přerušení, fronta zneplatnění VT-d (QI), oblasti RMRR
a IVMD (paměť, kterou firmware používá i po startu, např. starší USB) a
zařízení za mosty PCI vyjmenovaná v tabulkách.

## Syscally

Vstup instrukcí `syscall`, návrat přes `iretq` (bez nástrah `sysret`;
optimalizace později). Rozhraní: [syscally.md](syscally.md), chyby:
[chyby.md](chyby.md).

## Logování

Záznam:

```text
[    0.247482] cpu0 W tests::log: zprava (kernel/src/tests/log.rs:35)
 čas od startu  CPU úroveň modul       text          místo (jen W a E)
```

- Úrovně: `E` chyba, `W` varování, `I` info, `D` ladění, `T` trasování.
- Výstupy: sériová linka (vše povolené), framebuffer (do Info), kruhový
  buffer 64 KiB (`log::snapshot`, pro budoucí službu logu).
- **Framebuffer pro userspace:** když ho firmware dal (a fyzická adresa je
  zarovnaná na stránku), init dostane ve startovní zprávě objekt paměti
  `bootstrap::kind::FRAMEBUFFER` (geometrie z `BootInfo`, `FB_CONTROL`
  `INFO`). Init ho předá jedinému procesu - textové a záchranné konzoli
  (`user/console`). Převzetí je výslovné (`FB_CONTROL` `CLAIM`): jádro pak
  log do framebufferu nekreslí, na sériovou linku jde dál. `RELEASE` ho
  jádru vrátí (obrazovka se smaže); totéž udělá konec (i pád) procesu,
  který ho převzal. Bez framebufferu položka chybí.
- **Údaje ACPI pro userspace:** init dostane objekt paměti
  `bootstrap::kind::ACPI` (formát `abi::acpi::Info`): adresu a kopii RSDP
  (zkopírovanou při startu - RSDP může ležet v paměti, kterou jádro pak
  uvolní) a oblasti paměti firmwaru s tabulkami (`AcpiReclaimable`,
  `AcpiNvs`). Init je předá službě `acpid`, která v nich smí mapovat
  (`VMO_CREATE_PHYSICAL` s cache) - viz [ACPI](acpi.md). Jádro samo čte jen
  MADT, MCFG, HPET a tabulky IOMMU.
- Filtr: globální úroveň (`log::set_level`), úroveň pro modul a jeho
  podmoduly (`log::set_target_level("sched", Level::Trace)`). Release build
  vůbec nepřeloží Trace (feature `log-trace` ho vrátí). Vypnutá úroveň stojí
  jedno atomické čtení. Logování nealokuje, funguje i v přerušení.
- Chyby s kontextem: `result.context("co se dělalo")` zaloguje chybu
  i s místem volání (Debug), `warn_context` totéž jako varování.
- Programy logují přes `DEBUG_LOG` (`println!` v libos); záznam má prefix
  `[jméno id]`.
- Seznam hlášek: [hlasky.md](hlasky.md).

## Testy

- **Jádro** (`kernel/src/tests/`): paměť, platforma, log, plánovač, objekty,
  mechanismy pro userspace. Běží ve vlákně `boot` při každém startu.
- **Userspace** (`user/init/src/tests/`): syscally přes libos včetně
  chybových kódů, paměť, IPC v procesu, procesy (spawn, ping-pong s `echo`,
  předání handlu, kódy, kill, pád, druhé vlákno).
- **Knihovny na hostiteli**: `cargo test -p abi -p initfs --features initfs/std`
  (formát initfs, ABI, úplnost dokumentace chyb a syscallů).
- `cargo xtask test` spustí vše a vrátí výsledek jako návratový kód.
