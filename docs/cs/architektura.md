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
8. **Přenositelnost:** architektura v `arch/<arch>/` za pevným
   [rozhraním](#rozhraní-architektury), primárně x86_64.

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

Vlastní copy-on-write FS je hotový (formát v3 se zavádí po fázích - viz [verze formátu](fs-verze.md)); podrobný popis včetně
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
- **Cesta podobná Waylandu, ne X11:** kompozitor jen skládá buffery
  klientů; aplikace nevidí cizí okna ani vstup; snímání obrazovky
  a nahrávání jen přes capability (systém ukáže, kdo právě snímá).
- **Jedna nativní knihovna UI** (deklarativní, Rust) místo dvou
  konkurenčních jako GTK a Qt: jednotný vzhled, přístupnost a lokalizace
  pro všechny; vrstva Windows (`user32`/`gdi32`) se na ni mapuje.
- **Návrhy k odsouhlasení:**
  - **Okno ukazuje oprávnění:** ikona v titulku řekne, ke kterým složkám,
    síti nebo kameře má aplikace právě přístup; jedním klikem odebrat.
  - **Systémové dialogy nepodvrhnutelné:** dialog oprávnění nebo hesla
    kreslí kompozitor v oddělené vrstvě (aplikace ho nepřekreslí ani
    nenapodobí).
  - **Rozpracovaný stav aplikací přežije restart:** aplikace ukládá stav
    UI do vlastního datasetu; po aktualizaci nebo pádu se okna vrátí.
  - **Plynulost jako požadavek:** viz [Plynulost uživatelského
    rozhraní](#plynulost-uživatelského-rozhraní-plán) - termíny snímků,
    QoS a dědění priority přes IPC.

## Rozhodnutí k milníkům 6a-6d (odsouhlaseno)

| # | Otázka | Rozhodnutí |
|---|---|---|
| 1 | protokol terminálu | **vlastní nativní protokol** (typované záznamy, tabulky, odkazy, obrázky, průběh, výzvy); VT100/pty jen jako adaptér pro převzaté programy |
| 2 | jazyk shellu | **nový jazyk s typy**: bez implicitního dělení slov, chyba jako hodnota (kód + popis), `?` jako v Rustu; **skript nese manifest oprávnění**; POSIX `sh` zvlášť pro převzaté skripty |
| 3 | co posílá aplikace kompozitoru | **strom vrstev + strom přístupnosti** (obsah kreslí aplikace do bufferů, posun, průhlednost, animace a scroll dělá kompozitor; struktura okna - role, texty, stav - je součást protokolu) |
| 4 | dekorace oken | **kreslí systém**; vlastní vzhled jde, ale s pravidly níže |
| 5 | API akcí aplikací | **v základu GUI** (6c): aplikace v manifestu deklaruje akce s typovanými parametry a potřebnými oprávněními; volá je shell, průzkumník, asistent i agent |
| 6 | pořadí | **6a vstup -> 6b terminál -> 6c GUI -> 6d AI**; primitiva pro AI přicházejí průběžně s FS v3 a terminálem |

### Vstup: dva proudy a umělý vstup

- Aplikace dostává **fyzické klávesy** (kód nezávislý na rozložení - hry,
  zkratky) a zvlášť **vložený text** (z rozložení a metody zadávání: mrtvé
  klávesy, čínština, emoji, diktování). Rozložení a metody zadávání jsou
  jedna systémová služba, ne kód v každé aplikaci.
- `inputd` předává události jednomu vlastníkovi stanice (konzole, později
  kompozitor), ten je rozdělí oknům; bezpečnou zkratku zachytí `inputd`
  ještě před ním.
- **Umělý vstup** (automatizace, vzdálená plocha, AI agent) jen přes
  capability „vkládat vstup“; systém viditelně ukáže, že klávesnici ovládá
  někdo jiný.

### Terminál: líné undo a relace

- **Líný snapshot:** shell si u FS vyžádá snapshot až při první změně, kterou
  proces příkazu udělá - čtecí příkazy nestojí nic, každá změna jde vrátit.
- **Relace patří službě:** běžící příkaz přežije zavření i pád okna
  terminálu a jde se k němu připojit (i vzdáleně stejným strukturovaným
  protokolem, ne SSH s escape sekvencemi).
- Roura mezi procesy nese **binární proud záznamů se schématem**; program
  v manifestu deklaruje vstup a výstup, shell zná typy předem (doplňování,
  kontrola, převod na text až na konci).

### Vlastní vzhled okna (vlastní titulek)

Aplikace někdy chce okno po svém - karty v titulku (prohlížeč), vlastní
lišta (editor, přehrávač), okno bez rámu (hra, widget). Jde to, ale
důvěryhodné prvky zůstávají systému:

| Režim | Kdo kreslí | Co si systém nechá |
|---|---|---|
| **standardní** (výchozí) | rám a titulek systém | vše |
| **vlastní titulek** | aplikace kreslí i plochu titulku (karty, tlačítka, vyhledávání) | **odznak důvěry** (identita aplikace + co právě smí: složky, síť, kamera) na pevném místě, který aplikace nepřekreslí; **tlačítka okna** (zavřít, minimalizovat) kreslí systém, aplikace jen zvolí stranu a barvu; přesun a změna velikosti přes **deklarované oblasti** (aplikace řekne „tady se okno táhne“, „tady je tlačítko“) - kompozitor tak funguje i při zaseknuté aplikaci |
| **bez rámu / celá obrazovka** | vše aplikace | odznak se ukáže při najetí k okraji a vždy při změně oprávnění; bezpečná zkratka a systémové dialogy fungují vždy |

- **Systémové dialogy** (oprávnění, heslo, platba) kreslí kompozitor ve
  zvláštní vrstvě nad všemi okny, se ztlumením zbytku obrazovky a odznakem,
  který aplikace nemůže vykreslit (vrstvu nad ztlumením smí kreslit jen
  systém). Napodobenina v okně aplikace tak vždy vypadá jinak.
- Plocha titulku nese strom přístupnosti jako zbytek okna, takže vlastní
  titulek je přístupný stejně jako standardní.
- Podobně to dělá macOS (obsah až do titulku, tlačítka kreslí systém)
  a Windows (rozšíření obsahu do rámu); navíc je u nás odznak oprávnění
  a deklarované oblasti místo testování kliknutí v aplikaci.

## Vstup: klávesnice a myš (plán)

- **Služba vstupu** (`inputd`) v userspace: ovladače zařízení posílají
  události (klávesa, pohyb, tlačítko, kolečko, dotyk) jednotným protokolem
  přes kanál; služba je převádí podle rozložení klávesnice a předává
  **jen tomu, kdo má fokus** (terminál, kompozitor). Žádná aplikace
  nečte vstup globálně (ochrana proti keyloggerům).
- **Ovladače, postup:** virtio-input (klávesnice, myš, tablet) v QEMU -
  PS/2 jako záloha na starším hardwaru - USB HID přes vlastní ovladač
  xHCI (USB stack je velká samostatná práce).
- **Rozložení:** česká a anglická v první verzi, skládání diakritiky
  (mrtvé klávesy), Unicode vstup.
- **Bezpečná klávesová zkratka** (jako Ctrl+Alt+Del ve Windows): zachytí ji
  jen systém, aplikace ji nepodvrhne - pro odemčení, potvrzení v dialogu
  oprávnění a správce úloh.

## Shell a chytrý terminál (návrh)

> Odsouhlasená rozhodnutí: [Rozhodnutí k milníkům 6a-6d](#rozhodnutí-k-milníkům-6a-6d-odsouhlaseno).

Cíl: terminál, který je pohodlnější než dnešní a využívá, co umí náš FS
a capabilities.

| Funkce | Co dělá | Na čem stojí |
|---|---|---|
| **strukturovaný výstup** | příkazy vracejí tabulky a objekty (soubor, proces, síťové spojení), ne text; filtrování, řazení a sloupce bez `grep`/`awk`; převod na text jen pro výpis | protokol mezi shellem a programy (typovaný proud) |
| **undo každého příkazu** | před příkazem, který mění soubory, levný snapshot dotčeného datasetu; „vrať poslední příkaz“ | snapshoty, žurnál změn |
| **„ukaž, co se stane“** | nebezpečný příkaz (hromadné mazání, skript, instalace) běží v klonu složky, terminál ukáže diff, převezme se po potvrzení | klony, diff, převzetí změn (funkce „zkusit bokem“) |
| **oprávnění při spuštění** | shell ví z manifestu, co příkaz potřebuje (složka, síť), a předá jen to; nový nárok = dotaz | capabilities |
| **doplňování podle typů** | ví, že argument je cesta k souboru `.toml`, PID nebo jméno snapshotu, a doplňuje jen platné hodnoty | popis argumentů v manifestu programu |
| **vysvětlení chyb** | u chyby ukáže kód, význam a co dělat (z dokumentace chyb a hlášek) | `docs/chyby.md`, `hlasky.md` |
| **historie s kontextem** | historie příkazů i s pracovní složkou, výsledkem a dobou; hledání v ní i podle obsahu | žurnál, index |
| **bloky výstupu** | každý příkaz je samostatný blok (sbalit, kopírovat, znovu spustit, porovnat dva běhy) | terminál, ne shell |
| **asistent** (volitelný) | „najdi velké soubory starší než rok“ -> navržený příkaz, spustí se až po potvrzení a v náhledu | služba AI (níže) |

Návrhy navíc k odsouhlasení:

- **Zámek nebezpečných operací:** příkaz, který by smazal nebo přepsal
  víc než N souborů mimo dočasné složky, se vždy spustí přes „ukaž, co se
  stane“ - i v ostrém režimu stačí jedno potvrzení, ale nikdy naslepo.
- **Relace přežijí restart terminálu:** běžící příkaz patří službě relací,
  ne oknu; po pádu nebo zavření okna se jde vrátit (jako tmux, ale
  vestavěně).
- **Kompatibilita:** klasický textový režim pro skripty a POSIX nástroje
  (C/C++ z milníku jazyků) - strukturovaný výstup je nadstavba, ne náhrada.

## AI v systému (návrh)

> Odsouhlasené pořadí a API akcí: [Rozhodnutí k milníkům 6a-6d](#rozhodnutí-k-milníkům-6a-6d-odsouhlaseno).

Jádro o AI nic neví. Systém ale dává **primitiva, díky kterým je AI agent
bezpečný**, a nad nimi volitelné služby.

| Primitivum | Proč pro agenta |
|---|---|
| capability jen ke konkrétní složce | agent nevidí nic, co mu uživatel nedal |
| práce v klonu, výsledek jako diff | změny agenta se převezmou až po kontrole, nebo zahodí jedním krokem |
| podmíněné uložení (očekávaná revize) | agent tiše nepřepíše změnu uživatele nebo jiného nástroje |
| audit a žurnál s identitou aplikace | je vidět, co agent četl a změnil; jeho změny jdou vybraně vrátit |
| kvóty a priority (QoS, I/O) | agent nezahltí disk ani CPU, UI zůstane plynulé |
| síť jen přes capability | agent nepošle data ven bez povolení |

Služby nad tím (pořadí podle užitku):

1. **Služba modelů** (`aid`): lokální inference (CPU, později GPU/NPU),
   plánování výpočetních zdrojů mezi aplikacemi, jednotné API; cloudové
   modely jen jako volba uživatele a jen přes capability sítě.
2. **Sémantické vyhledávání:** index obsahu souborů (vektory) jako služba
   nad žurnálem změn, respektuje oprávnění (agent najde jen to, co smí
   číst).
3. **Asistent** v terminálu a průzkumníku: navrhuje příkazy a akce,
   provádí je přes „ukaž, co se stane“.
4. **Agentní relace:** spuštění agenta nad projektem = automaticky klon,
   capability jen k němu, audit zapnutý, na konci diff k převzetí.

⚠️ **Rizika (řečená předem):**

- **Podvržené pokyny** ve zpracovávaných datech (dokument nebo web, který
  agentovi „přikáže“ něco udělat): proto agent nikdy nedostane víc
  oprávnění, než si uživatel výslovně vybral, a nevratné akce potvrzuje
  člověk.
- **Soukromí:** výchozí jsou lokální modely; poslání dat do cloudu je
  viditelná volba.
- **Výkon:** modely jsou velké; běží v `background` QoS a s kvótou paměti.

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

aarch64 se portuje podle plánu milníků (milník 9): nejdřív QEMU `virt`,
pak Raspberry Pi. Jádro už má explicitní [rozhraní
architektury](#rozhraní-architektury), takže port znamená napsat jeho
implementaci v `kernel/src/arch/aarch64/` (kostra už existuje).

## Rozhraní architektury

Cíl: **port na jinou architekturu = napsat novou implementaci jednoho
rozhraní**, ne přepisovat půlku jádra. Všechno, co závisí na procesoru nebo
na jeho okolí (přerušení, časovače, stránkování, start CPU, IOMMU), je
v `kernel/src/arch/<arch>/` a zbytek jádra s tím mluví jen přes pevnou
smlouvu. Stav: x86_64 je za rozhraním celé, **aarch64 je zatím jen kostra**
(přeloží se, za běhu skončí na prvním `todo!`).

> **Změna (2026-11-06):** dřív jádro volalo funkce `arch::*`, které
> existovaly jen pro x86_64, a několik míst mimo `arch/` znalo x86 přímo
> (ACPI tabulky MADT/HPET/DMAR/IVRS v obecném `Platform`, start CPU přes
> stránky pod 1 MiB, vektory přerušení 48..239, srovnání TSC, obsluha
> přerušení volala přímo plánovač). **Proč:** bez explicitní smlouvy by
> port na aarch64 znamenal hledat skryté závislosti po celém jádře. Teď je
> smlouva v traitech, překladač hlídá úplnost a `cargo xtask test`
> ověřuje, že se jádro přeloží i pro aarch64.

<figure class="diagram">
<svg viewBox="0 0 760 360" xmlns="http://www.w3.org/2000/svg" role="img" aria-label="Rozhrani architektury">
<defs>
<marker id="ar1" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow" d="M0,0 L10,5 L0,10 z"/></marker>
<marker id="ar2" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto"><path class="arrow-acc" d="M0,0 L10,5 L0,10 z"/></marker>
</defs>
<rect class="box" x="10" y="10" width="560" height="56" rx="6"/>
<text class="t b" x="290" y="33" text-anchor="middle">obecné jádro</text>
<text class="m" x="290" y="53" text-anchor="middle">sched, mm, object, syscall, smp, acpi (RSDP, XSDT, MCFG), log</text>
<path class="ln" d="M290 66 L290 96" marker-end="url(#ar1)"/>
<rect class="acc" x="10" y="98" width="560" height="50" rx="6"/>
<text class="t b" x="290" y="119" text-anchor="middle">fasáda crate::arch</text>
<text class="m mono" x="290" y="138" text-anchor="middle">arch::now_ns(), arch::context::switch, arch::paging::PageTable, ...</text>
<path class="ln" d="M290 148 L290 176" marker-end="url(#ar1)"/>
<text class="m" x="300" y="167">statický dispatch na arch::Current</text>
<rect class="box" x="10" y="178" width="560" height="62" rx="6"/>
<text class="t b" x="290" y="198" text-anchor="middle">smlouva arch::api (traity, impl Architecture)</text>
<text class="m mono" x="290" y="216" text-anchor="middle">Cpu, Interrupts, Threads, Time, Ipi, Memory,</text>
<text class="m mono" x="290" y="232" text-anchor="middle">DeviceIrq, Iommu, Smp, PlatformInfo, Diagnostics</text>
<rect class="ok" x="10" y="276" width="270" height="62" rx="6"/>
<text class="t b" x="145" y="298" text-anchor="middle">arch/x86_64 (hotové)</text>
<text class="m" x="145" y="318" text-anchor="middle">GDT/IDT, APIC, TSC+HPET, VT-d, AMD-Vi, SIPI</text>
<rect class="warn" x="300" y="276" width="270" height="62" rx="6"/>
<text class="t b" x="435" y="298" text-anchor="middle">arch/aarch64 (kostra)</text>
<text class="m" x="435" y="318" text-anchor="middle">GICv3, generic timer, SMMUv3, PSCI: todo!</text>
<path class="ln dash" d="M145 276 L145 242" marker-end="url(#ar1)"/>
<path class="ln dash" d="M435 276 L435 242" marker-end="url(#ar1)"/>
<text class="m" x="152" y="262">implementuje</text>
<text class="m" x="442" y="262">implementuje</text>
<rect class="acc" x="600" y="98" width="150" height="142" rx="6"/>
<text class="t b" x="675" y="122" text-anchor="middle">crate::trap</text>
<text class="m" x="675" y="145" text-anchor="middle">přerušení, IPI,</text>
<text class="m" x="675" y="162" text-anchor="middle">syscall, výjimka</text>
<text class="m" x="675" y="179" text-anchor="middle">v userspace</text>
<text class="m" x="675" y="205" text-anchor="middle">jediná cesta</text>
<text class="m" x="675" y="222" text-anchor="middle">arch -&gt; jádro</text>
<path class="ln-acc" d="M145 338 L145 350 L675 350 L675 242" marker-end="url(#ar2)"/>
<path class="ln-acc" d="M435 338 L435 350"/>
<path class="ln-acc" d="M675 98 L675 38 L572 38" marker-end="url(#ar2)"/>
</svg>
<figcaption>Jádro volá architekturu jen přes fasádu (dolů), architektura jádro jen přes <code>crate::trap</code> (nahoru).</figcaption>
</figure>

### Tři části

1. **Smlouva** `kernel/src/arch/api/` - traity, které každá architektura
   implementuje na prázdném typu (`X86_64`, `Aarch64`). Souhrnný trait
   `Architecture` vyžaduje všechny skupiny, takže chybějící funkce nebo jiná
   signatura je chyba překladu, ne chyba za běhu.
2. **Fasáda** `kernel/src/arch/mod.rs` - volné funkce a typové aliasy
   (`arch::now_ns()`, `arch::context::Context`, `arch::paging::PageTable`,
   `arch::iommu::Domain`), které volání staticky přepošlou na
   `arch::Current` (vybírá `cfg(target_arch)`). Žádná dynamika, žádná režie.
3. **Zpětný směr** `kernel/src/trap.rs` - jediné místo, kam kód
   architektury volá do jádra: časovač, IPI, přerušení zařízení, syscall,
   výjimka v userspace, návrat do userspace.

| trait | co dodává architektura | x86_64 | aarch64 (plán) |
|---|---|---|---|
| `Cpu` | start CPU, HW ID, per-CPU ukazatel, zastavení, breakpoint | GDT/TSS/IDT, ID lokálního APIC, GS báze | VBAR_EL1, MPIDR_EL1, TPIDR_EL1 |
| `Interrupts` | zapnout/vypnout přerušení, uspat do přerušení | `sti`/`cli`/`hlt` | DAIF, `wfi` |
| `Threads` | kontext vlákna, přepnutí, zásobník jádra, vstup do userspace | callee-saved + FXSAVE, `iretq` | x19-x30 + v0-v31, `eret` |
| `Time` | hodiny v ns, čítač cyklů, jednorázový časovač, navázání času firmwaru (`ns_from_counter_to_start`), hardwarové hodiny ([reálný čas](cas.md)) | TSC (kalibrace HPET), APIC timer, CMOS RTC | CNTVCT_EL0, CNTP; hodiny zatím jen z UEFI |
| `Ipi` | přeplánuj, zastav, vyprázdni TLB | APIC IPI | GICv3 SGI |
| `Memory` | tabulky stránek (`PageTableOps`), TLB, cache pro DMA | 4 úrovně, CR3, IPI shootdown | TTBR0/TTBR1, `TLBI ... IS` bez IPI |
| `DeviceIrq` | rozsah čísel přerušení zařízení, zpráva MSI | vektory 48..239, 0xFEE.. | LPI od 8192 přes ITS |
| `Iommu` | domény překladu DMA (`IommuDomain`) | VT-d, AMD-Vi | SMMUv3 |
| `Smp` | start dalších CPU (`CpuStarter`) | INIT-SIPI, trampolína pod 1 MiB | PSCI CPU_ON |
| `PlatformInfo` | ACPI tabulky, které obecná část nezná | MADT, HPET, DMAR, IVRS | MADT (GICC/GICD/ITS), GTDT, IORT, SPCR |
| `Diagnostics` | ladicí sériová linka, výpis zásobníku, konec QEMU, vlastní testy | COM1, `rbp`, isa-debug-exit | PL011, `x29`, semihosting |

### Pravidla

- **Zbytek jádra** používá jen fasádu `crate::arch` (a typy z
  `arch::api`). Nikdy `arch::x86_64::...` - ten modul je soukromý, takže to
  ani nejde.
- **Kód architektury** volá do jádra jen `crate::trap` a obecné služby:
  `log`, `sync`, `error`, `fatal`, `mm` (alokátor fyzické paměti, přímá
  mapa, mapování MMIO) a čtení ACPI (`crate::acpi::read`). Plánovač,
  objekty ani syscally přímo ne.
- Vše v `trap` se volá se zakázanými přerušeními a s už potvrzeným
  přerušením v řadiči; výjimkou je `trap::syscall`, který běží
  s povolenými přerušeními na zásobníku jádra.
- Společné typy (`MapFlags`, `PageSize`, `MapError`, `PAGE_SIZE`) jsou
  v `arch::api::paging` jednou pro všechny architektury.
- Platí pro všechny architektury (část ABI nebo rozložení jádra):
  rámec jádra 4 KiB, stejné rozložení adresního prostoru (přímá mapa od
  `0xFFFF_8000_0000_0000`, jádro v posledních 2 GiB, společný
  `kernel/linker.ld`), per-CPU struktura s ukazatelem na sebe na offsetu 0
  a daty architektury na offsetu `CPU_LOCAL_OFFSET`, čísla chyb CPU pro
  ukončený proces = čísla vektorů x86 (aarch64 na ně mapuje své výjimky:
  abort 14, nedefinovaná instrukce 6, ...), vstupní body s `extern "C"`.
- Pozor na typové aliasy: `arch::paging::PageTable` je na x86_64 přímo
  typ z `arch/x86_64`, takže by šly volat i jeho vlastní metody mimo
  smlouvu. Na x86 to projde, ale `cargo xtask check --arch aarch64` (součást
  `cargo xtask test`) to odhalí.

### Příklady

Obecný kód (plánovač) - jen fasáda, žádná znalost x86:

```rust
use crate::arch::{self, interrupts};
use crate::arch::context::{self, Context, ThreadContext};

// nové vlákno: kontext připraví architektura
let ctx = unsafe { Context::new(stack.top(), thread_start) };

// přepnutí: adresní prostor, zásobník pro vstup z userspace, registry
unsafe {
    arch::switch_user_space(root); // Some(tabulky procesu) / None
    arch::set_kernel_stack(&cpu.arch, top);
    context::switch(from, to);
}
arch::set_timer(Some(1_000_000)); // přerušení za 1 ms
```

Implementace jedné skupiny v architektuře (x86_64):

```rust
impl api::Ipi for X86_64 {
    fn send_reschedule(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_RESCHEDULE);
    }
    fn send_halt(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_HALT);
    }
    fn send_tlb_flush(cpu_hw_id: u32) {
        apic::send_ipi(cpu_hw_id, apic::VECTOR_TLB_FLUSH);
    }
}
```

Cesta přerušení časovače (zpětný směr):

```text
APIC timer -> vektor 32 -> idt::trap_dispatch (x86)
           -> apic::eoi()                    (potvrzení v řadiči)
           -> trap::timer_interrupt()        (smlouva)
           -> sched::on_timer_interrupt()    (obecné jádro)
           -> trap::after_interrupt()        (případné přeplánování)
```

Kontrola, že se jádro přeloží pro jinou architekturu (je i součástí
`cargo xtask test`):

```bash
cargo xtask check --arch aarch64
```

### Kostra aarch64 a co do ní patří

`kernel/src/arch/aarch64/mod.rs` implementuje celou smlouvu; každá funkce,
která má něco dělat, končí `todo!` s popisem, co v ní bude. Cíl je nejdřív
**QEMU `virt`** (GICv3, PL011, PSCI, SMMUv3, UEFI z edk2 s ACPI), potom
Raspberry Pi. Plánované moduly:

| modul | obsah |
|---|---|
| `exceptions` | tabulka vektorů (VBAR_EL1), uložení registrů, ESR -> číslo chyby ABI |
| `gic` | GICv3: distributor, redistributor každého CPU, ITS pro MSI (LPI) |
| `timer` | generic timer: CNTVCT_EL0 jako hodiny, CNTP_TVAL_EL0 jednorázově |
| `mmu` | 4 úrovně s granulí 4 KiB, TTBR0 (proces) a TTBR1 (jádro), MAIR, TCR |
| `context` | x19-x30, sp, FPU/SIMD v0-v31, FPCR/FPSR; přepínání |
| `user` | EL0 <-> EL1: SVC, ERET, SP_EL0 |
| `psci` | start CPU přes CPU_ON (HVC/SMC podle ACPI FADT) |
| `smmu` | SMMUv3: stream table, domény |
| `acpi` | MADT (GICC, GICD, GIC ITS), GTDT, IORT (RID -> StreamID/DeviceID), SPCR |
| `pl011` | ladicí sériová linka |

Mimo jádro bude port potřebovat ještě: loader pro `aarch64-unknown-uefi`
(dnes x86_64: stránkování a skok do jádra), xtask (QEMU `virt`, firmware
edk2 pro aarch64, ukončení testu přes semihosting), `libs/elf` (dnes
přijímá jen `EM_X86_64`), `libs/libos` a `libs/abi` (instrukce syscallu,
`svc` místo `syscall`) a `user/user.ld`.

### Závazná omezení a jak jsou vyřešená

Port na ARM přináší věci, které na x86 neexistují. Rozhraní a jádro je řeší
předem, aby se při portu neměnila obecná část:

- **Jádro nesmí používat FPU/SIMD.** Stav FPU programu se ukládá jen při
  přepnutí vlákna, ne při každém vstupu do jádra. Jádro se proto překládá
  bez FPU: x86_64 bez SSE, aarch64 pro cíl `aarch64-unknown-none-softfloat`
  (s obyčejným `aarch64-unknown-none` by překladač použil registry NEON
  např. pro kopírování a přepsal je programu).
- **Nekoherentní DMA (Raspberry Pi 4).** Paměť pro DMA
  (`VMO_CREATE_CONTIGUOUS`) se do procesu mapuje s `MapFlags::DMA`:
  architektura podle platformy zvolí normální cache (koherentní platforma)
  nebo mapování bez cache (Normal Non-cacheable). Kde jádro samo sahá na
  tuto paměť přes přímou mapu (vynulování, `VMO_READ`/`VMO_WRITE`, vrácení
  stránek), volá `arch::dma_sync_for_device` / `dma_sync_for_cpu`.
  Ovladače nic řešit nemusí; na x86 jsou funkce prázdné.
- **Pořadí zápisů vůči zařízení.** Ovladače používají bariéry
  `libos::barrier` (`dma_wmb`, `dma_rmb`, `mmio_wmb`) místo `fence`; na ARM
  jsou to `dmb osh*` a `dsb st`, na x86 stejná instrukce jako dřív. Viz
  [libos](libos.md#bariéry-pro-ovladače).
- **MSI na GICv3 potřebuje identitu zařízení.** `INTERRUPT_CREATE` předá
  architektuře zařízení z omezeného Resource a `msi_message` může vrátit
  chybu: bez zařízení dostane program `INVALID_ARGS` (místo pádu jádra).
  Ovladače od `devmgr` mají prostředek omezený na zařízení vždy.
- **Tabulky jádra a procesu jsou na ARM v různých registrech** (TTBR1
  a TTBR0). Proto `PageTableOps::activate_kernel` (jen tabulky jádra)
  a `arch::switch_user_space(Option<root>)` pro proces (`None` = vlákno
  jádra, spodní polovina nic nemapuje).
- **Adresa instrukce v logu** se jmenuje podle architektury
  (`arch::PC_NAME`: `rip` / `pc`); symbolizace v `cargo xtask` přijímá obě.

Co vyřešit nejde nebo je to vědomé rozhodnutí:

- **Raspberry Pi 4 nemá IOMMU (SMMU).** DMA ovladačů tam nebude omezené -
  stejné riziko jako x86 bez IOMMU (chybný ovladač může přes DMA přepsat
  jádro); jádro to při startu zaloguje jako varování. Je to vlastnost
  hardwaru. RPi 5 sice IOMMU má (BCM2712), ale překládá jen displej
  a multimédia v jednom okně 4 GB a vše mimo okno projde bez kontroly -
  ochranu DMA ovladačů nedává, takže backend `Iommu` pro ni nedává smysl.
- **Velikost stránky procesu není konstanta ABI.** Jádro samo pracuje
  s rámcem 4 KiB, ale stránka procesu je vlastnost jeho adresního prostoru
  (`VM_PAGE_SIZE`, `libos::vm::page_size()`, mez 4 KiB až 64 KiB
  v `abi::vm`). Dnes je všude 4 KiB. Na aarch64 jde granuli nastavit pro
  každý adresní prostor zvlášť (TTBR0), takže nativní procesy mohou dostat
  16 KiB (o jednotky procent méně výpadků TLB) a procesy Windows vrstvy
  4 KiB (PE sekce s různými právy po 4 KiB). Programy jsou linkované se
  segmenty zarovnanými na 64 KiB (`user/user.ld`), takže fungují s každou
  velikostí. Apple M1/M2: procesor 4 KiB umí, jejich IOMMU (DART) ale jen
  16 KiB - backend by zarovnával paměť pro DMA na 16 KiB.


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
6. Systém: userspace ovladače (virtio - hotovo), FS (hotovo), síť (virtio-net,
   TCP/IP, DHCP, DNS, NTP a přístup přes capability hotové - viz
   [sit.md](sit.md); zbývá TLS 1.3, HTTP klient, IPv6, vlastní stack)
   - **6a Vstup:** služba vstupu, klávesnice a myš (virtio-input, PS/2,
     později USB HID), rozložení, bezpečná zkratka - viz [Vstup](#vstup-klávesnice-a-myš-plán)
   - **6b Shell a chytrý terminál:** konzole, strukturovaný výstup, undo
     příkazů, „ukaž, co se stane“, oprávnění při spuštění - viz [Shell](#shell-a-chytrý-terminál-návrh)
   - **6c GUI:** virtio-gpu, kompozitor, nativní knihovna UI, systémové
     dialogy - viz [Grafika](#grafika)
   - **6d AI-ready:** primitiva pro bezpečné agenty (většina z FS v3),
     služba modelů, sémantické vyhledávání, asistent - viz [AI v systému](#ai-v-systému-návrh)
7. Jazyky a překladač (viz [Jazyky a překladače](#jazyky-a-překladače-plán)):
   Rust (`std` pro náš OS), C (vlastní libc nad libos), C++ (libc++/libunwind
   nad libc), překlad clangem z hostitele, pak TinyCC v systému
8. Windows vrstva v2: user32/gdi32, GUI, DirectX přes Vulkan
9. Port na aarch64 a první ARM zařízení (rozhraní architektury a kostra
   aarch64 hotové, implementace zbývá)

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
- **Stack TCP/IP je smoltcp** (za modulem `stack` služby `netd`), vlastní
  stack ho nahradí později; služba sítě čeká na nejvýš 63 socketů
  (limit `OBJECT_WAIT_MANY`) a nemá kryptografický zdroj náhodnosti - viz
  [sit.md](sit.md#plán).
- **Mezi třídami QoS přísná priorita** (nižší třídy mohou hladovět).
- **Paměť po ukončených vláknech** (zásobníky jádra) se používá znovu, ale
  nevrací se.
