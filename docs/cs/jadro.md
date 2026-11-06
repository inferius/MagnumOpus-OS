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
  číst, zapisovat a mapovat; zatím alokované hned při vytvoření.
- **Adresní prostor procesu** (`mm/address_space.rs`): seznam mapování
  (objekt + offset + práva). Odmapování vyprázdní TLB na všech CPU
  (`mm/tlb.rs`) dřív, než se uvolní stránky.

## Plánovač

- Každé CPU má fronty vláken podle tříd QoS (`realtime`, `interactive`,
  `normal`, `background`): přísná priorita mezi třídami, round-robin s
  kvantem 10 ms uvnitř.
- **Tickless**: časovač se nastaví jen na konec kvanta nebo nejbližší
  probuzení; nečinné CPU spí v `hlt`.
- **Rozkládání**: nová a probuzená vlákna jdou na nečinné CPU; vystřídané
  vlákno se přesune na nečinné CPU; nečinné CPU si bere práci z cizích front.
- **Preempce** vlákno nikdy neuspí ani neukončí, jen ho vrátí do fronty -
  vlákno může být přerušeno kdekoli, i uprostřed přípravy na čekání.
- **Probuzení**: každé čekání má pořadové číslo; probuzení se starým číslem
  se ignoruje. Plané probuzení je dovolené, čekající kód podmínku ověří znovu.
- **Přepnutí na vlákno procesu** nastaví jeho adresní prostor (CR3)
  a zásobník jádra pro vstup z userspace (TSS.RSP0); vlákna jádra běží
  v adresním prostoru jádra.

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

## Procesy a vlákna

- Proces vzniká prázdný (`PROCESS_CREATE`), rodič do něj namapuje paměť
  a spustí první vlákno (`PROCESS_START`) s bootstrap kanálem. Formát
  spustitelných souborů řeší libos, jádro nahrává jen `init`.
- **Konec procesu**: `PROCESS_EXIT`, `PROCESS_KILL`, pád (výjimka CPU) nebo
  konec posledního vlákna. Všechna vlákna dostanou požadavek na ukončení
  (čekající skončí s `CANCELED`, běžící jinde se přeruší IPI), skončí při
  návratu do userspace. Poslední vlákno proces dokončí: zavřou se handly
  (uvolní objekty) a proces dostane signál `TERMINATED`. Adresní prostor
  zanikne s poslední referencí na proces.
- **Výjimka v programu** (page fault, neplatná instrukce, ...) ukončí jen
  proces, ne jádro: kód -1 000 100 - vektor, v logu adresa a typ chyby.
- **Vstup do userspace:** vstupní bod a zásobník vlákna musí ležet v paměti
  procesu (`USER_MIN`..`USER_MAX`); poslední stránka spodní poloviny
  adresního prostoru je vždy nenamapovaná, takže návrat ze syscallu nikdy
  nemíří na nekanonickou adresu (ta by vyvolala výjimku ještě v jádře).
- **Stav FPU/SSE:** každé vlákno má vlastní 512B oblast FXSAVE; přepnutí
  kontextu ji vždy uloží a obnoví. Nové vlákno začíná s výchozím stavem
  (FCW 0x037F, MXCSR 0x1F80), takže data jiného vlákna nemohou uniknout.
  Jádro samo FPU/SSE nepoužívá (soft-float). AVX není zapnuté (instrukce
  skončí výjimkou #UD).
- **Limity:** handly, vlákna, mapování, fronty kanálů a rezerva paměti
  jádra - viz [syscally.md](syscally.md#limity-prostředků).

## IOMMU (ochrana DMA)

Zařízení zapisuje do paměti samo (DMA), na adresy, které mu ovladač napíše
do deskriptorů. Bez IOMMU tak chybný ovladač může nechat zařízení přepsat
jádro. Jádro proto zapíná IOMMU, pokud ji stroj má: **Intel VT-d** (tabulka
ACPI DMAR) nebo **AMD-Vi** (tabulka ACPI IVRS). Obě fungují stejně a zbytek
jádra je nerozlišuje (`arch::iommu::Domain`):

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
