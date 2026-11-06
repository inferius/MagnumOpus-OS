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
