# Syscally

Rozhraní mezi procesem a jádrem. **Programy mají používat libos**
([libos.md](libos.md)) - čísla a přesné argumenty syscallů nejsou stabilní
rozhraní a mění se spolu s libos. Tento dokument popisuje, co jádro dělá,
aby šlo chování dohledat.

## Stabilita ABI

Do verze 1.0 se rozhraní jádra může měnit (programy se překládají spolu se
systémem). Od 1.0 platí:

- **Programy volají jádro přes systémovou knihovnu** (libos, později libc a
  ntdll Windows vrstvy), ne přímo instrukcí `syscall`. Stabilní je rozhraní
  knihovny; čísla syscallů jsou interní věc knihovny a jádra (stejně jako
  ve Windows, macOS nebo Fuchsii).
- Čísla syscallů se **nikdy nemažou ani znovu nepoužijí**; změna chování =
  nový syscall (např. `..._V2`), starý zůstává.
- Struktury předávané jádru nesou délku nebo verzi, nová pole se přidávají
  jen na konec.
- Nové kódy chyb a signály nesmí změnit význam existujících.
- Startovní zpráva procesu nese verzi; jádro a knihovna podporují i starší.

## Konvence (x86_64)

- Instrukce `syscall`, číslo v `rax`, argumenty v `rdi`, `rsi`, `rdx`,
  `r10`, `r8`, `r9`.
- Výsledek v `rax`: nezáporné číslo = úspěch, záporné = kód chyby
  ([chyby.md](chyby.md)). Ostatní registry kromě `rcx` a `r11` (přepíše je
  instrukce) zůstanou zachované.
- Handle je 32bitová hodnota platná jen v tabulce volajícího procesu.
- Ukazatele míří do paměti volajícího procesu. Jádro celý rozsah ověří
  (namapováno, u výstupu i zapisovatelné), jinak vrátí `BAD_ADDRESS`
  a nic neprovede.
- Čas a termíny jsou v nanosekundách od startu (`CLOCK_MONOTONIC`).
  `DEADLINE_INFINITE` = `u64::MAX` = čekat bez limitu.
- Čísla a limity: `libs/abi/src/syscall.rs`. Práva: `libs/abi/src/object.rs`.

Neúspěšné syscally jádro loguje na úrovni Debug (kromě `SHOULD_WAIT`
a `TIMED_OUT`, které jsou běžné).

## Logování a čas

### `DEBUG_LOG`

`(ptr, len) -> 0`. Zapíše text (UTF-8, max. 1024 B, neplatné UTF-8 se
nahradí) do logu jádra s prefixem `[jméno id]` procesu. Do vzniku služby
logu je to jediný výstup programů. Chyby: `BAD_ADDRESS`, `INVALID_ARGS`
(příliš dlouhý text).

### `CLOCK_MONOTONIC`

`() -> ns`. Monotónní čas v nanosekundách od startu jádra (TSC
kalibrovaný proti HPET). Nemůže selhat.

### `CLOCK_REALTIME`

`(out_ptr) -> 0`. Reálný čas („hodiny na zdi“) pro časy souborů
a zobrazení. Na `out_ptr` zapíše 16 B (little endian):

| Offset | Typ | Význam |
|---|---|---|
| 0 | i64 | sekundy od 1970-01-01 00:00:00 UTC (záporné = dřív) |
| 8 | u32 | nanosekundy uvnitř sekundy (0..10⁹) |
| 12 | u16 | zdroj: 1 UEFI, 2 CMOS RTC, 3 nastaveno za běhu |
| 14 | u16 | příznaky: bit 0 = pásmo neuvedeno, čas brán jako UTC |

Jádro si při startu uloží reálný čas v okamžiku 0 monotónních hodin; výsledek
je ten počátek + `CLOCK_MONOTONIC`. Přesnost počátku je 1 s (rozlišení
firmwaru i RTC). Na rozdíl od `CLOCK_MONOTONIC` **není monotónní**: skočí
při nastavení hodin (`CLOCK_SET`). Pro měření a termíny
vždy `CLOCK_MONOTONIC`. Obal: `libos::time::wall()`, `wall_reading()`.
Chyby: `NOT_SUPPORTED` (jádro reálný čas nezná), `BAD_ADDRESS`.
Podrobnosti: [Reálný čas](cas.md).

### `CLOCK_SET`

`(resource, in_ptr, flags) -> 0 | 1`. Nastaví reálný čas. Vyžaduje handle
na Resource s právem `SET_CLOCK`. Má ho jen kořenový Resource a `init` ho
předá jen službě času `timed` (bez `MANAGE`). Na `in_ptr` je 16 B (little
endian, `abi::time::SetRequest`):

| Offset | Typ | Význam |
|---|---|---|
| 0 | i64 | nový čas: sekundy od 1970-01-01 00:00:00 UTC |
| 8 | u32 | nanosekundy (0..10⁹, jinak `INVALID_ARGS`) |
| 12 | i32 | `rtc_offset`: sekundy přičtené k UTC při zápisu do hardwarových hodin (RTC v místním čase = posun pásma, jinak 0) |

`flags`: bit 0 `WRITE_HARDWARE` = zapsat čas i do hardwarových hodin (x86:
CMOS RTC). Ostatní bity = `INVALID_ARGS`. Vrací 1, když se hardwarové
hodiny zapsaly. Vrací 0 bez `WRITE_HARDWARE`, když hodiny chybí (aarch64,
ACPI hlásí, že RTC není) nebo když se rok nevejde do jejich rozsahu. Čas
systému se nastaví v každém případě. Poté `CLOCK_REALTIME` hlásí zdroj 3
(nastaveno za běhu) a žádné příznaky. Termíny čekání se nemění, počítají se
z `CLOCK_MONOTONIC`. Obal: `libos::time::set`, `Resource::set_clock`.
Chyby: `ACCESS_DENIED` (chybí `SET_CLOCK`), `WRONG_TYPE`, `BAD_HANDLE`,
`INVALID_ARGS`, `BAD_ADDRESS`. Podrobnosti:
[Nastavení hodin](cas.md#nastavení-hodin-clock_set).

## Handly

### `HANDLE_CLOSE`

`(handle) -> 0`. Zavře handle. Objekt zanikne, když na něj nevede žádný
handle ani interní odkaz (např. zavření posledního konce kanálu nastaví
druhému konci `PEER_CLOSED`). Chyby: `BAD_HANDLE`.

### `HANDLE_DUPLICATE`

`(handle, rights) -> handle`. Nový handle na stejný objekt. `rights` musí
být podmnožinou práv původního handlu; hodnota `u32::MAX` práva zachová.
Vyžaduje právo `DUPLICATE`. Chyby: `BAD_HANDLE`, `ACCESS_DENIED` (chybí
`DUPLICATE` nebo by se práva přidávala), `NO_RESOURCES`.

## Čekání a eventy

### `OBJECT_WAIT_ONE`

`(handle, mask, deadline, out_signals_ptr) -> 0`. Čeká, dokud signály
objektu neprotnou `mask`, nebo do termínu. Do `out_signals_ptr` (pokud není
0) zapíše aktuální signály objektu - i při `TIMED_OUT`. Vyžaduje `WAIT`.
Chyby: `BAD_HANDLE`, `ACCESS_DENIED`, `TIMED_OUT`, `CANCELED`,
`BAD_ADDRESS`.

### `OBJECT_WAIT_MANY`

`(items_ptr, count, deadline) -> index`. Čeká na více objektů najednou
(max. 64). `items` je pole `WaitItem { handle: u32, mask: u32, observed: u32 }`;
jádro vrátí index prvního splněného objektu a do `observed` všech položek
zapíše jejich aktuální signály. `count` = 0 je uspání do termínu (končí
`TIMED_OUT`). Vyžaduje `WAIT` u všech handlů. Chyby: jako `OBJECT_WAIT_ONE`
a `INVALID_ARGS` (count > 64).

### `OBJECT_SIGNAL`

`(handle, clear, set) -> 0`. Změní signály eventu: nejdřív smaže bity
`clear`, pak nastaví `set`. Proces smí měnit jen `SIGNALED`; ostatní signály
řídí jádro. Vyžaduje `SIGNAL`. Chyby: `BAD_HANDLE`, `ACCESS_DENIED`,
`WRONG_TYPE` (není event), `INVALID_ARGS` (jiný bit než `SIGNALED`).

### `EVENT_CREATE`

`() -> handle`. Nový event (nesignalizovaný) s právy `DUPLICATE`,
`TRANSFER`, `WAIT`, `SIGNAL`. Chyby: `NO_RESOURCES`.

## Porty

Port je fronta událostí pro čekání na **libovolně mnoho objektů** (služby
se stovkami kanálů, např. `netd`). `OBJECT_WAIT_MANY` je omezený na 64
objektů a při každém čekání registruje vlákno u všech. Port místo toho
drží **jednorázové registrace**: objekt, jehož signály protnou masku,
vloží do portu paket (klíč + signály) a registrace zanikne. Program
paket vyzvedne a po obsluze objekt zaregistruje znovu. Cena události
nezávisí na počtu hlídaných objektů.

- **Místo pro paket se rezervuje při registraci**, takže doručení
  (i z obsluhy přerušení) nealokuje a paket se neztratí. Registrací
  a čekajících paketů může být na portu nejvýš `PORT_MAX_PENDING`
  (65 536) a ve všech portech jednoho procesu dohromady nejvýš
  `PROCESS_MAX_PORT_PENDING` (65 536), protože každá registrace drží
  paměť jádra. Místo v kvótě se vrátí vyzvednutím paketu, zánikem
  objektu nebo zavřením portu. Prázdný port vrátí přebytečnou kapacitu
  fronty.
- **Zaniklý objekt** (zavřený poslední handle) vrátí rezervaci svých
  nedoručených registrací. **Zavřený port**: objekty jeho registrace
  zahodí.
- Port má signál `READABLE`, dokud v něm je paket. Lze na něj tedy čekat
  i přes `OBJECT_WAIT_ONE`/`OBJECT_WAIT_MANY`.
- Pakety přicházejí v pořadí, v jakém nastaly.
- Obaly: `libos::Port` (`create`, `wait_async`, `wait`).

```rust
let port = libos::Port::create()?;
port.wait_async(ch.handle(), 1, Signals::READABLE)?;     // klíč 1
port.wait_async(ch.handle(), 2, Signals::PEER_CLOSED)?;  // klíč 2
loop {
    let p = port.wait(None)?;
    match p.key {
        1 => { /* přečíst zprávu */ port.wait_async(ch.handle(), 1, Signals::READABLE)?; }
        2 => break, // druhá strana zavřela
        _ => {}
    }
}
```

### `PORT_CREATE`

`() -> handle`. Nový prázdný port s právy `DUPLICATE`, `TRANSFER`, `WAIT`,
`READ`, `WRITE`. Chyby: `NO_RESOURCES`.

### `OBJECT_WAIT_ASYNC`

`(handle, port, key, mask) -> 0`. Jednorázová registrace: až signály
objektu `handle` protnou `mask`, port dostane paket s `key` (u64, význam
určuje program). Když je podmínka splněná už teď, paket přijde hned.
Vyžaduje `WAIT` na objektu a `WRITE` na portu. Na port samotný registrovat
nejde. Chyby: `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE` (`port` není
port), `NOT_SUPPORTED` (objekt je port), `INVALID_ARGS` (prázdná maska),
`NO_RESOURCES` (port má `PORT_MAX_PENDING` registrací a paketů, nebo
proces vyčerpal `PROCESS_MAX_PORT_PENDING`), `NO_MEMORY`.

### `PORT_WAIT`

`(port, deadline, out_ptr) -> 0`. Vyjme nejstarší paket a zapíše ho na
`out_ptr` (16 B, `abi::object::PortPacket`): klíč u64, signály objektu
v okamžiku splnění u32, rezerva u32. Prázdný port čeká do termínu
(`deadline` = 0: jen zkusit). Vyžaduje `READ`. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `TIMED_OUT`, `CANCELED`, `BAD_ADDRESS`
(paket je pak ztracený).

## Kanály

Zpráva = data (max. 64 KiB) + handly (max. 64). Každý konec má vlastní
frontu příchozích zpráv (max. 1024). Signály konce: `READABLE`,
`PEER_CLOSED`.

### `CHANNEL_CREATE`

`(out_handles_ptr) -> 0`. Vytvoří propojenou dvojici konců a zapíše dva
handly (u32) do `out_handles_ptr`. Práva: `DUPLICATE`, `TRANSFER`, `WAIT`,
`READ`, `WRITE`. Chyby: `NO_RESOURCES`, `BAD_ADDRESS`.

### `CHANNEL_WRITE`

`(handle, data_ptr, data_len, handles_ptr, handles_count) -> 0`. Pošle
zprávu druhému konci. Předávané handly musí mít právo `TRANSFER`, nesmí se
opakovat a nesmí jít o zapisovaný konec ani o jeho protějšek (zpráva by
skončila ve frontě protějšku, který by tak držel sám sebe). Konec kanálu,
v jehož frontě už leží jiný konec kanálu, poslat nejde (`BAD_STATE`): jen
tak nikdy nevznikne cyklus konců, které drží jeden druhého ve frontách a
nikdy se neuvolní. Předávané handly se z tabulky
odesílatele **odeberou vždy, i když zápis selže** - handle tak nikdy nemá
odesílatel i příjemce zároveň. Vyžaduje `WRITE`. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `PEER_CLOSED`,
`SHOULD_WAIT` (plná fronta), `BAD_ADDRESS`.

### `CHANNEL_READ`

`(handle, data_ptr, data_cap, handles_ptr, handles_cap, out_actual_ptr) -> 0`.
Vyjme další zprávu: data zkopíruje do `data_ptr`, handly vloží do tabulky
procesu a jejich hodnoty zapíše do `handles_ptr`, do `out_actual_ptr` zapíše
dva u32 (délka dat, počet handlů). Neblokuje. Vyžaduje `READ`. Chyby:
`SHOULD_WAIT` (prázdná fronta), `PEER_CLOSED` (prázdná fronta a druhý konec
zavřený), `BUFFER_TOO_SMALL` (zpráva zůstane ve frontě, do `out_actual_ptr`
se zapíšou potřebné velikosti), `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`,
`BAD_ADDRESS`.

## Paměť

### `VMO_CREATE`

`(size) -> handle`. Objekt paměti o velikosti `size` (zaokrouhleno na 4 KiB,
max. 4 GiB), vyplněný nulami. Stránky se zatím alokují hned. Práva:
`DUPLICATE`, `TRANSFER`, `WAIT`, `READ`, `WRITE`, `MAP`, `EXECUTE`. Chyby:
`INVALID_ARGS` (0 nebo příliš velký), `NO_MEMORY`, `NO_RESOURCES`.

### `VMO_READ`

`(handle, offset, buf_ptr, len) -> 0`. Přečte data objektu (max. 1 MiB na
volání). Vyžaduje `READ`. Chyby: `OUT_OF_RANGE`, `BAD_ADDRESS`,
`BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`.

### `VMO_WRITE`

`(handle, offset, buf_ptr, len) -> 0`. Zapíše data do objektu (max. 1 MiB).
Vyžaduje `WRITE`. Chyby: jako `VMO_READ`.

### `VM_MAP`

`(process, vmo, vmo_offset, len, flags, addr) -> addr`. Namapuje `len`
bajtů objektu od `vmo_offset` do adresního prostoru procesu `process`
(může být vlastní i jiný, např. při spouštění programu). `flags`:
`MAP_READ` (povinné), `MAP_WRITE`, `MAP_EXECUTE` (nesmí být s `MAP_WRITE` -
W^X), `MAP_FIXED` (přesně na `addr`; jinak je `addr` nápověda a 0 = vybere
jádro). Adresa, délka a offset musí být zarovnané na velikost stránky
procesu (`VM_PAGE_SIZE`, dnes 4 KiB), rozsah v
`0x10000..0x7FFF_FFFF_F000` (`USER_MIN`..`USER_MAX`; poslední stránka
spodní poloviny zůstává vždy nenamapovaná). Práva: proces `WRITE`; objekt `MAP` + `READ`,
případně `WRITE`/`EXECUTE` podle příznaků. Mapování drží objekt naživu i po
zavření handlu. Chyby: `INVALID_ARGS`, `OUT_OF_RANGE`, `BAD_STATE`
(`MAP_FIXED` přes existující mapování), `NO_RESOURCES` (není volné místo),
`NO_MEMORY`, `BAD_HANDLE`, `ACCESS_DENIED`, `WRONG_TYPE`.

### `VM_UNMAP`

`(process, addr, len) -> 0`. Zruší mapování. Rozsah musí přesně pokrývat
celá mapování (částečné odmapování zatím není). Po návratu žádné CPU nemá
starý překlad v TLB. Vyžaduje `WRITE` na procesu. Chyby: `NOT_FOUND`,
`INVALID_ARGS`, `BAD_HANDLE`, `ACCESS_DENIED`.

### `VM_PAGE_SIZE`

`(process) -> bajty`. Velikost stránky adresního prostoru procesu: mocnina
dvou od `MIN_PAGE_SIZE` (4 KiB) do `MAX_PAGE_SIZE` (64 KiB) z `abi::vm`.
Na tuto hodnotu musí být zarovnané adresy, délky a offsety pro `VM_MAP`
a `VM_UNMAP`. Není to konstanta ABI: dnes je všude 4 KiB, ale na aarch64
může jádro dát nativním procesům 16 KiB a procesům Windows vrstvy 4 KiB
(každý adresní prostor má vlastní granuli). Rodič se ptá na potomka při
nahrávání programu (`libos::vm::page_size_of`), program na sebe
(`libos::vm::page_size`, zjistí jednou a uloží). Stačí libovolný handle
procesu (žádná práva). Chyby: `BAD_HANDLE`, `WRONG_TYPE`.

## Procesy a vlákna

Spuštění programu (dělá ho libos `spawn`): `PROCESS_CREATE` ->
`VMO_CREATE`/`VMO_WRITE`/`VM_MAP` segmentů a zásobníku -> `CHANNEL_CREATE`
a zápis startovní zprávy -> `THREAD_CREATE` -> `PROCESS_START`.

### `PROCESS_CREATE`

`(name_ptr, name_len) -> handle`. Prázdný proces bez paměti a vláken. Jméno:
UTF-8, max. 64 B (pro logy). Práva: `DUPLICATE`, `TRANSFER`, `WAIT`,
`MANAGE`, `WRITE`, `READ`. Chyby: `INVALID_ARGS`, `NO_MEMORY`, `BAD_ADDRESS`.

### `PROCESS_START`

`(process, thread, entry, stack, bootstrap_handle) -> 0`. Spustí první
vlákno procesu na adrese `entry` se zásobníkem `stack`. Handle
`bootstrap_handle` (typicky konec kanálu se startovní zprávou) se přesune do
tabulky nového procesu a vlákno dostane jeho hodnotu v `rdi`. Vlákno musí
patřit procesu a proces nesmí být spuštěný. Vyžaduje `MANAGE` na procesu
i vláknu, `TRANSFER` na bootstrap handlu. `entry` musí ležet v paměti
procesu (`USER_MIN` až `USER_MAX`), `stack` nejvýš na `USER_MAX`, jinak
`INVALID_ARGS`. Chyby: `BAD_STATE`, `INVALID_ARGS`, `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`.

### `PROCESS_EXIT`

`(code)` - nevrací se. Ukončí aktuální proces s kódem `code`: všechna jeho
vlákna skončí (čekající s `CANCELED`), handly se zavřou, proces dostane
signál `TERMINATED`.

### `PROCESS_KILL`

`(process) -> 0`. Ukončí jiný proces s kódem -1 000 000. Vlákna, která
právě běží na jiném CPU, jádro přeruší. Vyžaduje `MANAGE`.

### `PROCESS_EXIT_CODE`

`(process, out_code_ptr) -> 0`. Zapíše návratový kód (i64) skončeného
procesu. Vyžaduje `READ`. Chyby: `BAD_STATE` (proces ještě běží - nejdřív
počkat na `TERMINATED`).

### `THREAD_CREATE`

`(process, name_ptr, name_len) -> handle`. Nové (nespuštěné) vlákno procesu.
Vyžaduje `MANAGE` na procesu. Chyby: `BAD_STATE` (proces končí),
`NO_MEMORY`, `INVALID_ARGS`.

### `THREAD_START`

`(thread, entry, stack, arg0, arg1) -> 0`. Spustí další vlákno již běžícího
procesu; `arg0`, `arg1` dostane v `rdi`, `rsi`. Zásobník musí být zarovnaný
jako po instrukci `call` (adresa = 8 mod 16), pokud vstupní funkce dodržuje
System V ABI. Pro `entry` a `stack` platí totéž co u `PROCESS_START`.
Vyžaduje `MANAGE`. Chyby: `BAD_STATE`, `INVALID_ARGS`.

### `THREAD_EXIT`

`()` - nevrací se. Ukončí aktuální vlákno. Konec posledního vlákna ukončí
proces (kód -1 000 002, pokud předtím nevolal `PROCESS_EXIT`).

### `THREAD_YIELD`

`() -> 0`. Přenechá CPU dalšímu připravenému vláknu stejné nebo vyšší třídy.

## Limity prostředků

Program nesmí spotřebovat paměť jádra bez omezení (`abi::limits`):

| Limit | Hodnota | Po překročení |
|---|---|---|
| rezerva jádra: paměť objektů paměti (`VMO_CREATE*`), zásobníky vláken, data zpráv ve frontách, tabulky stránek mapování, nové procesy a objekty (každý nový handle) se nealokují, kdyby volné paměti zbylo méně | 1/32 RAM, nejméně 16 MiB | `NO_MEMORY` |
| handly v tabulce procesu | 65 536 | `NO_RESOURCES` |
| vlákna procesu (včetně ukončených, na která někdo drží handle) | 1024 | `NO_RESOURCES` |
| mapování v adresním prostoru procesu | 65 536 | `NO_RESOURCES` |
| zprávy ve frontě jednoho konce kanálu | 1024 | `SHOULD_WAIT` |
| bajty dat ve frontě jednoho konce kanálu | 4 MiB | `SHOULD_WAIT` |
| registrace a nevyzvednuté pakety ve všech portech procesu | 65 536 | `NO_RESOURCES` |

Vyčerpání paměti programem tak jádro neshodí: tabulky stránek, heap a
zásobníky jádra smí čerpat i z rezervy. Plán: kvóty pro skupiny procesů
(job) nastavitelné rodičem.

## Hardware (ovladače)

Vyžadují handle na **Resource** s právem `MANAGE` (oprávnění k hardwaru).
Jádro vytvoří jediný kořenový Resource bez omezení a dá ho procesu `init`
(startovní zpráva, druh `ROOT_RESOURCE`); init ho předá správci zařízení.
Ten každému ovladači vytvoří **omezený** Resource (`RESOURCE_CREATE`): jen
pamětové BARy jeho zařízení a stránku jeho konfiguračního prostoru PCI,
nejvýš 8 přerušení a 4 MiB paměti pro DMA. Chyba v ovladači tak nemůže
přepsat registry jiného zařízení. S IOMMU navíc zařízení smí přes DMA jen
do paměti vytvořené přes svůj prostředek (viz [jadro.md](jadro.md#iommu-ochrana-dma));
bez IOMMU může zapsat kamkoli.

Kořenový Resource má navíc právo `SET_CLOCK` (nastavení hodin,
[`CLOCK_SET`](#clock_set)). Správce zařízení dostane kořen bez něj
a omezené prostředky ho nemají nikdy.

### `VMO_CREATE_CONTIGUOUS`

`(resource, size) -> handle`. Fyzicky souvislý objekt paměti pro DMA
(max. 4 MiB), vyplněný nulami. Do procesu se mapuje jako paměť pro DMA:
na koherentní platformě (x86_64, QEMU virt) normálně s cache, na
nekoherentní (např. Raspberry Pi 4) bez cache, aby zařízení i ovladač
viděli stejná data bez ruční údržby cache. Pořadí zápisů vůči zařízení
musí ovladač zajistit bariérami (`libos::barrier`). Chyby: `INVALID_ARGS`
(0 nebo nad 4 MiB), `NO_MEMORY` (není souvislý blok), `ACCESS_DENIED`,
`WRONG_TYPE`.

### `VMO_CREATE_PHYSICAL`

`(resource, phys, size) -> handle`. Okno na registry zařízení (MMIO).
Rozsah nesmí ležet v RAM ani paměti firmwaru (jinak `ACCESS_DENIED`).
Mapuje se bez cache; přes `VMO_READ`/`VMO_WRITE` ho číst nejde
(`NOT_SUPPORTED`, čtení registrů má vedlejší efekty) a jako buffer
syscallu ho použít nejde (`BAD_ADDRESS`). Omezený prostředek
(`RESOURCE_CREATE`) smí jen své rozsahy. Chyby: `INVALID_ARGS`
(nezarovnaná adresa), `ACCESS_DENIED`.

### `VMO_PHYS_ADDR`

`(resource, vmo, offset) -> phys`. Fyzická adresa bajtu `offset` objektu
paměti (pro programování DMA). U souvislého objektu platí pro celý rozsah,
u běžného jen v rámci jedné stránky. Chyby: `OUT_OF_RANGE`, `ACCESS_DENIED`.

### `INTERRUPT_CREATE`

`(resource, out_msi_ptr) -> handle`. Objekt přerušení s volným vektorem
CPU. Do `out_msi_ptr` zapíše zprávu MSI `{ address: u64, data: u32,
reserved: u32 }`, kterou ovladač zapíše do zařízení (tabulka MSI-X).
Když zařízení přerušení vyvolá, objekt dostane signál `SIGNALED`. Na
aarch64 (GICv3 ITS) musí být prostředek omezený na zařízení
(`RESOURCE_CREATE` s `device`), jinak `INVALID_ARGS`; na x86_64 to není
potřeba. Chyby: `NO_RESOURCES` (došly vektory), `INVALID_ARGS`,
`ACCESS_DENIED`, `BAD_ADDRESS`.

### `RESOURCE_CREATE`

`(resource, ranges_ptr, ranges_count, irq_max, dma_max, device) -> handle`. Užší
prostředek pro ovladač. `ranges_ptr` ukazuje na `ranges_count` (max. 16)
dvojic u64 (fyzický začátek, délka; obojí zarovnané na stránky, jinak
`INVALID_ARGS`). Nový prostředek smí vytvářet okna
(`VMO_CREATE_PHYSICAL`) jen uvnitř těchto rozsahů, nejvýš `irq_max` objektů
přerušení a nejvýš `dma_max` bajtů paměti pro DMA (`VMO_CREATE_CONTIGUOUS`,
počítá se skutečně alokovaný blok 2^n stránek; limity platí po celou dobu
života prostředku, po vyčerpání `NO_RESOURCES`, neúspěšný pokus nic
nespotřebuje). Kvóty se odečítají v celém řetězci rodičů: potomci dohromady
nikdy nepřekročí limit rodiče. Je-li rodič sám omezený, musí rozsahy ležet v
jeho rozsazích a limity nesmí být vyšší (jinak `ACCESS_DENIED`). Vyžaduje
`MANAGE`. Chyby: `INVALID_ARGS` (víc než 16 rozsahů, nulová délka,
přetečení), `ACCESS_DENIED`, `BAD_ADDRESS`, `BAD_HANDLE`, `WRONG_TYPE`.

`device` = zařízení, kterému prostředek patří: 0 = žádné, jinak
`1 << 32 | segment << 16 | sběrnice << 8 | zařízení << 3 | funkce`
(`abi::syscall::pci_device`). S IOMMU dostane zařízení doménu a paměť z
`VMO_CREATE_CONTIGUOUS` přes tento prostředek (a jeho potomky) se do ní
namapuje. Potomek zdědí zařízení rodiče a jiné zvolit nesmí
(`ACCESS_DENIED`). Bez IOMMU se `device` jen zaznamená.

### `INTERRUPT_ACK`

`(interrupt) -> 0`. Potvrdí přerušení (smaže `SIGNALED`). Ovladač má
potvrdit **před** obsluhou zařízení, aby se přerušení, které přijde během
obsluhy, neztratilo. Vyžaduje `WRITE`.

## Náhodná čísla

Jádro má kryptografický generátor (`kernel/src/random.rs`, crate
`libs/csprng`):

- **Zásobník entropie** míchá vstupy hashem BLAKE2s. Každý vstup má kredit
  = odhad skutečné náhody: RDSEED/RDRAND 32 bitů z 64, data od ovladače
  virtio-rng podle ovladače (4 bity na bajt), jitter čítače, čas firmwaru
  a časování přerušení 0.
- **Generátor je ChaCha20.** Reseed: klíč = BLAKE2s(starý klíč, zásobník),
  nejpozději po 10 s nebo když čeká aspoň 256 bitů kreditu. Po každém
  požadavku se klíč přepíše prvním blokem proudu (fast key erasure), takže
  pozdější únik stavu jádra neprozradí dřívější výstupy.
- **Nasazený** je generátor po reseedu se součtem kreditu aspoň 256 bitů.
  Před nasazením se kredit sčítá přes všechny reseedy (entropie zůstává
  v klíči), takže stačí i zdroj, který dodává po malých dávkách.
  V QEMU stačí RDRAND (`-cpu max`), na stroji bez něj ovladač virtio-rng.

⚠️ Bez hardwarového zdroje a bez virtio-rng generátor nasazený nebude
(výstup jen z času a jitteru) a jádro to zaloguje. Klíče (TLS) proto berte
přes `REQUIRE_SEEDED`.

### `RANDOM_GET`

`(buf_ptr, len, flags) -> 0 | 1`. Naplní buffer (max. `RANDOM_MAX` =
4096 B) náhodnými bajty. Vrací 1, když je generátor nasazený, 0, když
ještě ne (data jsou pak slabá). `flags`: bit 0 `REQUIRE_SEEDED` =
nenasazený generátor vrátí `SHOULD_WAIT` a nic nezapíše. Nepotřebuje
handle. Obaly: `libos::random::fill`, `fill_seeded`, `u64`. Chyby:
`INVALID_ARGS` (délka, neznámý příznak), `SHOULD_WAIT`, `BAD_ADDRESS`.

### `RANDOM_ADD`

`(resource, ptr, len, credit_bits) -> 0`. Přimíchá data (max. 4096 B) do
zásobníku entropie s kreditem `credit_bits` (jádro ho omezí na 8 bitů na
bajt). Vyžaduje Resource s právem `MANAGE` (ovladač zařízení). Přimíchání
nikdy nesníží kvalitu generátoru, ale **kredit věří volajícímu**: vadný
ovladač může generátor prohlásit za nasazený dřív, než skutečně je. Obal:
`libos::random::add`. Používá ho ovladač `virtio-rng`. Chyby: `BAD_HANDLE`,
`ACCESS_DENIED`, `WRONG_TYPE`, `INVALID_ARGS`, `BAD_ADDRESS`.
