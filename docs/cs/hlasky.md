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
| `libos: nedostatek pameti pro heap` / `heap nelze namapovat` | Program nemůže začít (heap 4 MiB). |
| `PANIC: ...` | Panika v programu; proces končí s kódem -1 000 001. |
