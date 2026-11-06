# Hlášky jádra, loaderu a libos

Seznam varování a chyb, které se mohou objevit v logu, s významem a tím, co
dělat. Formát záznamu logu a úrovně popisuje [jadro.md](jadro.md#logování).
U varování (W) a chyb (E) log uvádí i soubor a řádek zdroje.

## Loader (UEFI)

Loader píše přes konzoli firmware (obrazovka i sériová linka) ve formátu
`[ WARN]: loader\src\main.rs@NNN: ...`.

| Hláška | Význam a řešení |
|---|---|
| `initfs nenalezen (...), jadro nespusti zadny proces` | Na ESP chybí `\os\initfs.img`. Jádro nastartuje a proběhnou jeho testy, ale žádný proces. Sestavit obraz přes `cargo xtask image`. |
| `GOP neni k dispozici, framebuffer nebude` | Firmware nemá grafický výstup. Jádro píše jen na sériovou linku. |
| `GOP nelze otevrit, framebuffer nebude` | Grafický výstup existuje, ale nejde použít. Jako výše. |
| `nepodporovany format pixelu ..., framebuffer nebude` | Režim obrazovky není 32bitový RGB/BGR. Jako výše. |
| `ACPI 2.0 RSDP nenalezen` | Firmware nepředal ACPI 2.0. Jádro pak nenastartuje (`loader nepredal ACPI RSDP`). |
| panika `nelze nacist \os\kernel.elf` | Na ESP chybí jádro. Znovu sestavit obraz. |
| panika `kernel neni platny ELF` | Soubor jádra je poškozený nebo jiného formátu. |
| panika `kernel musi byt linkovany od KERNEL_BASE` | Jádro bylo slinkováno bez `kernel/linker.ld`. |

## Jádro: start a platforma

| Hláška | Úroveň | Význam a řešení |
|---|---|---|
| `kernel: neplatny BootInfo od loaderu` | - | Loader a jádro mají jinou verzi rozhraní `boot-info` (jádro se zastaví bez dalšího výpisu). Sestavit oba stejnou verzí (`cargo xtask build`). |
| `loader nepredal ACPI RSDP` | E | Bez ACPI jádro nezná CPU ani časovače a zpanikuje (`cteni ACPI selhalo`). Firmware musí podporovat ACPI 2.0. |
| `neplatny ACPI RSDP na ...` | E | Struktura RSDP je poškozená. Chyba firmwaru. |
| `korenova ACPI tabulka (XSDT/RSDT): CORRUPTED` | W | Kořenová tabulka má špatný checksum nebo délku; ACPI nejde číst. |
| `ACPI tabulka na ... vynechana: CORRUPTED (-10)` | W | Jedna tabulka je poškozená. Pokud nejde o MADT nebo HPET, jádro pokračuje bez ní (na reálném HW se to stává). |
| `MADT: poskozena polozka na offsetu N, zbytek vynechan` | W | Položka MADT má neplatnou délku. CPU popsaná za ní se nespustí. |
| `MADT: prilis kratka polozka typu T (N B), vynechana` | W | Položka je kratší než její formát; přeskočí se. |
| `tabulka HPET je prilis kratka, vynechana` | W | Bez HPET jádro nezkalibruje hodiny a zpanikuje (`platforma nema HPET`). |
| `chybi platna tabulka MADT ...` | E | Bez seznamu CPU jádro nenastartuje. Chyba firmwaru. |
| panika `cteni ACPI selhalo: ...` | panika | Viz předchozí řádky. |
| panika `platforma nema HPET` | panika | Jádro zatím vyžaduje HPET pro kalibraci TSC a APIC timeru (QEMU q35 a moderní PC ho mají). |
| `casovace: ... (CPU nehlasi invariantni TSC)` | I | CPU nehlásí stálou frekvenci TSC. V QEMU s emulací (TCG) je to normální; na reálném HW by se čas mohl při úsporných režimech zkreslit. |
| `chybi volna pamet pod 1 MiB, bezim jen na jednom CPU` | W | Pro start dalších CPU jsou potřeba dvě volné stránky pod 1 MiB. Systém běží dál na jednom CPU. |
| `nedostatek pameti pro zasobnik CPU N` | E | Další CPU se nespustí. |
| `CPU s APIC ID N se neohlasilo, dalsi CPU nestartuji` | E | CPU nereagovalo na INIT-SIPI-SIPI do ~400 ms. Jeho zásobník a startovací kód zůstávají vyhrazené (kdyby naběhlo později); další CPU se už nespouštějí. |
| `citac cyklu CPU (APIC ID N) je posunuty o X tiku proti BSP ..., opravuji` | W | TSC tohoto CPU není synchronní s prvním CPU. Jádro ho posune přes `IA32_TSC_ADJUST`. |
| `CPU neumi opravit citac cyklu (posun X tiku): cas na tomto CPU bude posunuty` | E | Posun je velký a CPU nemá `IA32_TSC_ADJUST`. Čas na tomto CPU se liší; termíny čekání mohou být nepřesné. |
| `mereni posunu citace cyklu neprobehlo, pokracuji bez nej` | W | CPU naběhlo pozdě (po vypršení času); TSC se neověřil. |
| `necekane preruseni, vektor N` | W | Přišlo přerušení, které nikdo neobsluhuje (zatím se nepoužívají přerušení zařízení). Jednotlivý výskyt je neškodný. |
| `prilis mnoho prepsani urovne logu, X ignorovan` | W | Maximálně 16 modulů může mít vlastní úroveň logu. |

## Jádro: procesy

| Hláška | Úroveň | Význam a řešení |
|---|---|---|
| `loader nepredal initfs: zadny proces se nespusti` | W | Viz loader. |
| `initfs je poskozeny: ...` | E | Obraz initfs nemá správný formát (`libs/initfs`). Znovu sestavit. |
| `initfs neobsahuje program init` | E | V initfs chybí soubor `init`. |
| `init neni platny program: ...` | E | `init` není statický ELF64 pro x86_64. |
| `init: poskozeny segment: ...` | E | Segment ELF leží mimo soubor. |
| `init se nepodarilo spustit: ...` | E | Souhrnná hláška s kódem chyby ([chyby.md](chyby.md)). |
| `init spusten (proces N), vstup ...` | I | První proces běží. |
| `proces N (jméno), vlakno ...: vyjimka CPU V, kod ..., rip ..., adresa ...` | E | Program spadl (např. V = 14 page fault, adresa = kam přistupoval). Proces se ukončí s kódem -1 000 100 - V. Adresu instrukce (`rip`) lze najít v ELF programu (`llvm-objdump -d`). |
| `proces N (jméno) skoncil, kod C (popis)` | I | Konec procesu a význam kódu ([chyby.md](chyby.md#návratové-kódy-procesů)). |
| `init skoncil s kodem C (...)` | E | Init skončil neúspěšně; v testech to znamená selhání testů userspace (podrobnosti výše v logu: `[SELHAL] ...`). |
| `init nebezi` | E | Init se nespustil (viz předchozí chyby). |
| `syscall X (N) selhal: ...` | D | Neúspěšný syscall (úroveň Debug). Pro dohledání chyb programů. |

## Fatální chyby

| Hláška | Význam |
|---|---|
| `KERNEL PANIC na cpuN: panicked at soubor:řádek: zpráva` | Neopravitelná chyba v jádře. Následuje výpis zásobníku; `cargo xtask run`/`test` ho po skončení QEMU přeloží na jména funkcí. Ostatní CPU se zastaví. |
| `KERNEL: neosetrena vyjimka NÁZEV na cpuN, vektor V, chybovy kod ...` | Výjimka CPU v jádře (chyba jádra). U page faultu následuje adresa a typ přístupu, pak registry a zásobník. |
| `Double Fault (#DF)` s `rsp` těsně pod zásobníkem | Typicky přetečení zásobníku jádra (ochranná stránka). |

Interní pojistky (`expect`/`assert` v jádře, např. `nedostatek pameti pro
PML4`, `neni misto pro bitmapy alokatoru`) znamenají, že se nepodařil krok
startu, bez kterého jádro nemůže běžet (typicky příliš málo RAM), nebo
porušený vnitřní předpoklad (chyba jádra).

## libos (programy)

| Hláška | Význam |
|---|---|
| `libos: chybi nebo je prilis velka startovni zprava` | Proces nedostal startovní zprávu (`abi::bootstrap`). Proces končí s kódem -1 000 001. |
| `libos: neplatna startovni zprava` | Zpráva nemá správné magické číslo nebo verzi. |
| `libos: startovni zprava neobsahuje handle procesu` | Chybí `SELF_PROCESS`. |
| `libos: nedostatek pameti pro heap` | Program nemůže začít: nejde vytvořit nebo namapovat počáteční heap (4 MiB). Heap pak roste podle potřeby; když už růst nejde (dojde paměť nebo rezervovaný rozsah 64 GiB), alokace selže a program zpanikuje (`PANIC: ... alloc.rs ...`). |
| `PANIC: ...` | Panika v programu; proces končí s kódem -1 000 001. |

## Ovladače a správce zařízení

| Hláška | Význam a řešení |
|---|---|
| `devmgr: pci BB:ZZ.F vvvv:dddd trida cc.ss.pp` | Nalezené zařízení PCI (informace). |
| `devmgr: ovladac pro ... nelze spustit: ...` | Ovladač chybí v initfs nebo se nespustil (kód chyby). |
| `virtio-blk BB:ZZ.F: N MiB, flush ano/ne, discard ano/ne` | Disk je připravený. `discard ne` = zařízení neumí TRIM (v QEMU chybí `discard=unmap` u disku); FS pak TRIM přeskakuje. |
| `virtio-blk ...: inicializace selhala: ...` | Zařízení neodpovídá specifikaci virtio 1.x nebo chybí prostředky (MSI-X, DMA paměť). |
| `init: blokove zarizeni neodpovida: ...` | Ovladač neodpověděl na `INFO`; disk se nepoužije. |

## Služba fsd a souborový systém

| Hláška | Význam a řešení |
|---|---|
| `init: souborovy system 'X' pripojen (generace N)` | Init našel disk se souborovým systémem a služba odpovídá. |
| `init: zadny disk se souborovym systemem` | Žádný disk nemá v blocích 1-8 podpis `OSFS  v2`. Testy FS pak selžou (`sluzba fsd nebezi`). `cargo xtask run/test` datový disk vytváří vždy - zkontrolovat, že ho QEMU dostal. |
| `init: souborovy system nelze spustit: ...` | fsd chybí v initfs nebo neodpověděl na `INFO` (viz hlášky fsd před tím). |
| `fsd: pripojeno 'X': N MiB, volno N MiB, generace N, soucty Xxh3/Blake3` | Svazek je připojený. |
| `fsd: VAROVANI: nejnovejsi stav byl poskozeny, pouzit stav o N generaci starsi` | Nejnovější commit nešel načíst (disk ztratil zápisy, přestože potvrdil flush, nebo je poškozený). Změny z posledních N commitů jsou ztracené; zkontrolovat disk a spustit kontrolu (`CHECK` se scrubem). |
| `fsd: z intent logu prehrano N operaci` | Předchozí běh skončil bez commitu (výpadek); změny potvrzené `fsync` se obnovily. Normální po pádu. |
| `fsd: souborovy system nejde pripojit: ...` | `NOT_FOUND` = disk nemá superblok; `CORRUPTED` = žádný z posledních stavů nejde načíst; `NOT_SUPPORTED` = jiná verze formátu. Služba končí s kódem 3. |
| `fsd: disk neodpovida: ...` | Ovladač disku neodpověděl (kód 2). |
| `fsd: commit selhal: ...` | Chyba zápisu na disk. Instance je pak jen pro čtení (změny vrací `IO_ERROR`); na disku platí poslední dokončený commit. Restartovat službu. |
| `fsd: zaverecny commit selhal: ...` | Jako výše při ukončení (kód 4); změny od posledního commitu nebo `fsync` jsou ztracené. |
| `fsd: defragmentace: ...` | Krok defragmentace selhal (typicky `CORRUPTED` při čtení poškozeného bloku). Defragmentace se do dalších změn pozastaví; data se nemění. |
| `fsd: chyba kanalu: ...` | Neočekávaná chyba kanálu ke klientovi; služba udělá commit a skončí. |
| `fsd: chybi handly ve startovni zprave` | Chybný start služby (kód 1). |
