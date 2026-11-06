# Architektura

Živý dokument: co stavíme, jak a proč. Každé rozhodnutí má uvedený důvod
a cenu. Když se rozhodnutí změní, upraví se tady spolu s kódem.

## Principy

1. **Čisté jádro.** Jádro o Windows ani jiném systému nic neví.
2. **Žádné ohýbání kvůli kompatibilitě.** Když Windows vrstva potřebuje něco,
   co by nativní systém sám nechtěl, řeší se to ve vrstvě, ne v jádře.
   Do jádra se přidává jen obecný mechanismus, který dává smysl i nativně.
3. **Capabilities.** Žádný root, žádné ACL. Proces má přístup jen k tomu,
   na co dostal handle.
4. **Asynchronní I/O od základu**, synchronní API je tenká vrstva nad ním.
5. **Ovladače v userspace.** Pád ovladače neshodí systém.
6. **Jen moderní platformy:** 64 bit, UEFI / device tree, ACPI, SMP.
7. **UTF-8 všude.**
8. **Přenositelnost:** architektura v `arch/<arch>/`, primárně x86_64.

## Vrstvy

```
+--------------------------------------------------------------+
|  Nativní aplikace            |  Windows aplikace (.exe)      |
|                              |  Windows vrstva (sandbox)     |
|  libos (nativní API, C ABI)  |  libos                        |
+--------------------------------------------------------------+
|  Systémové služby (userspace): init, device manager,         |
|  ovladače, storage + FS, namespace, kompozitor, konfigurace  |
+--------------------------------------------------------------+
|  Mikrokernel: paměť, plánovač, IPC, přerušení, objekty,      |
|  handly + práva, asynchronní fronty                          |
+--------------------------------------------------------------+
|  arch/x86_64 (později arch/aarch64)  |  UEFI loader          |
+--------------------------------------------------------------+
```

## Jádro

### Typ: mikrokernel

V jádře jen paměť, plánovač, IPC, přerušení a objekty s handly.

- **Proč:** ovladače v userspace a capabilities vedou přirozeně
  k mikrokernelu (seL4, Zircon, QNX). Hybrid otevírá cestu výjimkám
  "kvůli výkonu" (win32k ve Windows).
- **Cena:** víc přepínání kontextu. Řešení: zero-copy přes sdílenou paměť,
  fronty bez syscallu na operaci, rychlá cesta pro synchronní volání
  (přímé přepnutí na příjemce jako call/reply v seL4).

### Objekty, handly, práva

Vše, na co lze ukázat, je kernelový objekt. Proces k němu přistupuje přes
handle (index do své handle tabulky). Handle nese práva (bitmaska: `READ`,
`WRITE`, `EXECUTE`, `MAP`, `DUPLICATE`, `TRANSFER`, `WAIT`, `SIGNAL`, ...).
Při duplikaci lze práva jen ubírat.

| Objekt | Význam |
|---|---|
| `Process` | adresní prostor + handle tabulka |
| `Thread` | vlákno |
| `MemoryObject` | stránky paměti, lazy alokace, copy-on-write klony |
| `AddressRegion` | část adresního prostoru, mapování |
| `Channel` | obousměrná fronta zpráv (bajty + handly) |
| `Port` | fronta dokončení a událostí |
| `Event`, `Timer` | synchronizace |
| `Interrupt` | IRQ doručované ovladači |
| `MmioRange`, `IoPorts`, `DmaRegion` | přístup ovladačů k HW |
| `Job` | skupina procesů s limity |

Jádro nemá globální jmenný prostor. Jména řeší služba namespace
v userspace, každý proces vidí jen svůj strom (Plan 9).

Každý objekt má **signály** (bitmaska stavů). Čekání: `wait_one`,
`wait_many`, nebo registrace na `Port`, kam při změně signálu přijde paket.

### Asynchronní I/O

Proces si vytvoří ring: submission a completion frontu ve sdílené paměti
(styl io_uring). Syscall slouží jen k probuzení a uspání. Synchronní
funkce v libos = vložit požadavek a počkat na dokončení.

### IPC

- `Channel`: zpráva do 64 KiB dat a 64 handlů. Handly se zprávou přesouvají.
- Velká data: pošle se handle na `MemoryObject` (zero-copy).
- Rozhraní služeb popisuje jednoduchý vlastní IDL, z něj se generuje kód.

### Paměť

- Fyzická paměť: buddy alokátor nad mapou paměti od loaderu.
- Virtuální: 4úrovňové stránkování, kernel v horní polovině, přímá mapa
  fyzické paměti od `0xFFFF_8000_0000_0000`, kernel od `0xFFFF_FFFF_8000_0000`.
- `MemoryObject`: lazy alokace, copy-on-write klony.
- Kernel heap: slab pro objekty pevné velikosti + obecný heap.

### Plánovač

- Per-CPU fronty, work stealing, SMP od milníku 3.
- Tickless (APIC one-shot, později TSC deadline) kvůli spotřebě: časovač se
  nastavuje jen na konec kvanta nebo nejbližší probuzení, nečinné CPU spí
  bez přerušení.
- QoS třídy místo surových priorit: `realtime`, `interactive`, `normal`,
  `background`. Na big.LITTLE se mapují na typ jader. Zatím přísná priorita
  mezi třídami a round-robin uvnitř (kvantum 10 ms); později férovější
  algoritmus (EEVDF), aby nižší třídy nehladověly.
- Rozkládání zátěže: nová a probuzená vlákna jdou na nečinné CPU,
  vystřídané vlákno se přesune na nečinné CPU, nečinné CPU si práci bere.
- Probuzení: každé čekání má pořadové číslo, probuzení se starým číslem se
  ignoruje; plané probuzení je dovolené a čekající kód podmínku ověří znovu.
- Preempce vlákno nikdy neuspí ani neukončí, jen ho vrátí do fronty. Vlákno
  může být přerušeno kdekoli, třeba uprostřed přípravy na čekání.
- Hodiny: TSC kalibrovaný proti HPET (předpoklad: invariantní TSC
  synchronizovaný mezi CPU).

### Syscall ABI a libos

**libos** je knihovna v userspace, kterou má každý proces (obdoba `ntdll`
ve Windows nebo vDSO + libc v Linuxu).

- Stabilní ABI je funkce v libos, ne číslo syscallu. Jádro tak může syscally
  měnit bez rozbití aplikací.
- Obsahuje synchronní API nad asynchronními frontami a pomocné funkce pro IPC.
- Exportuje C ABI, interně je v Rustu. Windows vrstva je jen další klient.

## Bezpečnost

- Init dostane kořenové capabilities a rozdává je službám, každé jen jejich část.
- Uživatel je pojem userspace: session = sada capabilities.
- Aplikace jsou vždy v sandboxu; manifest říká, jaké capabilities potřebují.
- Přístup k souboru = handle na adresář, ve kterém leží. Žádná ACL.

## Systémové služby

| Služba | Úkol |
|---|---|
| init / supervisor | spouští služby, restartuje padlé |
| device manager | enumerace PCI, ACPI, device tree; spouští ovladače s jejich prostředky |
| ovladače | nejdřív virtio, pak NVMe, xHCI, HDA, ... |
| ACPI | interpret AML; jádro samo parsuje jen statické tabulky (MADT, HPET, MCFG) |
| storage + FS | blokové zařízení, oddíly, souborový systém |
| namespace | skládá pohled procesu na strom |
| kompozitor | displej, vstup, okna |
| konfigurace | čtení, validace a verzování nastavení |

## Souborový systém

- Vlastní copy-on-write FS: B-stromy, checksumy, snapshoty, subvolumy.
- Atomické A/B aktualizace: systém je read-only subvolume, aktualizace
  vytvoří novou verzi, při selhání bootu návrat ke staré.
- Cesty: oddělovač `/`, UTF-8, bez limitu délky, bez písmen disků.
- Do vlastního FS: **initfs** (read-only archiv nahraný loaderem do paměti).

### Jména souborů: case-insensitive globálně

Dvě jména, která člověk čte jako stejná, jsou stejné jméno.

- **Case-preserving, case-insensitive:** soubor se jmenuje tak, jak ho
  uživatel pojmenoval, ale `Foo` a `foo` vedle sebe existovat nemohou.
- **Unicode normalizace NFC:** `é` jako jeden znak a `e` + kombinující čárka
  jsou stejné jméno.
- **Nezávislé na jazyku:** Unicode simple case folding, žádná pravidla podle
  locale (turecké `i`). Verze tabulky se ukládá do FS, aby nová verze
  Unicode nezměnila význam existujících jmen.
- **Cena:** cizí software se jmény lišícími se jen velikostí písmen (některé
  git repozitáře) nejde uložit. Případná volba case-sensitive pro jeden
  subvolume se přidá, až bude opravdu potřeba.

## Grafika

- Kompozitor v userspace (koncept Waylandu), klienti renderují do bufferů
  a předávají je přes channel.
- Vulkan-first, HiDPI se zlomkovým škálováním, dotyk jako výchozí vstup.
- Ovladače GPU:
  - userspace část (Mesa: Vulkan driver, kompilátor shaderů) se portuje
    přes vrstvu winsys,
  - kernelovou část (v Linuxu DRM: správa GPU paměti, odesílání příkazů,
    firmware) nelze jen přeložit, je hluboce svázaná s Linuxem; u nás bude
    jako vlastní userspace ovladač psaný podle linuxového kódu.
- Postup: virtio-gpu (Venus) v QEMU, pak jedna rodina reálných GPU
  (nejspíš Intel).

## Konfigurace

- Konfigurace psaná člověkem: **TOML** (komentáře, čitelnost), validace
  proti schématu v konfigurační službě.
- Verzovaná jako součást snapshotu FS, rollback zadarmo. Žádné registry.
- Stav zapisovaný programy si volí každá služba sama.

## Windows vrstva

Samostatná personalita v userspace nad libos. Dobré NT koncepty mají
nativní protějšek, protože jsou dobré samy o sobě, ne kvůli Windows:

| Windows | Nativně |
|---|---|
| HANDLE | handle (vrstva si drží vlastní tabulku kvůli NT sémantice) |
| WaitForMultipleObjects | `wait_many` |
| IOCP | `Port` |
| Section | `MemoryObject` |
| VirtualAlloc | `AddressRegion` |
| Job | `Job` |
| SEH | výjimka doručená na exception channel (obecný mechanismus i pro debuggery) |
| TEB přes GS | obecný syscall pro thread-local bázový registr (nativně TLS) |
| APC | jen ve vrstvě, přes alertable čekání |
| UTF-16 | převod na hranici vrstvy |
| `C:\...` | překlad na cestu v sandboxu |
| registry | soubor v sandboxu, obsluhuje služba vrstvy |

Každá aplikace má adresář s virtuálním `C:` a vlastními registry.
Odinstalace = smazání adresáře.

První cíl (milník 5): .exe se statickým CRT (`/MT`), které potřebuje jen
`kernel32` a `ntdll`. Hned potom vlastní UCRT.

## Cizí kód a licence

- Licence projektu `MIT OR Apache-2.0`.
- Převzít smíme cokoli (Mesa, DXVK, VKD3D, ...), kde psaní vlastního
  nedává smysl. Dlouhodobě směřujeme k vlastním komponentám.

## Platformy

1. QEMU x86_64 + OVMF (hlavní vývoj)
2. reálné x86_64 PC
3. QEMU aarch64 `virt`
4. Raspberry Pi, PinePhone (Pro)

aarch64 se portuje podle plánu milníků (milník 8). Do té doby se kód
důsledně dělí do `arch/`, aby port nebolel.

## Dočasná řešení

| Co | Proč | Kdy pryč |
|---|---|---|
| sériový ovladač v jádře | ladicí výstup před existencí userspace | zůstává jen jako early console |
| konzole na framebufferu v jádře | totéž | převezme kompozitor |

## Milníky

1. Boot: UEFI loader, kernel vypíše text (hotovo)
2. Základ jádra: GDT/IDT, výjimky, alokátor fyzické paměti, stránkování, heap (hotovo)
3. Procesy: plánovač, SMP, vlákna, objekty, handly, capabilities (hotovo)
4. Userspace: syscally, první proces, IPC, libos, initfs
5. Windows vrstva v1: PE loader, ntdll/kernel32, konzolové .exe
6. Systém: userspace ovladače (virtio), FS, grafika, kompozitor
7. Windows vrstva v2: user32/gdi32, GUI, DirectX přes Vulkan
8. Port na aarch64 a první ARM zařízení
