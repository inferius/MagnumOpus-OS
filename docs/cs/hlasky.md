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
| `cas firmwaru: RRRR-MM-DD hh:mm:ss (pasmo neuvedeno, beru jako UTC)` | Čas z UEFI `GetTime`. Pásmo firmware téměř nikdy neuvádí; když má počítač v hodinách místní čas (Windows), bude reálný čas posunutý o pásmo. Viz [Reálný čas](cas.md). |
| `UEFI GetTime selhal: ...` / `UEFI vratil neplatny cas` / `UEFI cas mimo kalendar` | Firmware čas nedal. Jádro zkusí CMOS RTC. |
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
| `realny cas: ... UTC (zdroj UEFI\|CMOS RTC...)` | I | Reálný čas při startu a jeho zdroj. „pasmo neuvedeno - bran jako UTC“ = viz hláška loaderu o pásmu. |
| `realny cas neni znamy (firmware ani RTC ho nedaly)` | W | `CLOCK_REALTIME` vrací `NOT_SUPPORTED`; `fsd` použije čas od startu (soubory budou mít datum 1970). Na PC to znamená vadné hodiny nebo vybitou baterii CMOS. |
| `chybi volna pamet pod 1 MiB, bezim jen na jednom CPU` | W | Pro start dalších CPU jsou potřeba dvě volné stránky pod 1 MiB. Systém běží dál na jednom CPU. |
| `nedostatek pameti pro zasobnik CPU N` | E | Další CPU se nespustí. |
| `CPU s APIC ID N se neohlasilo, dalsi CPU nestartuji` | E | CPU nereagovalo na INIT-SIPI-SIPI do ~400 ms. Jeho zásobník a startovací kód zůstávají vyhrazené (kdyby naběhlo později); další CPU se už nespouštějí. |
| `citac cyklu CPU (APIC ID N) je posunuty o X tiku proti BSP ..., opravuji` | W | TSC tohoto CPU není synchronní s prvním CPU. Jádro ho posune přes `IA32_TSC_ADJUST`. |
| `CPU neumi opravit citac cyklu (posun X tiku): cas na tomto CPU bude posunuty` | E | Posun je velký a CPU nemá `IA32_TSC_ADJUST`. Čas na tomto CPU se liší; termíny čekání mohou být nepřesné. |
| `mereni posunu citace cyklu neprobehlo, pokracuji bez nej` | W | CPU naběhlo pozdě (po vypršení času); TSC se neověřil. |
| `necekane preruseni, vektor N` | W | Přišlo přerušení, které nikdo neobsluhuje (zatím se nepoužívají přerušení zařízení). Jednotlivý výskyt je neškodný. |
| `prilis mnoho prepsani urovne logu, X ignorovan` | W | Maximálně 16 modulů může mít vlastní úroveň logu. |

## Jádro: IOMMU

| Hláška | Úroveň | Význam a řešení |
|---|---|---|
| `IOMMU VT-d na ADRESA: segment S, N urovne tabulek, ...` | I | Překlad DMA je zapnutý; zařízení smí jen do své domény. |
| `IOMMU neni: zarizeni smi pres DMA zapisovat kamkoli do pameti ...` | W | Stroj nemá VT-d ani AMD-Vi (tabulky DMAR a IVRS chybí), nebo běh s `--iommu none`. Systém funguje, ale chyba ovladače může přes DMA poškodit paměť. Na reálném PC zapnout VT-d / AMD-Vi (IOMMU) v nastavení firmwaru. |
| `IOMMU na ADRESA nejde zapnout: ...` | E | Jednotka nepodporuje 39/48bitové tabulky (`NOT_SUPPORTED`), chybí paměť, nebo nedokončila příkaz (`TIMED_OUT`). Zařízení pod ní nejsou chráněná. |
| `IOMMU: ... nedobehlo` | E | Jednotka nedokončila příkaz nebo zneplatnění do 1 s (chyba hardwaru nebo emulace). |
| `IOMMU AMD-Vi na ADRESA: segment S, 4 urovne tabulek` | I | Překlad DMA přes AMD-Vi je zapnutý. |
| `IOMMU AMD-Vi: prikazy nedobehly` | E | Jednotka nedokončila frontu příkazů do 1 s (chyba hardwaru nebo emulace). |
| `IOMMU AMD-Vi: preruseni MSI nejde nastavit, ...` | W | Konfigurace PCI jednotky nemá MSI nebo není v ECAM. Ochrana funguje, jen se zablokované přístupy nehlásí. |
| `IOMMU: zablokovan pristup zarizeni BB:ZZ.F na adrese A (udalost AMD-Vi K)` | W | Jako u VT-d níže; K = kód události AMD-Vi (0x2 = chyba stránky). |
| `IOMMU: dalsich N udalosti AMD-Vi vynechano` | W | Jeden zablokovaný přenos vyvolal víc událostí; podrobně se vypisují jen první 4. |
| `IOMMU: zablokovan zapis/cteni zarizeni BB:ZZ.F na adrese A (duvod R)` | W | Zařízení sáhlo mimo svou doménu: chyba ovladače (špatná adresa v deskriptoru) nebo vadné zařízení. Paměť zůstala nedotčená. Kód důvodu podle specifikace VT-d (např. 0x5 = stránka v doméně chybí, 0x6 = zápis do stránky jen pro čtení). |

## Jádro: procesy

| Hláška | Úroveň | Význam a řešení |
|---|---|---|
| `loader nepredal initfs: zadny proces se nespusti` | W | Viz loader. |
| `initfs je poskozeny: ...` | E | Obraz initfs nemá správný formát (`libs/initfs`). Znovu sestavit. |
| `initfs neobsahuje program init` | E | V initfs chybí soubor `init`. |
| `init neni platny program: ...` | E | `init` není statický ELF64 pro x86_64. |
| `framebuffer se do userspace nepreda: ...` | W | Framebuffer překrývá RAM pro alokátor, nezačíná na hranici stránky nebo je menší než jeho geometrie. Konzole jádra ho dál používá, GUI bez virtio-gpu nenaběhne. |
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
| `PANIC: ...` při nedostatku paměti | Panika se formátuje do bufferu na zásobníku (max. 512 B), takže ji jde vypsat i bez volné paměti. Panika během paniky proces jen ukončí. |
| `libos: nedostatek pameti pro heap` | Program nemůže začít: nejde vytvořit nebo namapovat počáteční heap (4 MiB). Heap pak roste podle potřeby; když už růst nejde (dojde paměť nebo rezervovaný rozsah 64 GiB), alokace selže a program zpanikuje (`PANIC: ... alloc.rs ...`). |
| `PANIC: ...` | Panika v programu; proces končí s kódem -1 000 001. |

## Ovladače a správce zařízení

| Hláška | Význam a řešení |
|---|---|
| `devmgr: pci BB:ZZ.F vvvv:dddd trida cc.ss.pp` | Nalezené zařízení PCI (informace). |
| `devmgr: ovladac X pro ... nelze spustit: ...` | Ovladač chybí v initfs nebo se nespustil (kód chyby). |
| `virtio-blk BB:ZZ.F: N MiB, flush ano/ne, discard ano/ne` | Disk je připravený. `discard ne` = zařízení neumí TRIM (v QEMU chybí `discard=unmap` u disku); FS pak TRIM přeskakuje. |
| `virtio-blk ...: inicializace selhala: ...` | Zařízení neodpovídá specifikaci virtio 1.x, nedokončilo reset do sekundy (`TIMED_OUT`) nebo chybí prostředky (MSI-X, DMA paměť, `NO_RESOURCES` = vyčerpaná kvóta omezeného Resource). |
| `X: disk D: nas oddil GPT, N MiB od sektoru S, sektor B B, zapis povolen` | Ovladač disku (`virtio-blk`, `nvme`, `ahci`) našel náš oddíl; klient vidí jen ten ([Disky](disky.md)). |
| `X: disk D: cely disk s nasim FS, ..., zapis povolen` | Disk bez tabulky oddílů s naším FS (datový disk v QEMU). |
| `X: disk D: cizi disk (...), ..., JEN PRO CTENI` | Disk nám nepatří (cizí GPT, MBR, prázdný, poškozená GPT, neplatný náš oddíl). Zápis a TRIM vrací `READ_ONLY`, FS se připojí jen pro čtení. Úmyslná ochrana; pro zápis na disk vytvořit náš oddíl. |
| `nvme BB:ZZ.F: nepodporovany radic (CAP ...)` | Řadič neumí sadu příkazů NVM, stránku 4 KiB nebo má doorbelly mimo BAR. |
| `nvme: jmenny prostor N ma metadata ... B na sektor - nepodporovano, vynechan` | Jmenný prostor je naformátovaný s metadaty (T10 PI). Přeformátovat na formát bez metadat (nástroj výrobce, `nvme format`). |
| `nvme: jmenny prostor N ma sektor ... B - nepodporovano, vynechan` | Sektor větší než jeden přenos řadiče (`MDTS`) nebo než 32 KiB. Běžné sektory 512 B a 4 KiB fungují. |
| `nvme: radic neodpovida spravne (...), dalsi pozadavky odmitnuty` | Příkaz nedoběhl do 10 s nebo řadič vrátil cizí číslo příkazu. Řadič je vypnutý (nepoužívá DMA); disk do restartu nefunguje. |
| `ahci BB:ZZ.F: MSI nelze zapnout (...), obsluha dotazovanim` | Řadič nemá MSI (nebo zpráva nejde zapsat); ovladač funguje dotazováním, jen s vyšší zátěží CPU. |
| `ahci: port P: disk ... neumi LBA48 - vynechan` | Velmi starý disk bez LBA48. Nepodporováno. |
| `ahci: port P: disk ... ma logicky sektor ... B - nepodporovano, vynechan` | Disk hlásí logický sektor, který není mocnina dvou 512 B..32 KiB (chybná odpověď IDENTIFY). Disky 512 B i 4Kn fungují. |
| `ahci: port P: zarizeni s podpisem ... (ne disk SATA) vynechano` | Na portu je mechanika ATAPI nebo port multiplier. |
| `ahci: port P: neodpovida (...), dalsi pozadavky odmitnuty` | Příkaz nedoběhl do 20 s; port je zastavený. |
| `devmgr: ovladac X pro ... nenahlasil disky: ...` | Ovladač `nvme`/`ahci` neposlal `done` do 150 s (zaseknutý řadič). Disky nahlášené do té doby fungují. |
| `ahci: port P: radic umi jen 32bitove adresy a DMA pamet je nad 4 GiB - vynechan` | Řadič bez 64bitových adres (`CAP.S64A = 0`) a paměť portu leží nad 4 GiB. Port se vynechá, jinak by řadič psal do cizí paměti. |
| `ahci: port P: ulozeni mezipameti pri ukonceni selhalo: ...` | Při ukončení ovladače disk nepřijal FLUSH CACHE; data z mezipaměti disku se mohla ztratit. |
| `virtio-net BB:ZZ.F: MAC xx:..., linka pripojena/odpojena` | Síťová karta je připravená. MAC je z konfigurace zařízení, jinak místně spravovaná (`02:...`). |
| `virtio-net ...: inicializace selhala: ...` | Jako u disku: zařízení neodpovídá virtio 1.x nebo chybí prostředky (MSI-X, DMA paměť). |
| `virtio-net ...: zarizeni neodpovida spravne (...), koncim` | Zařízení vrátilo nesmyslný index fronty nebo buffer, který nemělo (`CORRUPTED`). Ovladač ho resetuje (přestane používat DMA) a skončí; síť do restartu nefunguje. |
| `virtio-net ...: zahozeno N prijatych ramcu` | Při ukončení: rámce, které služba sítě nestíhala přijímat (plná fronta kanálu) nebo byly neplatné. |
| `virtio-rng BB:ZZ.F: 256 B entropie predano jadru` | Zdroj náhody z hostitele je připravený a jádro dostalo první data. Další dávky každou minutu. |
| `virtio-rng ...: zarizeni neodpovida (...), koncim` | Zařízení nedokončilo požadavek do 5 s nebo vrací prázdná data. Jádro dál používá ostatní zdroje. |
| `virtio-blk: zarizeni neodpovida spravne (...), dalsi pozadavky odmitnuty` | Zařízení nedokončilo požadavek do 10 s (`TimedOut`) nebo vrátilo neplatnou položku fronty (`Corrupted`). Ovladač zařízení resetuje a další požadavky vrací `IO_ERROR` (DMA buffer by zařízení mohlo ještě přepisovat). Klient (např. fsd) dostane chybu I/O. |
| `ps2: klavesnice`, `ps2: PS/2 mys ...` | Ovladač PS/2 našel klávesnici, myš (druh podle ID: kolečko, 5 tlačítek). |
| `devmgr: radic PS/2 nenalezen` | Port 0x64 čte `0xFF` (řadič chybí) nebo nenašel žádné zařízení. Na moderních deskách bez PS/2 normální. |
| `init: displej je framebuffer z firmwaru (fbdisplay)` | Správce zařízení nenašel virtio-gpu, GUI poběží přes `fbdisplay` s pevným rozlišením firmwaru. |
| `fbdisplay: framebuffer WxH (krok K, rgb/bgr)` | Ovladač framebufferu převzal obrazovku (`FB_CONTROL` `CLAIM`). |
| `fbdisplay: chybi handly ve startovni zprave` | Init ho spustil bez objektu paměti a kanálu (chyba initu). |
| `fbdisplay: framebuffer nelze prevzit: ...` | `FB_CONTROL` nebo mapování selhalo: `WRONG_TYPE` (objekt není framebuffer z jádra), `NOT_SUPPORTED` (neznámý formát nebo rozměry), `NO_RESOURCES` (není místo v adresním prostoru). |
| `init: grafika (virtio-gpu ani framebuffer) neni, GUI nespoustim` | Není virtio-gpu a jádro nepředalo framebuffer (bez GOP, nepodporovaný formát pixelu, viz hlášky jádra). |
| `ps2: radic neodpovida spravne: ...` | Řadič je, ale neprošel self-testem nebo neodpovídá včas (`TIMED_OUT`). |
| `ps2: klavesnice neodpovida na reset`, `ps2: mys neodpovida (...)` | Port je, ale zařízení na něm nereaguje (nepřipojené). |
| `xhci BB:ZZ.F: N portu (M USB 3), S slotu, kontext K B` | Řadič USB je připravený. |
| `xhci: firmware radic nepustil, pokracuji` | Firmware do sekundy neuvolnil řadič (USB Legacy Support); emulace PS/2 z USB může dál běžet. |
| `xhci ...: inicializace selhala: ...` | Řadič nedokončil zastavení, reset nebo start (`TIMED_OUT`), nebo chybí prostředky. |
| `xhci: port P: zarizeni vvvv:pppp (rychlost), slot S` / `zarizeni odpojeno` | Zařízení na portu nastavené / odpojené. |
| `xhci: prikaz typu T selhal (kod C)` | Příkaz řadiče (Enable Slot, Address Device, Configure Endpoint ...) vrátil chybu; zařízení se nepoužije. |
| `usb-hub: root hub, N portu` | Ovladač hubu běží na kořenových portech. |
| `usb-hub: port P: zarizeni nelze nastavit: ...` | Reset portu nebo nastavení zařízení selhalo (`TIMED_OUT`, `IO_ERROR`). |
| `usb-hub: externi hub [cesta], N portu` | Ovladač hubu běží na externím hubu. |
| `usb-hid: USB klavesnice/mys vvvv:pppp (port [...], rychlost)` | Ovladač HID se představil službě vstupu. |
| `usb-hid: rozhrani cc/ss/pp: deskriptoru reportu nerozumim` | Deskriptor reportu chybí, je poškozený nebo nemá nic známého a rozhraní nemá boot protokol. Rozhraní se nepoužije. |
| `usb-hid: rozhrani N nema interrupt endpoint IN` | Rozhraní HID bez vstupního endpointu; nepoužije se. |
| `devmgr: usb vvvv:pppp rozhrani cc/ss/pp: ovladac neni` | Pro třídu rozhraní není ovladač (např. disk USB). |
| `devmgr: radic USB nedokoncil pruchod porty` | `usb-ready` nepřišlo do 10 s; init dostane `done` i tak. |
| `inputd: nove zarizeni '...' ceka na overeni`, `inputd: zarizeni '...' overeno` / `odmitnuto` | Volba ověření nové klávesnice (BadUSB), viz [USB](usb.md). |
| `init: supervisor bezi` | Start hotový, init hlídá služby a zařízení ([Init](init.md)). |
| `init: sluzba X skoncila: ...` | Hlídaná služba skončila (důvod podle návratového kódu). |
| `init: sluzba X znovu spustena` | Supervisor službu obnovil. |
| `init: sluzba X spadla 6x za 60 s, vzdavam to` | Příliš mnoho pádů; služba zůstane zastavená. |
| `init: sluzbu X zatim neumim obnovit` | Pád služby bez restartu (`devmgr`, `fsd`, `inputd`); systém běží bez ní. |
| `init: spravce zarizeni skoncil, hotplug nefunguje` | `devmgr` zavřel kanál; zařízení připojená potom se nenajdou. |
| `init: blokove zarizeni neodpovida: ...` | Ovladač neodpověděl na `INFO`; disk se nepoužije. |
| `virtio-input BB:ZZ.F: 'jméno' (klavesnice/mys)` | Vstupní zařízení je připravené; druh podle podporovaných událostí. `nezname zarizeni` = nemá klávesy ani relativní osy (tablet, zatím nepodporovaný). |
| `virtio-input ...: inicializace selhala: ...` | Jako u virtio-blk: zařízení neodpovídá virtio 1.x nebo chybí prostředky (MSI-X, DMA). |
| `virtio-input: prvni udalosti od zarizeni (N)` | Zařízení poprvé poslalo události (informace; potvrzuje, že fungují přerušení a fronta). |
| `virtio-input: sluzba nestiha, zahozeno N zprav` | Fronta kanálu ke službě vstupu je plná; ovladač události zahazuje, aby neblokoval zařízení. Hlásí se při 1, 2, 4, 8 ... zprávách. |
| `init: N blokovych a M vstupnich zarizeni` | Co správce zařízení našel. |

## Služba vstupu (inputd)

| Hláška | Význam a řešení |
|---|---|
| `inputd: bezim (rozlozeni us)` | Služba běží s výchozím rozložením. |
| `inputd: zarizeni 'jméno' pripojeno` / `odpojeno` | Ovladač se představil (`IDEV`) nebo zavřel kanál. |
| `inputd: neplatna zprava od zdroje N` | Zdroj poslal zprávu, která neodpovídá protokolu zařízení (neznámá značka, špatná délka); zpráva se zahodí. |
| `inputd: bezpecna zkratka, ale nikdo ji neposloucha` | Ctrl+Alt+Delete, ale nikdo nemá kanál `secure`; zkratka se jen pohltí. |
| `inputd: bezpecna zkratka neprosla, kanal secure je plny` | Příjemce zkratky nečte; zkratka se pohltí a bezpečný režim nezačne (vstup zůstává vlastníkovi fokusu). |
| `inputd: bezpecna zkratka, ale kanal secure je zavreny` | Příjemce zkratky skončil; zkratka se pohltí. |
| `init: sluzba vstupu bezi (N zarizeni)` | Init spustil `inputd` a připojil N ovladačů. |
| `init: sluzbu vstupu nelze spustit: ...` | `inputd` chybí v initfs nebo neodpověděla na příkaz (`TIMED_OUT`). Testy vstupu pak selžou (`sluzba vstupu nebezi`). |

## Služba fsd a souborový systém

| Hláška | Význam a řešení |
|---|---|
| `init: souborovy system 'X' pripojen (generace N)` | Init našel disk se souborovým systémem a služba odpovídá. |
| `init: zadny disk se souborovym systemem` | Žádný disk nemá v blocích 1-8 podpis `OSFS\0\0v2`. Testy FS pak selžou (`sluzba fsd nebezi`). `cargo xtask run/test` datový disk vytváří vždy - zkontrolovat, že ho QEMU dostal. |
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

## Služba sítě netd

| Hláška | Význam a řešení |
|---|---|
| `netd: bezim, cekam na DHCP` | Služba dostala `INFO` od ovladače a spustila klienta DHCP. |
| `netd: DHCP: adresa A/P, brana G, DNS [...]` | Rozhraní je nastavené; programy čekající na `wait_configured` pokračují. |
| `netd: DHCP: adresa ztracena` | Pronájem vypršel nebo server adresu odebral. Spojení dostávají `UNREACHABLE`, dokud DHCP adresu neobnoví. |
| `netd: VAROVANI: generator jadra neni nasazeny, ...` | Jádro zatím nemá dost entropie (bez RDRAND a bez virtio-rng): ID dotazů DNS a čísla TCP jsou odhadnutelná. Viz [Náhodná čísla](syscally.md#náhodná-čísla). |
| `netd: nova relace s pravy ...` | Někdo s právem `RESTRICT` (init) vytvořil uzší relaci. |
| `netd: servery DNS nastaveny rucne: [...]` | `SET_DNS` (právo `CONFIGURE`); prázdný seznam = zpět servery z DHCP. |
| `netd: ovladac sitove karty neposlal INFO: ...` | Ovladač do 5 s neodpověděl (kód 2); `init` pak síť nespustí. |
| `netd: ovladac skoncil (zahozeno ramcu: ...)` | Ovladač karty skončil (vadné zařízení) a služba končí s ním (kód 3). |
| `netd: chybi handly ve startovni zprave` | Chybný start služby (kód 1). |
| `init: sluzbu site nelze spustit: ...` | Chybí síťová karta (`NOT_FOUND`) nebo služba neodpověděla. |
| `ntpd: bezim, servery [...]` | Synchronizace času běží; servery z `ntpd.toml` (v testech server hostitele). |
| `ntpd: prvni mereni (NTS\|neoverene): posun X ms, zpozdeni Y ms, timed ok:step\|ok:slew` | První úspěšné měření (ověřené NTS, nebo neověřené NTP v záloze); `step` = čas se nastavil skokem (posun nad 128 ms), `slew` = plynulé dorovnání. |
| `ntpd: NTS-KE <server>: ...` | Výměna klíčů NTS selhala (spojení, certifikát, odpověď); server se v kole přeskočí. Při neplatném certifikátu zkontrolovat hodiny (RTC). |
| `ntpd: neoverene servery: Never\|Fallback` | Init změnil pravidlo podle profilu sítě. |
| `ntpd: zadny server neodpovedel (..., NTS: ...)` | V tomto kole žádná odpověď (síť, firewall, server). Další pokus za 2, 4, 8, … s (nejvýš `min_poll`), stejná hláška se znovu nevypíše; hodiny běží dál podle poslední frekvence. |
| `ntpd: konfigurace: ...` | Konfigurace od init je neplatná; `ntpd` skončí. Opravit `ntpd.toml`. |
| `timed: prvni synchronizace: skok o N ms` | První korekce od `ntpd` byla velká: čas se nastavil skokem (jediný povolený skok). |
| `timed: prvni synchronizace: dorovnani o N ms` | První korekce byla malá: dorovná se plynule. |
| `timed: klient synchronizace skoncil` | `ntpd` skončil nebo zavřel kanál; hodiny běží dál bez korekcí. |
| `init: ntpd.toml: ...`, `init: timed odmitl ntpd: ...`, `init: ntpd hlasi: ...` | `ntpd` se nespustil (neplatná konfigurace, `timed` nepřijal kanál, chyba při startu); hodiny zůstanou podle RTC. |
| `init: synchronizaci casu nelze spustit: ...` | Souhrn předchozího; systém běží bez synchronizace. |
| `init: korenove certifikaty aktualizovany (N kB)` | Sada CA Mozilla v `/system/config/certs/mozilla.pem` se změnila (první start, nová verze systému). Viz [TLS a HTTP](tls-http.md#úložiště-kořenových-certifikátů). |
| `init: uloziste certifikatu nelze pripravit: ...` | Bez FS nebo bez práv: programy nemají žádnou CA a `https` selže (`UnknownIssuer`). |
| `fetch: uloziste certifikatu: ...`, `fetch: /certs/F: ...` | `fetch` nemůže číst úložiště nebo soubor (větší než 4 MiB, chyba FS); ostatní soubory načte. |
| `fetch: TLS: ...` | Konfigurace TLS nejde vytvořit; `fetch` skončí s kódem 2. |
| `revocd: bezim (OneCRL ano\|ne, N zaznamu, mezipamet na FS\|jen v pameti)` | Služba odvolání běží; N = záznamy OneCRL načtené z FS. |
| `revocd: OneCRL: N zaznamu` | Nový seznam odvolaných CA stažen a uložen. |
| `revocd: OneCRL: neplatny dokument` | Stažený JSON nejde přečíst; platí předchozí seznam, nový pokus za `retry_minutes`. |
| `revocd: <url>: ...` | Stažení CRL nebo OneCRL selhalo (stav HTTP, chyba spojení); adresa se `retry_minutes` nezkouší, stav odvolání je zatím neznámý. |
| `revocd: <url>: neni CRL` | Server vrátil něco jiného než CRL. |
| `tls: CRL <url> nejde pouzit (...)` | CRL z distribučního bodu certifikátu nejde přečíst ani ověřit (nepodporované rozšíření, špatný podpis, vydavatel mimo řetěz); přeskočí se, stav odvolání z ní je neznámý a spojení pokračuje. Jednou pro adresu. |
| `netd: DNS pres TCP: spojeni se nenavazalo do 3 s, novy pokus (n)` | Spojení TCP pro zkrácenou odpověď DNS uvázlo; resolver zkouší nové (nejvýš 3×). |
| `netproto: sluzba site neodpovedela na pozadavek OP do N s (...)` | `netd` požadavek nedokončil ani po limitu + rezervě (chyba nebo zaseknutí služby). Program dostal `TIMED_OUT`, kanál už nejde použít. OP = kód operace (`netproto::op`). |
| `dnsd: bezim, resolvery [...], DNSSEC ano\|ne, kotvy N, CA M` | Systémový resolver běží. |
| `dnsd: <resolver>: ...` | Resolver neodpověděl (spojení, TLS, HTTP); `dnsd` zkusí další. |
| `dnsd: <jméno>: DNSSEC neprosel (...)` | Odpověď je podvržená nebo poškozená (důvod v závorce); program dostane `CORRUPTED`. |
| `netd: dnsd skoncil (...), preklad bez nej` | Systémový překlad jen podle pravidla profilu (veřejná síť = žádný). |
| `init: sifrovane DNS nelze spustit (...)` | `dnsd` nenaběhl; překlad jen podle pravidla profilu. |
| `init: kontrolu odvolani certifikatu nelze spustit: ...` | `revocd` nenaběhl; programy kontrolují jen OCSP stapling. |
| `init: revocd bez FS (...)` | Bez FS: mezipaměť jen v paměti a bez kořenových CA pro OneCRL přes https. |
| `netd: bez sluzby firewallu (verejny profil, dotazy se odmitaji)` | `init` nepředal kanál k `fwd`. Platí veřejný profil (nic dovnitř), dotazy aplikací s `ask` se rovnou odmítnou. |
| `netd: sluzba firewallu skoncila, dotazy se odmitaji` | `fwd` skončil. Platí poslední konfigurace, rozpracované a další dotazy se odmítnou. |
| `netd: profil site P` | `fwd` poslal konfiguraci s jiným profilem sítě (`public`, `home`, ...). Viz [Profily sítí](firewall.md#profily-sítí). |
| `netd: neplatna konfigurace firewallu: ...` | Zpráva `CONFIG` od `fwd` nejde přečíst; platí předchozí konfigurace. |
| `fwd: bezim (agent ano\|ne, ukladani na FS\|jen v pameti)` | Služba firewallu běží. „jen v paměti“ = bez relace FS, rozhodnutí se po restartu ztratí. |
| `fwd: vychozi konfigurace ulozena do /config/firewall.toml` | První start: na FS ještě konfigurace nebyla. |
| `fwd: ... je neplatna (...), pouzivam vychozi` | Soubor na FS nejde přečíst nebo je neplatný (číslo řádku v popisu). Opravit soubor, `fwd` ho při dalším startu převezme. |
| `fwd: sit M (brana G), profil P (neznama sit)` | `netd` ohlásil síť (MAC brány). Neznámá síť dostane `unknown_network` a agent dostane dotaz. |
| `fwd: A povolit\|zakazat vzdy: cil` | Uživatel trvale rozhodl; pravidlo je v konfiguraci. |
| `fwd: dotaz N (A) vyprsel` | Agent neodpověděl do `ask_timeout`; platí `ask_default`. |
| `fwd: agent uzivatele skoncil, dotazy maji vychozi odpoved` | Kanál agenta je zavřený; další dotazy dostanou `ask_default`. |
| `fwd: konfiguraci nejde ulozit: ...` | Zápis na FS selhal; konfigurace platí v paměti. |
| `init: P (aplikace) v jobu: pamet ..., pamet jadra ...` | Program běží v jobu s limity ze svého manifestu. |
| `init: manifest P: ...` | Manifest programu je neplatný (popis chyby); program se nespustí. |
| `init: firewall bez FS (konfigurace se neulozi): ...` | Pro `fwd` nejde připravit relaci FS (chybí FS nebo práva). |
| `init: sluzbu firewallu nelze spustit: ...` | `fwd` nenaběhl; `netd` uvidí zavřený kanál a dotazy odmítá. |

## Textová konzole console

| Hláška | Význam a řešení |
|---|---|
| `console: bezim (ceka na displej a fokus)` | Konzole běží; kreslit začne po `display`, klávesy bere po `focus`. |
| `console: displej prevzat (SxR znaku)` | Převzala framebuffer; mřížka znaků podle rozlišení. |
| `console: displej vracen` | Po `release`; framebuffer je zase u jádra. |
| `console: ridici kanal zavren, koncim` | Vlastník konzole skončil; displej se vrátí jádru. |
| `console: chybi kanal k termd` | Start bez kanálu ke službě relací; konzole končí s kódem 1. |
| `framebuffer prevzal proces X N (log jadra dal na seriove lince)` | Jádro: `FB_CONTROL` `CLAIM`; log už nekreslí na obrazovku. |
| `framebuffer vratil proces X N` | Jádro: `FB_CONTROL` `RELEASE`; obrazovka se smaže a log se kreslí dál. |

## Služba relací termd

| Hláška | Význam a řešení |
|---|---|
| `termd: bezim (protokol vN, FS fsd/zadny, registr ano/ne, prevzato N)` | Prostředník běží. `zadny` = nedostal kanál fsd; shell pak vidí jen prázdný kořen (zápis vrací `READ_ONLY`). `registr ne` = relace nepřežijí konec termd. `prevzato N` = počet relací převzatých po předchozím termd. |
| `termd: prevzata relace N` | Relace z registru přešla na nový termd; okno se k ní může připojit. |
| `termd\| ...` | Zrcadlo výstupu relací jako text (adaptér VT100), aby šlo číst i na sériové lince. Bloky začínají `> příkaz`. |
| `termd: okno zavreno, koncim (N relaci bezi dal)` | Okno se zavřelo; relace se odpojí a běží dál ve svých procesech. Nový termd je převezme z registru. |
| `termd: chyba cekani: ...` | Neočekávaná chyba čekání na kanály; prostředník končí s kódem 1 (relace běží dál). |
| `shsess N: bezim (znacka)` | Proces relace N běží; značka je ve jménech jejích klonů bokem. |
| `shsess N: termd skoncil, cekam na prevzeti` | Hlavní kanál zavřen; příkaz a úlohy doběhnou, relace čeká na `Adopt` od nového termd. |
| `shsess N: prevzato novym termd` | Relace přešla na nový termd. |
| `shsess N: nikdo neprevzal, koncim` | Registr kanál zavřel (nebo žádný není) a žádná úloha nečeká; relace končí. |
| `shsess: chybi Init` / `prvni zprava neni Init` | Proces relace nedostal úvodní zprávu od termd; končí s kódem 1. |
