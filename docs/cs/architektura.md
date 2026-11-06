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

### Plynulost uživatelského rozhraní (plán)

Aby se UI nesekalo, nestačí jedno „UI vlákno“ - plynulost musí zajistit
celý řetěz od vstupu po displej:

- **QoS `interactive`** pro kompozitor a vlákna okna s fokusem: krátká
  kvanta, přednostní probuzení; práce na pozadí (commit FS, defragmentace,
  indexování, scrub) v `background`.
- **Dědění priority přes IPC:** když UI vlákno čeká na odpověď služby
  (fsd, ovladač), služba zprávu zpracuje s jeho QoS. Bez toho priorita
  končí u první služby a mikrojádro se seká právě tam.
- **Priority I/O:** požadavky na disk nesou třídu; čtení pro UI předběhne
  zápis defragmentace. fsd nikdy neblokuje čtení kvůli commitu.
- **Termíny snímků:** kompozitor plánuje podle vsync (deadline), ne podle
  pořadí ve frontě.
- **Aplikace:** UI vlákno jen kreslí a reaguje; práci dělá asynchronně
  (libos má asynchronní I/O jako základní model).

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
| storage + FS | blokové zařízení, oddíly, souborový systém (`fsd` hotovo) |
| namespace | skládá pohled procesu na strom |
| kompozitor | displej, vstup, okna |
| konfigurace | čtení, validace a verzování nastavení |

## Souborový systém

Vlastní copy-on-write FS je hotový (formát v2); podrobný popis včetně
formátu na disku, algoritmů a příkladů je v [souborovy-system.md](souborovy-system.md).
Ve zkratce:

- jeden CoW B+strom pro metadata, Merkleův strom s 128bitovými součty
  (XXH3-128 nebo BLAKE3) v každém ukazateli, kruh 8 superbloků a návrat až
  o 3 generace při poškozeném nejnovějším stavu,
- extenty s odloženou alokací (souvislé zápisy), alokační skupiny se space
  logy, odložené znovupoužití bloků, TRIM, defragmentace na pozadí,
- intent log pro rychlý `fsync`, snapshoty s mazáním přes deadlisty,
  okamžité velikosti adresářů, fsck a scrub,
- služba `fsd` v userspace nad ovladačem disku, protokol `fsproto`.

Dál podle návrhu:

- Atomické A/B aktualizace: systém je read-only subvolume, aktualizace
  vytvoří novou verzi, při selhání bootu návrat ke staré (potřebuje
  zapisovatelné klony - plán).
- Cesty: oddělovač `/`, UTF-8, bez písmen disků.
- Programy při startu systému zatím nahrává loader z **initfs** (read-only
  archiv v paměti); datový disk má vlastní FS.

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

### Obsahově adresovaný FS a nativní verzování (návrh)

Git je v jádru jednoduchý: soubory a adresáře identifikované hashem obsahu
(Merkleho strom), commit = neměnný strom + metadata + rodiče, větev =
pojmenovaný ukazatel na commit. CoW FS s checksumy už má většinu
z toho - při každé změně stejně přepisuje cestu ke kořeni. Návrh: dotáhnout
to tak, aby verzování bylo vlastnost FS, ne programu nad ním.

- **Hash obsahu u každého souboru a adresáře**, udržovaný FS při zápisu
  (Merkleho strom, BLAKE3 - rychlý a umí hashovat po částech). Zapnutelné
  pro subvolume, protože něco stojí.
- **Snapshot = commit:** neměnný kořen + autor, zpráva, rodiče. Historie je
  DAG, větve jsou pojmenované reference, nová větev nebo klon je CoW
  (okamžitý, bez kopírování dat).
- **Rozdíl dvou snapshotů v čase úměrném změně:** stejný hash podstromu =
  není třeba do něj lézt.
- **Žurnál změn:** uspořádaný proud změn v subvolume (jako USN žurnál NTFS
  nebo FSEvents). Pro build systémy, synchronizaci, indexování, `git status`
  bez procházení disku.
- **Transakce:** atomický zápis více souborů najednou (commit nebo nic).
- **Deduplikace** zadarmo díky hashům obsahu.
- **Programové API:** vedle cest i typované IPC rozhraní: transakce, dotazy
  na metadata, hashe, reflink kopie, rozdíly snapshotů, odběr změn.

**Interoperabilita s gitem:** git počítá ID objektů vlastním formátem (SHA-1
nebo SHA-256 přes `blob <velikost>\0obsah`), ty z BLAKE3 odvodit nejde.
Služba `git` proto ID počítá líně a cachuje je v metadatech souborů. Pracovní
strom je přímo FS (žádný oddělený index - stav zná FS ze žurnálu),
objekty repozitáře jsou snapshoty, packfily jen pro síťovou výměnu. Pro
existující nástroje adaptér na úložiště objektů (např. backend libgit2).

**Pořadí:** nejdřív samotný CoW FS (milník 6), pak hashe a DAG snapshotů,
pak služba pro git. Zbytek návrhu se upřesní při implementaci.

### Metadata a vyhledávání (návrh)

Cíl: velikost, typ a časy čehokoli okamžitě, výpis velké složky bez
čekání a hledání bez procházení disku.

- **Tabulka objektů (obdoba MFT v NTFS, ale jako B-strom):** každý soubor
  a adresář je záznam s pevným ID: velikost, časy, typ, hash obsahu, seznam
  extentů. Malé soubory (do ~3 KiB) leží přímo v záznamu, takže jejich
  čtení je jedna operace. B-strom místo pevného pole jako u MFT: roste
  i zmenšuje se, nefragmentuje se a snapshoty sdílí nezměněné uzly (CoW).
- **Adresáře** jsou B-stromy klíčované normalizovaným jménem (NFC +
  case folding) - vyhledání v O(log n). Položka adresáře nese i kopii
  nejčastějších metadat (velikost, typ, čas změny), takže výpis složky
  nemusí skákat do tabulky objektů (jako indexové položky NTFS).
- **Souhrny podstromu v každém adresáři:** celková velikost, počet souborů,
  poslední změna. Velikost složky je tak k dispozici okamžitě (žádné `du`).
  Aktualizují se při CoW zápisu, který cestu ke kořeni přepisuje stejně.
- **Indexy pro hledání**, udržované v téže transakci jako změna: jméno
  (n-gramy, hledání části jména), typ/přípona, velikost, čas změny, štítky
  a typované uživatelské atributy (klíč = hodnota). Dotaz přes API, např.
  "jméno obsahuje `faktura`, velikost > 1 MiB, změněno po 1. 1." bez
  procházení disku. Indexy jsou volitelné pro subvolume (stojí zápis).
- **Fulltext** (hledání v obsahu) je služba v userspace nad žurnálem změn,
  aby nezpomalovala zápis.

Další vylepšení, se kterými se počítá:

- průhledná komprese po extentech (zstd, lz4) podle typu dat,
- šifrování subvolume (klíč jako capability, ne heslo v FS),
- samoopravné ukládání: checksumy + druhá kopie metadat (a volitelně dat),
  při chybě čtení oprava z kopie,
- víc disků v jednom poolu (zrcadlení, rozšiřování za běhu),
- kvóty a rezervace pro subvolume,
- historie souboru zadarmo z DAG snapshotů (časová osa verzí, "undo"),
- start bez kontroly disku (CoW + transakce = disk je vždy konzistentní).

## Aplikace: instalace, aktualizace, odinstalace (návrh)

Cíl: jednoduché jako přenosná aplikace ve Windows, bezpečné jako sandbox na
mobilu, bez závislostního pekla linuxových balíčkovačů.

- **Jedna aplikace = jeden adresář** `/apps/<id>/` (vlastní subvolume):
  program, jeho knihovny a data, která k němu patří. Žádné sdílené
  knihovny mezi aplikacemi, žádné zápisy jinam. Uživatelská data aplikace
  leží v `/home/<uživatel>/apps/<id>/`.
- **Odinstalace = smazání těchto dvou adresářů.** Nic jiného v systému
  nezůstane, protože aplikace nikam jinam zapisovat nemůže (dostává jen
  capabilities ke svým adresářům).
- **Balíček** je jeden soubor (archiv + manifest): jméno, verze, vydavatel,
  podpis, požadované capabilities (síť, kamera, soubory uživatele, ...).
  Instalace = ověřit podpis, ukázat požadovaná oprávnění, rozbalit. Funguje
  dvojklikem na stažený soubor stejně jako z katalogu.
- **Oficiální katalog** je jen jeden ze zdrojů: služba, která umí hledat,
  stahovat a hlídat aktualizace. Aplikace mimo katalog jdou instalovat
  taky, systém jen jasně řekne, kdo je podepsal.
- **Aktualizace jsou atomické:** nová verze se rozbalí do nového snapshotu
  subvolume a přepne se najednou; když nová verze nenaběhne, vrátí se
  předchozí. Uživatel volí: automaticky, upozornit, nebo nikdy. Žádné
  aktualizace během práce, žádné restarty kvůli aplikacím.
- **Systém se aktualizuje stejně** (A/B subvolume), jen má vlastní zdroj.
- Windows aplikace dostanou totéž: instalace do vlastního adresáře s
  virtuálním `C:`, odinstalace = smazání.

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

## Chyby a logování

Cíl: každou chybu jde dohledat - co se stalo, kde, na kterém CPU a kdy.

- **Chyby:** jeden typ pro jádro i userspace (`libs/abi`), stabilní
  číselné kódy, které jsou zároveň návratovými kódy syscallů. Kontext se
  loguje v místě vzniku. Všechny kódy: [chyby.md](chyby.md).
- **Log jádra:** úrovně, čas, CPU, modul, místo ve zdrojáku; filtr globálně
  i pro moduly; kruhový buffer; bez alokace. Podrobnosti:
  [jadro.md](jadro.md#logování), hlášky: [hlasky.md](hlasky.md).
- **Fatální chyby:** panika i výjimka vypíšou zprávu, registry a zásobník;
  xtask adresy přeloží na jména funkcí.
- Později: služba logu v userspace, strukturované záznamy (klíč = hodnota),
  persistentní log na disku, úrovně z příkazové řádky jádra.

## Konvence kódu

Viz [sestaveni.md](sestaveni.md#konvence-kódu). Komentáře v kódu česky,
dokumentace česky i anglicky (`docs/cs`, `docs/en`).

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

## Jazyky a překladače (plán)

Dohodnutý postup (2026-11-06):

1. **Rust:** vlastní cíl `x86_64-unknown-magnumopus` a port `std` nad
   libos (vlákna, TLS, synchronizace, soubory, procesy, čas).
2. **C:** vlastní libc nad libos (pod naší kontrolou, bez POSIX dědictví,
   kde není potřeba); překlad clangem na hostiteli.
3. **C++:** libc++, libc++abi a libunwind nad naší libc, překlad clangem.
4. **Překladač běžící v systému:** nejdřív port TinyCC (malý, rychlý
   překlad C), později clang/LLVM.

### Vlastní překladač (dlouhodobý cíl)

Do budoucna chceme produkovat **optimální kód pro naši platformu**, proto
je v plánu i vlastní překladač. Není potřeba pro start (výše uvedené pokryje
C, C++ i Rust), ale dává smysl jako samostatný projekt:

- znalost našeho ABI, syscallů a formátu binárek - kód bez vrstev navíc
  (přímá volání libos, volání IPC bez obalů),
- optimalizace pro naše vzory: kanály a zprávy, capability handly, async I/O,
- nezávislost na cizím toolchainu (self-hosting),
- rozumný postup: nejdřív backend (vlastní IR + generování x86_64 a
  aarch64), který může používat i existující frontend (např. jako backend
  pro C nebo jako cranelift-like backend pro Rust), teprve pak vlastní
  frontend jazyka.

## Dočasná řešení

| Co | Proč | Kdy pryč |
|---|---|---|
| sériový ovladač v jádře | ladicí výstup před existencí userspace | zůstává jen jako early console |
| konzole na framebufferu v jádře | totéž | převezme kompozitor |

## Milníky

1. Boot: UEFI loader, kernel vypíše text (hotovo)
2. Základ jádra: GDT/IDT, výjimky, alokátor fyzické paměti, stránkování, heap (hotovo)
3. Procesy: plánovač, SMP, vlákna, objekty, handly, capabilities (hotovo)
4. Userspace: syscally, první proces, IPC, libos, initfs (hotovo)
5. Windows vrstva v1: PE loader, ntdll/kernel32, konzolové .exe
6. Systém: userspace ovladače (virtio - hotovo), FS (hotovo), grafika, kompozitor
7. Jazyky a překladač (viz [Jazyky a překladače](#jazyky-a-překladače-plán)):
   Rust (`std` pro náš OS), C (vlastní libc nad libos), C++ (libc++/libunwind
   nad libc), překlad clangem z hostitele, pak TinyCC v systému
8. Windows vrstva v2: user32/gdi32, GUI, DirectX přes Vulkan
9. Port na aarch64 a první ARM zařízení

## Aktuální odchylky od návrhu

Co zatím funguje jednodušeji, než návrh popisuje (a proč to nevadí teď):

- **Syscally jsou synchronní** a protokoly ovladačů a FS (`blockproto`,
  `fsproto` v1) kopírují data zprávou kanálu; fronty požadavků ve sdílené
  paměti (styl io_uring) přijdou, až je bude potřebovat výkon.
- **Objekty paměti alokují stránky hned** (žádné lazy alokace ani CoW klony).
- **Odmapování jen celých mapování.**
- **Návrat ze syscallu přes `iretq`** místo rychlejšího `sysret`.
- **Programy se zatím překládají bez SSE/FPU** (soft-float). Jádro už stav
  FPU/SSE každého vlákna ukládá (FXSAVE, x87 + XMM0-15); AVX (XSAVE) zapne
  milník jazyků.
- **IOMMU (Intel VT-d, AMD-Vi) jen pro překlad DMA:** chybí překlad
  přerušení a oblasti RMRR/IVMD; na stroji bez IOMMU DMA omezené není
  (jádro to zaloguje).
- **Kvóty jen na proces**, ne pro skupiny procesů (job).
- **Mezi třídami QoS přísná priorita** (nižší třídy mohou hladovět).
- **Paměť po ukončených vláknech** (zásobníky jádra) se používá znovu, ale
  nevrací se.
